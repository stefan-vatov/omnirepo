//! Focused contract coverage for the deterministic agent protocol double.
//!
//! These tests keep malformed protocol input, barrier misuse, error projection,
//! and drop cleanup visible without widening the production harness API.

use std::{error::Error, fs, io};

use omnirepo_test_support::{
    agent_double::{
        AgentDouble, AgentDoubleError, AgentMessage, AgentProtocolViolation, parse_agent_json,
    },
    lifecycle_fixture::{FixtureError, FixtureOutcome, FixtureSpec, LifecycleFixture},
};

#[test]
fn parser_accepts_whitespace_and_escaped_string_content() {
    let message =
        parse_agent_json(r#" { "kind" : "result,with:delimiters", "status" : "a\"b\\c" } "#)
            .expect("documented JSON-line grammar should accept escaped strings");

    assert_eq!(
        message,
        AgentMessage {
            kind: "result,with:delimiters".to_owned(),
            status: "a\"b\\c".to_owned(),
        }
    );
}

#[test]
fn parser_rejects_missing_duplicate_unexpected_and_empty_members() {
    let cases = [
        (r#"{}"#, AgentProtocolViolation::MissingField("kind")),
        (
            r#"{"kind":"result"}"#,
            AgentProtocolViolation::MissingField("status"),
        ),
        (
            r#"{"kind":"first","kind":"second","status":"ok"}"#,
            AgentProtocolViolation::DuplicateField("kind".to_owned()),
        ),
        (
            r#"{"kind":"result","status":"ok","extra":"x"}"#,
            AgentProtocolViolation::UnexpectedField("extra".to_owned()),
        ),
        (
            r#"{"kind":"result",,"status":"ok"}"#,
            AgentProtocolViolation::MalformedJson,
        ),
        (
            r#"{"kind":"result","status":"ok",}"#,
            AgentProtocolViolation::MalformedJson,
        ),
    ];

    for (line, expected) in cases {
        assert_eq!(parse_agent_json(line), Err(expected), "line={line:?}");
    }
}

#[test]
fn parser_rejects_invalid_quoted_strings_and_trailing_escapes() {
    let cases = [
        (r#"not-json"#, AgentProtocolViolation::MalformedJson),
        (
            r#"["kind","result"]"#,
            AgentProtocolViolation::MalformedJson,
        ),
        (
            r#"{"kind":"result","status":"ok""#,
            AgentProtocolViolation::MalformedJson,
        ),
        (r#"{"kind":"result\"#, AgentProtocolViolation::MalformedJson),
        (
            r#"{"kind":result,"status":"ok"}"#,
            AgentProtocolViolation::InvalidString,
        ),
        (
            r#"{"kind":"result",status:"ok"}"#,
            AgentProtocolViolation::InvalidString,
        ),
        (
            "{\"kind\":\"result\",\"status\":\"line\nfeed\"}",
            AgentProtocolViolation::InvalidString,
        ),
    ];

    for (line, expected) in cases {
        assert_eq!(parse_agent_json(line), Err(expected), "line={line:?}");
    }
}

#[test]
fn parser_distinguishes_an_even_escape_from_a_trailing_escape() {
    let message = parse_agent_json(r#"{"kind":"a\\","status":"ok"}"#)
        .expect("an even escape pair should close the string");
    assert_eq!(message.kind, r"a\");

    assert_eq!(
        parse_agent_json(r#"{"kind":"a\","status":"ok"}"#),
        Err(AgentProtocolViolation::MalformedJson)
    );
}

#[test]
fn agent_errors_have_stable_display_source_and_conversions() {
    let fixture = AgentDoubleError::from(FixtureError::Invariant("bad barrier".to_owned()));
    assert_eq!(
        fixture.to_string(),
        "agent double fixture error: fixture invariant failed: bad barrier"
    );
    assert!(fixture.source().is_none());

    let io_error = AgentDoubleError::from(io::Error::new(io::ErrorKind::PermissionDenied, "no"));
    assert_eq!(io_error.to_string(), "agent double I/O error: no");
    assert!(io_error.source().is_none());

    let protocol = AgentDoubleError::Protocol("bad line".to_owned());
    assert_eq!(
        protocol.to_string(),
        "agent double protocol error: bad line"
    );
    assert!(protocol.source().is_none());

    let thread = AgentDoubleError::Thread("dead worker".to_owned());
    assert_eq!(thread.to_string(), "agent double thread error: dead worker");
    assert!(thread.source().is_none());
}

#[test]
fn release_before_hit_is_rejected_then_normal_release_succeeds() {
    let mut fixture = LifecycleFixture::create(FixtureSpec::new("agent-release-order", 7_601))
        .expect("fixture should be created");
    // Deterministic barrier semantics first: a barrier that was never hit
    // rejects release with the exact invariant.  (The live agent session
    // below races its own thread, so the rejection is proven here.)
    let barrier = fixture
        .barriers()
        .arm("release-order-before-hit")
        .expect("arm barrier");
    assert!(matches!(
        barrier.release(),
        Err(FixtureError::Invariant(message)) if message == "barrier was not hit"
    ));
    // hit() blocks until release() (the barrier protocol), so the hit runs
    // on its own thread and the main thread waits for it.
    let barrier_for_thread = barrier.clone();
    let hit_thread = std::thread::spawn(move || {
        barrier_for_thread.hit().expect("hit barrier");
    });
    barrier.wait_for_hit().expect("barrier hit observed");
    barrier.release().expect("release after hit");
    hit_thread.join().expect("hit thread");

    let session = AgentDouble::start(
        &mut fixture,
        "release-order",
        vec![r#"{"kind":"result","status":"ok"}"#.to_owned()],
    )
    .expect("agent should start");
    session
        .wait_for_barrier()
        .expect("agent should report its barrier");
    session.release().expect("first release should succeed");
    let evidence = session.join().expect("agent should be reaped");
    assert_eq!(evidence.accepted.len(), 1);
    assert!(fixture.cleanup(FixtureOutcome::Success).leaks.is_empty());
}

#[test]
fn repeated_release_is_rejected_and_evidence_is_written() {
    let mut fixture = LifecycleFixture::create(FixtureSpec::new("agent-repeat-release", 7_602))
        .expect("fixture should be created");
    let session = AgentDouble::start(
        &mut fixture,
        "repeat-release",
        vec![r#"{"kind":"result","status":"ok"}"#.to_owned()],
    )
    .expect("agent should start");
    session
        .wait_for_barrier()
        .expect("agent should report its barrier");
    session.release().expect("first release should succeed");
    assert!(matches!(
        session.release(),
        Err(AgentDoubleError::Fixture(FixtureError::Invariant(message)))
            if message == "barrier was not hit"
    ));
    let evidence = session.join().expect("agent should be reaped");
    assert_eq!(evidence.barrier, "released");

    let evidence_path = fixture
        .roots()
        .artifacts()
        .join("repeat-release.agent.evidence");
    assert_eq!(
        fs::read_to_string(evidence_path).expect("agent evidence should be persisted"),
        format!(
            "home={}\nbarrier=released\nambient_credentials_absent=true\naccepted=1\nviolations=0\n",
            fixture
                .environment()
                .value("HOME")
                .expect("HOME should exist")
        )
    );
    assert!(fixture.cleanup(FixtureOutcome::Success).leaks.is_empty());
}

#[test]
fn dropping_a_live_session_aborts_and_reaps_without_residue() {
    let mut fixture = LifecycleFixture::create(FixtureSpec::new("agent-drop-abort", 7_603))
        .expect("fixture should be created");
    let evidence_path = fixture
        .roots()
        .artifacts()
        .join("drop-abort.agent.evidence");
    {
        let session =
            AgentDouble::start(&mut fixture, "drop-abort", Vec::new()).expect("agent should start");
        session
            .wait_for_barrier()
            .expect("agent should report its barrier");
        // Drop must abort the worker and join it. The aborted worker must not
        // publish success evidence after its barrier is cancelled.
    }
    assert!(
        !evidence_path.exists(),
        "aborted session must not publish evidence"
    );
    let report = fixture.cleanup(FixtureOutcome::Success);
    assert!(report.removed);
    assert!(report.leaks.is_empty());
}

#[test]
fn start_rejects_an_evidence_path_that_escapes_fixture_roots() {
    let mut fixture = LifecycleFixture::create(FixtureSpec::new("agent-invalid-case", 7_604))
        .expect("fixture should be created");
    let error = match AgentDouble::start(&mut fixture, "../escape", Vec::new()) {
        Ok(_) => panic!("case IDs must not escape the artifact root"),
        Err(error) => error,
    };
    assert!(matches!(
        error,
        AgentDoubleError::Fixture(FixtureError::InvalidPath { .. })
    ));
    assert!(
        error
            .to_string()
            .contains("parent traversal is not accepted")
    );
    let report = fixture.cleanup(FixtureOutcome::Success);
    assert!(report.removed);
    assert!(report.leaks.is_empty());
}
