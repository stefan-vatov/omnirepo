use std::{
    fs,
    io::{Read, Write},
    net::{Shutdown, TcpStream},
    thread,
};

use omnirepo_test_support::{
    lifecycle_fixture::{FixtureError, FixtureOutcome, FixtureSpec, LifecycleFixture},
    network_double::{HttpDoubleSpec, LocalHttpDouble, NetworkDoubleError},
};

fn start_request(
    endpoint: &str,
    request: Vec<u8>,
) -> thread::JoinHandle<Result<Vec<u8>, NetworkDoubleError>> {
    let endpoint = endpoint.to_owned();
    thread::spawn(move || LocalHttpDouble::send_request(&endpoint, &request))
}

fn cleanup(fixture: LifecycleFixture) {
    let report = fixture.cleanup(FixtureOutcome::Success);
    assert!(report.removed, "successful fixtures must be removed");
    assert!(
        report.leaks.is_empty(),
        "fixture leaked: {:?}",
        report.leaks
    );
}

#[test]
fn protocol_barriers_are_single_use_and_ordered() {
    let mut fixture = LifecycleFixture::create(FixtureSpec::new("network-ordering", 7_901))
        .expect("fixture should be created");
    let mut server =
        LocalHttpDouble::bind(&mut fixture, "ordering", HttpDoubleSpec::new(200, b"ok\n"))
            .expect("server should bind");

    let client = start_request(
        server.endpoint(),
        b"GET /ordering HTTP/1.1\r\nHost: localhost\r\n\r\n".to_vec(),
    );
    let request = server
        .wait_for_request()
        .expect("request barrier should be released");
    assert_eq!(request.method, "GET");
    assert_eq!(request.path, "/ordering");
    let duplicate_wait = server
        .wait_for_request()
        .expect_err("request barrier must be single-use");
    assert_eq!(
        duplicate_wait.to_string(),
        "network double protocol error: request barrier was already consumed"
    );

    server.respond().expect("response should be released");
    let duplicate_response = server
        .respond()
        .expect_err("response barrier must be single-use");
    assert_eq!(
        duplicate_response.to_string(),
        "network double protocol error: response was already released"
    );
    let raw = client
        .join()
        .expect("client should join")
        .expect("client request should complete");
    assert!(String::from_utf8_lossy(&raw).starts_with("HTTP/1.1 200 OK"));
    let exchange = server.finish().expect("server should join");
    assert_eq!(exchange.response.status, 200);
    cleanup(fixture);
}

#[test]
fn respond_before_request_is_rejected_and_drop_reaps_the_server() {
    let mut fixture = LifecycleFixture::create(FixtureSpec::new("network-respond-first", 7_902))
        .expect("fixture should be created");
    let server = LocalHttpDouble::bind(
        &mut fixture,
        "respond-first",
        HttpDoubleSpec::new(200, b"ok\n"),
    )
    .expect("server should bind");
    let mut server = server;
    let error = server
        .respond()
        .expect_err("response cannot precede request barrier");
    assert_eq!(
        error.to_string(),
        "network double protocol error: respond called before request barrier"
    );
    drop(server);
    cleanup(fixture);
}

#[test]
fn finish_before_response_returns_a_typed_error_but_drop_releases_client() {
    let mut fixture = LifecycleFixture::create(FixtureSpec::new("network-finish-first", 7_903))
        .expect("fixture should be created");
    let mut server = LocalHttpDouble::bind(
        &mut fixture,
        "finish-first",
        HttpDoubleSpec::new(200, b"ok\n"),
    )
    .expect("server should bind");
    let client = start_request(
        server.endpoint(),
        b"GET /finish-first HTTP/1.1\r\nHost: localhost\r\n\r\n".to_vec(),
    );
    server
        .wait_for_request()
        .expect("request barrier should be released");
    let error = server
        .finish()
        .expect_err("finish must reject an unreleased response");
    assert_eq!(
        error.to_string(),
        "network double protocol error: finish called before response release"
    );
    let raw = client
        .join()
        .expect("client should join after drop releases it")
        .expect("drop response should be readable");
    assert!(String::from_utf8_lossy(&raw).starts_with("HTTP/1.1 500 Internal Server Error"));
    cleanup(fixture);
}

