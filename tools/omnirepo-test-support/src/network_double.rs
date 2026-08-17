#![allow(dead_code)]

// Shared hermetic network double; owned by the private test-support crate.

// Shared hermetic network double; owned by the private test-support crate.

use std::{
    fs,
    io::{self, Read, Write},
    net::{Shutdown, TcpListener, TcpStream},
    path::PathBuf,
    sync::mpsc::{self, Receiver, SyncSender},
    thread::{self, JoinHandle},
};

use super::lifecycle_fixture::{FixtureError, LifecycleFixture};

#[derive(Debug)]
pub enum NetworkDoubleError {
    Io(io::Error),
    Fixture(FixtureError),
    Protocol(String),
    Thread(String),
}

impl std::fmt::Display for NetworkDoubleError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "network double I/O error: {error}"),
            Self::Fixture(error) => write!(formatter, "network double fixture error: {error}"),
            Self::Protocol(message) => {
                write!(formatter, "network double protocol error: {message}")
            }
            Self::Thread(message) => write!(formatter, "network double thread error: {message}"),
        }
    }
}

impl std::error::Error for NetworkDoubleError {}

impl From<io::Error> for NetworkDoubleError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<FixtureError> for NetworkDoubleError {
    fn from(error: FixtureError) -> Self {
        Self::Fixture(error)
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct HttpDoubleSpec {
    pub status: u16,
    pub body: Vec<u8>,
    pub required_bearer: Option<String>,
    pub request_limit: usize,
}

impl HttpDoubleSpec {
    pub fn new(status: u16, body: impl Into<Vec<u8>>) -> Self {
        Self {
            status,
            body: body.into(),
            required_bearer: None,
            request_limit: 64 * 1024,
        }
    }

    pub fn requiring_bearer(mut self, token: impl Into<String>) -> Self {
        self.required_bearer = Some(token.into());
        self
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct HttpRequestEvidence {
    pub method: String,
    pub path: String,
    pub authorization_present: bool,
    pub authorization_valid: bool,
    pub body: Vec<u8>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct HttpResponseEvidence {
    pub status: u16,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct HttpExchangeEvidence {
    pub request: HttpRequestEvidence,
    pub response: HttpResponseEvidence,
}

pub struct LocalHttpDouble {
    endpoint: String,
    response_tx: SyncSender<HttpResponseEvidence>,
    request_rx: Receiver<HttpRequestEvidence>,
    join: Option<JoinHandle<Result<HttpExchangeEvidence, NetworkDoubleError>>>,
    request: Option<HttpRequestEvidence>,
    success_status: u16,
    success_body: Vec<u8>,
    response_released: bool,
    evidence_path: PathBuf,
}

impl LocalHttpDouble {
    pub fn bind(
        fixture: &mut LifecycleFixture,
        case_id: &str,
        spec: HttpDoubleSpec,
    ) -> Result<Self, NetworkDoubleError> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let address = listener.local_addr()?;
        let endpoint = format!("http://{address}");
        let evidence_path = fixture
            .roots()
            .artifacts()
            .join(format!("{case_id}.http.evidence"));
        fixture.track_ephemeral(&evidence_path)?;
        let (request_tx, request_rx) = mpsc::sync_channel(1);
        let (response_tx, response_rx) = mpsc::sync_channel(1);
        let evidence_for_thread = evidence_path.clone();
        let required_bearer = spec.required_bearer.clone();
        let request_limit = spec.request_limit;
        let join = thread::Builder::new()
            .name(format!("omnirepo-http-{case_id}"))
            .spawn(move || {
                let (mut stream, _) = listener.accept()?;
                let request = read_request(&mut stream, request_limit, required_bearer.as_deref())?;
                request_tx.send(request.clone()).map_err(|_| {
                    NetworkDoubleError::Protocol("request receiver dropped".to_owned())
                })?;
                let response = response_rx.recv().map_err(|_| {
                    NetworkDoubleError::Protocol("response was not released".to_owned())
                })?;
                write_response(&mut stream, &response)?;
                let evidence = HttpExchangeEvidence { request, response };
                fs::write(&evidence_for_thread, evidence_lines(&evidence))?;
                Ok(evidence)
            })
            .map_err(|error| NetworkDoubleError::Thread(error.to_string()))?;
        fixture.record(
            "double.network.bind",
            format!("case={case_id};endpoint={endpoint}"),
        );
        Ok(Self {
            endpoint,
            response_tx,
            request_rx,
            join: Some(join),
            request: None,
            success_status: spec.status,
            success_body: spec.body,
            response_released: false,
            evidence_path,
        })
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub fn wait_for_request(&mut self) -> Result<HttpRequestEvidence, NetworkDoubleError> {
        if self.request.is_some() {
            return Err(NetworkDoubleError::Protocol(
                "request barrier was already consumed".to_owned(),
            ));
        }
        let request = self
            .request_rx
            .recv()
            .map_err(|_| NetworkDoubleError::Protocol("request thread ended early".to_owned()))?;
        self.request = Some(request.clone());
        Ok(request)
    }

    pub fn respond(&mut self) -> Result<(), NetworkDoubleError> {
        if self.response_released {
            return Err(NetworkDoubleError::Protocol(
                "response was already released".to_owned(),
            ));
        }
        let request = self.request.as_ref().ok_or_else(|| {
            NetworkDoubleError::Protocol("respond called before request barrier".to_owned())
        })?;
        let (status, body) = if request.authorization_valid {
            (self.success_status, self.success_body.clone())
        } else {
            (401, b"unauthorized\n".to_vec())
        };
        self.response_tx
            .send(HttpResponseEvidence { status, body })
            .map_err(|_| NetworkDoubleError::Protocol("response thread ended early".to_owned()))?;
        self.response_released = true;
        Ok(())
    }

    pub fn finish(mut self) -> Result<HttpExchangeEvidence, NetworkDoubleError> {
        if self.request.is_some() && !self.response_released {
            return Err(NetworkDoubleError::Protocol(
                "finish called before response release".to_owned(),
            ));
        }
        let join = self
            .join
            .take()
            .ok_or_else(|| NetworkDoubleError::Protocol("server was already joined".to_owned()))?;
        join.join()
            .map_err(|_| NetworkDoubleError::Thread("server thread panicked".to_owned()))?
    }

    pub fn send_request(endpoint: &str, request: &[u8]) -> Result<Vec<u8>, NetworkDoubleError> {
        let address = endpoint
            .strip_prefix("http://")
            .ok_or_else(|| NetworkDoubleError::Protocol("endpoint is not local HTTP".to_owned()))?;
        let mut stream = TcpStream::connect(address)?;
        stream.write_all(request)?;
        stream.shutdown(Shutdown::Write)?;
        let mut response = Vec::new();
        stream.read_to_end(&mut response)?;
        Ok(response)
    }
}

impl Drop for LocalHttpDouble {
    fn drop(&mut self) {
        if self.join.is_none() {
            return;
        }
        let _ = self.response_tx.send(HttpResponseEvidence {
            status: 500,
            body: b"double dropped\n".to_vec(),
        });
        if let Ok(stream) = TcpStream::connect(
            self.endpoint
                .strip_prefix("http://")
                .unwrap_or("127.0.0.1:0"),
        ) {
            let _ = stream.shutdown(Shutdown::Both);
        }
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

fn read_request(
    stream: &mut TcpStream,
    limit: usize,
    required_bearer: Option<&str>,
) -> Result<HttpRequestEvidence, NetworkDoubleError> {
    let (raw, truncated) = read_limited(stream, limit)?;
    let text = String::from_utf8_lossy(&raw);
    let mut sections = text.split("\r\n\r\n");
    let headers = sections
        .next()
        .ok_or_else(|| NetworkDoubleError::Protocol("request has no headers".to_owned()))?;
    let body = sections.collect::<Vec<_>>().join("\r\n\r\n").into_bytes();
    let mut lines = headers.lines();
    let first = lines
        .next()
        .ok_or_else(|| NetworkDoubleError::Protocol("request has no start line".to_owned()))?;
    let mut first_parts = first.split_whitespace();
    let method = first_parts.next().unwrap_or_default().to_owned();
    let path = first_parts.next().unwrap_or_default().to_owned();
    let mut authorization = None;
    let mut duplicate_authorization = false;
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("authorization") {
                if authorization.is_some() {
                    duplicate_authorization = true;
                } else {
                    authorization = Some(value.trim().to_owned());
                }
            }
        }
    }
    let authorization_present = authorization.is_some();
    let authorization_valid = if duplicate_authorization {
        false
    } else {
        match (authorization.as_deref(), required_bearer) {
            (Some(value), Some(token)) => value == format!("Bearer {token}"),
            (None, None) => true,
            _ => false,
        }
    };
    Ok(HttpRequestEvidence {
        method,
        path,
        authorization_present,
        authorization_valid,
        body,
        truncated,
    })
}

fn write_response(
    stream: &mut TcpStream,
    response: &HttpResponseEvidence,
) -> Result<(), NetworkDoubleError> {
    let reason = match response.status {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        401 => "Unauthorized",
        404 => "Not Found",
        409 => "Conflict",
        500 => "Internal Server Error",
        _ => "Fixture Status",
    };
    write!(
        stream,
        "HTTP/1.1 {} {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        response.status,
        reason,
        response.body.len()
    )?;
    stream.write_all(&response.body)?;
    // The shutdown signals EOF for "Connection: close".  The client may
    // already have closed after reading the response, in which case macOS
    // reports ENOTCONN; the exchange evidence stands either way.
    let _ = stream.shutdown(Shutdown::Both);
    Ok(())
}

fn read_limited(stream: &mut TcpStream, limit: usize) -> io::Result<(Vec<u8>, bool)> {
    let mut bytes = Vec::new();
    let mut truncated = false;
    let mut buffer = [0_u8; 8192];
    loop {
        let count = stream.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        if bytes.len() < limit {
            let remaining = limit - bytes.len();
            let kept = remaining.min(count);
            bytes.extend_from_slice(&buffer[..kept]);
            if kept < count {
                truncated = true;
            }
        } else {
            truncated = true;
        }
    }
    Ok((bytes, truncated))
}

fn evidence_lines(evidence: &HttpExchangeEvidence) -> String {
    format!(
        "method={}\npath={}\nauthorization_present={}\nauthorization_valid={}\nstatus={}\nbody_len={}\n",
        evidence.request.method,
        evidence.request.path,
        evidence.request.authorization_present,
        evidence.request.authorization_valid,
        evidence.response.status,
        evidence.response.body.len()
    )
}
