//! Authenticated agent control framing and untrusted-output sanitation.
//!
//! Commands to repair agents travel in a versioned frame carrying an
//! authentication token and a bounded payload; malformed, unknown-version,
//! or oversized frames fail closed.  Agent stdout is untrusted: ANSI escape
//! sequences and control characters are stripped to inert text, and output
//! is bounded.

#![allow(dead_code)]

use std::{error::Error, fmt};

#[cfg(test)]
mod agent_framing_tests;

/// Framing protocol version.
pub const FRAME_VERSION: &str = "omnirepo-control-v1";
/// Maximum accepted payload bytes.
pub const MAX_FRAME_PAYLOAD_BYTES: usize = 64 * 1024;
/// Maximum accepted token bytes.
pub const MAX_TOKEN_BYTES: usize = 128;

/// One authenticated control frame.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ControlFrame {
    pub token: String,
    pub payload: String,
}

/// Typed framing failures.
#[derive(Debug)]
pub enum FrameError {
    Malformed { reason: String },
    UnknownVersion { version: String },
    Oversized { field: &'static str },
}

impl fmt::Display for FrameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed { reason } => write!(formatter, "malformed control frame: {reason}"),
            Self::UnknownVersion { version } => {
                write!(formatter, "unknown control frame version {version:?}")
            }
            Self::Oversized { field } => {
                write!(formatter, "control frame {field} exceeds its bound")
            }
        }
    }
}
impl Error for FrameError {}

/// Build a control frame: `omnirepo-control-v1 token=<token> payload=<len>:<payload>`.
pub fn frame(token: &str, payload: &str) -> Result<String, FrameError> {
    if token.is_empty() || token.len() > MAX_TOKEN_BYTES {
        return Err(FrameError::Oversized { field: "token" });
    }
    if token.bytes().any(|byte| !(0x21..=0x7e).contains(&byte)) {
        return Err(FrameError::Malformed {
            reason: "token must be printable ASCII".to_owned(),
        });
    }
    if payload.len() > MAX_FRAME_PAYLOAD_BYTES {
        return Err(FrameError::Oversized { field: "payload" });
    }
    if payload.contains('\n') || payload.contains('\r') {
        return Err(FrameError::Malformed {
            reason: "payload must be single-line".to_owned(),
        });
    }
    Ok(format!(
        "{FRAME_VERSION} token={token} payload={}:{payload}",
        payload.len()
    ))
}

/// Parse and authenticate a frame; any deviation fails closed.
pub fn parse_frame(line: &str) -> Result<ControlFrame, FrameError> {
    let Some(rest) = line.strip_prefix(FRAME_VERSION) else {
        let version = line.split_whitespace().next().unwrap_or("<empty>");
        return Err(FrameError::UnknownVersion {
            version: version.to_owned(),
        });
    };
    let rest = rest.trim_start();
    let Some(token_value) = rest.strip_prefix("token=") else {
        return Err(FrameError::Malformed {
            reason: "missing token field".to_owned(),
        });
    };
    let (token, rest) =
        token_value
            .split_once(char::is_whitespace)
            .ok_or_else(|| FrameError::Malformed {
                reason: "missing payload field".to_owned(),
            })?;
    let rest = rest.trim_start();
    let Some(payload_value) = rest.strip_prefix("payload=") else {
        return Err(FrameError::Malformed {
            reason: "missing payload field".to_owned(),
        });
    };
    let (length_text, payload) =
        payload_value
            .split_once(':')
            .ok_or_else(|| FrameError::Malformed {
                reason: "payload length missing".to_owned(),
            })?;
    let declared: usize = length_text.parse().map_err(|_| FrameError::Malformed {
        reason: "payload length is not an integer".to_owned(),
    })?;
    if declared != payload.len() || declared > MAX_FRAME_PAYLOAD_BYTES {
        return Err(FrameError::Oversized { field: "payload" });
    }
    if token.is_empty() || token.len() > MAX_TOKEN_BYTES {
        return Err(FrameError::Oversized { field: "token" });
    }
    Ok(ControlFrame {
        token: token.to_owned(),
        payload: payload.to_owned(),
    })
}

/// Sanitize untrusted agent output: strip ANSI escape sequences and control
/// characters (newlines and tabs survive), bound the result.
pub fn sanitize_output(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut in_escape = false;
    for character in text.chars() {
        if in_escape {
            if ('@'..='~').contains(&character) {
                in_escape = false;
            }
            continue;
        }
        if character == '\u{1b}' {
            in_escape = true;
            continue;
        }
        let code = character as u32;
        if code < 0x20 && !matches!(character, '\n' | '\t') {
            continue; // C0 control characters are inert
        }
        if code == 0x7f {
            continue;
        }
        output.push(character);
        if output.len() >= MAX_FRAME_PAYLOAD_BYTES {
            break;
        }
    }
    output
}