#[test]
fn invalid_endpoints_fail_closed_without_connecting_to_external_services() {
    let protocol = LocalHttpDouble::send_request(
        "https://127.0.0.1:1",
        b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n",
    )
    .expect_err("non-http endpoint must be rejected before connect");
    assert_eq!(
        protocol.to_string(),
        "network double protocol error: endpoint is not local HTTP"
    );

    let invalid_address =
        LocalHttpDouble::send_request("http://", b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .expect_err("empty loopback address must fail as I/O");
    assert!(matches!(invalid_address, NetworkDoubleError::Io(_)));
}

#[test]
fn malformed_headers_are_reported_by_the_server_thread() {
    let mut fixture = LifecycleFixture::create(FixtureSpec::new("network-malformed", 7_904))
        .expect("fixture should be created");
    let mut server =
        LocalHttpDouble::bind(&mut fixture, "malformed", HttpDoubleSpec::new(200, b"ok\n"))
            .expect("server should bind");
    let client = start_request(server.endpoint(), b"\r\n\r\n".to_vec());
    let barrier_error = server
        .wait_for_request()
        .expect_err("malformed request cannot cross request barrier");
    assert_eq!(
        barrier_error.to_string(),
        "network double protocol error: request thread ended early"
    );
    assert!(client.join().expect("client should join").is_ok());
    let finish_error = server
        .finish()
        .expect_err("server thread must preserve malformed request error");
    assert_eq!(
        finish_error.to_string(),
        "network double protocol error: request has no start line"
    );
    cleanup(fixture);
}

#[test]
fn authorization_requires_exactly_one_expected_header() {
    let cases = [
        (
            "network-auth-missing",
            7_905,
            b"GET /missing HTTP/1.1\r\nHost: localhost\r\n\r\n".as_slice(),
            false,
        ),
        (
            "network-auth-wrong",
            7_906,
            b"GET /wrong HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer unexpected\r\n\r\n".as_slice(),
            false,
        ),
        (
            "network-auth-duplicate",
            7_907,
            b"GET /duplicate HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer fixture-token\r\nAuthorization: Bearer fixture-token\r\n\r\n".as_slice(),
            false,
        ),
        (
            "network-auth-valid",
            7_908,
            b"GET /valid HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer fixture-token\r\n\r\n".as_slice(),
            true,
        ),
    ];

    for (case_id, seed, request_bytes, expected_valid) in cases {
        let mut fixture = LifecycleFixture::create(FixtureSpec::new(case_id, seed))
            .expect("fixture should be created");
        let mut server = LocalHttpDouble::bind(
            &mut fixture,
            "auth",
            HttpDoubleSpec::new(200, b"ok\n").requiring_bearer("fixture-token"),
        )
        .expect("server should bind");
        let client = start_request(server.endpoint(), request_bytes.to_vec());
        let request = server
            .wait_for_request()
            .expect("request should reach the barrier");
        assert_eq!(
            request.authorization_valid, expected_valid,
            "case={case_id}"
        );
        assert_eq!(
            request.authorization_present,
            case_id != "network-auth-missing"
        );
        server.respond().expect("response should be released");
        let raw = client
            .join()
            .expect("client should join")
            .expect("client request should complete");
        let expected_status = if expected_valid { 200 } else { 401 };
        assert!(String::from_utf8_lossy(&raw).starts_with(&format!("HTTP/1.1 {expected_status} ")));
        assert_eq!(
            server.finish().expect("server should join").response.status,
            expected_status
        );
        cleanup(fixture);
    }
}

#[test]
fn unsolicited_authorization_is_rejected() {
    let mut fixture = LifecycleFixture::create(FixtureSpec::new("network-auth-unsolicited", 7_909))
        .expect("fixture should be created");
    let mut server = LocalHttpDouble::bind(&mut fixture, "auth", HttpDoubleSpec::new(200, b"ok\n"))
        .expect("server should bind");
    let client = start_request(
        server.endpoint(),
        b"GET /unsolicited HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer unexpected\r\n\r\n"
            .to_vec(),
    );
    let request = server
        .wait_for_request()
        .expect("request should reach the barrier");
    assert!(request.authorization_present);
    assert!(!request.authorization_valid);
    server.respond().expect("response should be released");
    let raw = client
        .join()
        .expect("client should join")
        .expect("client request should complete");
    assert!(String::from_utf8_lossy(&raw).starts_with("HTTP/1.1 401 Unauthorized"));
    assert_eq!(
        server.finish().expect("server should join").response.status,
        401
    );
    cleanup(fixture);
}

#[test]
fn status_reason_phrases_cover_known_and_default_statuses() {
    let statuses = [
        (200, "OK"),
        (201, "Created"),
        (204, "No Content"),
        (401, "Unauthorized"),
        (404, "Not Found"),
        (409, "Conflict"),
        (500, "Internal Server Error"),
        (418, "Fixture Status"),
    ];

    for (index, (status, reason)) in statuses.into_iter().enumerate() {
        let case_id = format!("network-reason-{status}");
        let mut fixture = LifecycleFixture::create(FixtureSpec::new(case_id, 7_910 + index as u64))
            .expect("fixture should be created");
        let mut server = LocalHttpDouble::bind(
            &mut fixture,
            "reason",
            HttpDoubleSpec::new(status, b"body\n"),
        )
        .expect("server should bind");
        let client = start_request(
            server.endpoint(),
            b"GET /reason HTTP/1.1\r\nHost: localhost\r\n\r\n".to_vec(),
        );
        server
            .wait_for_request()
            .expect("request should reach the barrier");
        server.respond().expect("response should be released");
        let raw = client
            .join()
            .expect("client should join")
            .expect("client request should complete");
        assert!(
            String::from_utf8_lossy(&raw).starts_with(&format!("HTTP/1.1 {status} {reason}")),
            "unexpected response for status {status}: {:?}",
            String::from_utf8_lossy(&raw)
        );
        assert_eq!(
            server.finish().expect("server should join").response.status,
            status
        );
        cleanup(fixture);
    }
}

#[test]
fn fragmented_oversized_requests_are_bounded_and_recorded() {
    let mut fixture = LifecycleFixture::create(FixtureSpec::new("network-truncated", 7_920))
        .expect("fixture should be created");
    let request_prefix = b"POST /bounded HTTP/1.1\r\nHost: localhost\r\n\r\n".to_vec();
    let limit = request_prefix.len() + 32;
    let mut server = LocalHttpDouble::bind(
        &mut fixture,
        "bounded",
        HttpDoubleSpec {
            status: 200,
            body: b"ok\n".to_vec(),
            required_bearer: None,
            request_limit: limit,
        },
    )
    .expect("server should bind");
    let endpoint = server.endpoint().to_owned();
    let body = vec![b'x'; 32 * 1024];
    let client = thread::spawn(move || {
        let address = endpoint
            .strip_prefix("http://")
            .expect("fixture endpoint must be local HTTP");
        let mut stream = TcpStream::connect(address).expect("loopback client should connect");
        stream
            .write_all(&request_prefix)
            .expect("request headers should be written");
        stream
            .write_all(&body)
            .expect("large body should be written");
        stream
            .shutdown(Shutdown::Write)
            .expect("client write side should close");
        let mut response = Vec::new();
        stream
            .read_to_end(&mut response)
            .expect("response should be readable");
        response
    });
    let request = server
        .wait_for_request()
        .expect("bounded request should reach barrier");
    assert!(request.truncated);
    assert_eq!(request.body.len(), 32);
    assert_eq!(request.body, vec![b'x'; 32]);
    server.respond().expect("response should be released");
    let raw = client.join().expect("client should join");
    assert!(String::from_utf8_lossy(&raw).starts_with("HTTP/1.1 200 OK"));
    let exchange = server.finish().expect("server should join");
    assert!(exchange.request.truncated);
    let evidence_path = fixture.roots().artifacts().join("bounded.http.evidence");
    assert_eq!(
        fs::read_to_string(evidence_path).expect("evidence should be written"),
        "method=POST\npath=/bounded\nauthorization_present=false\nauthorization_valid=true\nstatus=200\nbody_len=3\n"
    );
    cleanup(fixture);
}

#[test]
fn dropped_server_has_no_residue_after_successful_fixture_cleanup() {
    let mut fixture = LifecycleFixture::create(FixtureSpec::new("network-drop-cleanup", 7_921))
        .expect("fixture should be created");
    let root = fixture.roots().root().to_path_buf();
    let server = LocalHttpDouble::bind(&mut fixture, "dropped", HttpDoubleSpec::new(200, b"ok\n"))
        .expect("server should bind");
    let endpoint = server.endpoint().to_owned();
    drop(server);
    assert!(endpoint.starts_with("http://127.0.0.1:"));
    let report = fixture.cleanup(FixtureOutcome::Success);
    assert!(report.removed);
    assert!(report.leaks.is_empty());
    assert!(!root.exists(), "drop cleanup must remove the fixture root");
}

#[test]
fn network_error_projections_are_stable() {
    let protocol = NetworkDoubleError::Protocol("bad request".to_owned());
    assert_eq!(
        protocol.to_string(),
        "network double protocol error: bad request"
    );
    assert!(std::error::Error::source(&protocol).is_none());

    let thread = NetworkDoubleError::Thread("panic".to_owned());
    assert_eq!(thread.to_string(), "network double thread error: panic");
    let fixture = NetworkDoubleError::from(FixtureError::Invariant("broken".to_owned()));
    assert_eq!(
        fixture.to_string(),
        "network double fixture error: fixture invariant failed: broken"
    );
    let io = NetworkDoubleError::from(std::io::Error::other("broken"));
    assert_eq!(io.to_string(), "network double I/O error: broken");
}
