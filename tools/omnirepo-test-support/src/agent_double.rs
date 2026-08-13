#![allow(dead_code)]

// Shared hermetic agent protocol double; owned by the private test-support crate.

use std::{
    collections::BTreeMap,
    fs,
    thread::{self, JoinHandle},
};

use super::lifecycle_fixture::{DeterministicBarrier, FixtureError, LifecycleFixture};

#[derive(Debug)]
pub enum AgentDoubleError {
    Fixture(FixtureError),
    Io(std::io::Error),
    Protocol(String),
    Thread(String),
}

impl std::fmt::Display for AgentDoubleError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Fixture(error) => write!(formatter, "agent double fixture error: {error}"),
            Self::Io(error) => write!(formatter, "agent double I/O error: {error}"),
            Self::Protocol(message) => write!(formatter, "agent double protocol error: {message}"),
            Self::Thread(message) => write!(formatter, "agent double thread error: {message}"),
        }
    }
}

impl std::error::Error for AgentDoubleError {}

impl From<FixtureError> for AgentDoubleError {
    fn from(error: FixtureError) -> Self {
        Self::Fixture(error)
    }
}

impl From<std::io::Error> for AgentDoubleError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum AgentProtocolViolation {
    MalformedJson,
    MissingField(&'static str),
    UnexpectedField(String),
    DuplicateField(String),
    InvalidString,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct AgentMessage {
    pub kind: String,
    pub status: String,
}

pub fn parse_agent_json(line: &str) -> Result<AgentMessage, AgentProtocolViolation> {
    let trimmed = line.trim();
    if !trimmed.starts_with('{') || !trimmed.ends_with('}') {
        return Err(AgentProtocolViolation::MalformedJson);
    }
    let inner = &trimmed[1..trimmed.len() - 1];
    let mut fields = BTreeMap::new();
    if !inner.trim().is_empty() {
        for member in split_members(inner)? {
            let (raw_key, raw_value) = member
                .split_once(':')
                .ok_or(AgentProtocolViolation::MalformedJson)?;
            let key = parse_json_string(raw_key.trim())?;
            if fields
                .insert(key.clone(), parse_json_string(raw_value.trim())?)
                .is_some()
            {
                return Err(AgentProtocolViolation::DuplicateField(key));
            }
        }
    }
    for key in fields.keys() {
        if key != "kind" && key != "status" {
            return Err(AgentProtocolViolation::UnexpectedField(key.clone()));
        }
    }
    let kind = fields
        .remove("kind")
        .ok_or(AgentProtocolViolation::MissingField("kind"))?;
    let status = fields
        .remove("status")
        .ok_or(AgentProtocolViolation::MissingField("status"))?;
    Ok(AgentMessage { kind, status })
}

fn split_members(input: &str) -> Result<Vec<&str>, AgentProtocolViolation> {
    let mut members = Vec::new();
    let mut start = 0;
    let mut quoted = false;
    let mut escaped = false;
    for (index, byte) in input.bytes().enumerate() {
        match byte {
            b'\\' if quoted => escaped = !escaped,
            b'"' if !escaped => quoted = !quoted,
            b',' if !quoted => {
                members.push(input[start..index].trim());
                start = index + 1;
            }
            _ => escaped = false,
        }
    }
    if quoted || escaped {
        return Err(AgentProtocolViolation::MalformedJson);
    }
    members.push(input[start..].trim());
    if members.iter().any(|member| member.is_empty()) {
        return Err(AgentProtocolViolation::MalformedJson);
    }
    Ok(members)
}

fn parse_json_string(input: &str) -> Result<String, AgentProtocolViolation> {
    if input.len() < 2 || !input.starts_with('"') || !input.ends_with('"') {
        return Err(AgentProtocolViolation::InvalidString);
    }
    let inner = &input[1..input.len() - 1];
    let mut value = String::with_capacity(inner.len());
    let mut escaped = false;
    for character in inner.chars() {
        if escaped {
            match character {
                '"' | '\\' => value.push(character),
                _ => return Err(AgentProtocolViolation::InvalidString),
            }
            escaped = false;
        } else {
            match character {
                '\\' => escaped = true,
                '"' | '\n' | '\r' => return Err(AgentProtocolViolation::InvalidString),
                _ => value.push(character),
            }
        }
    }
    if escaped {
        return Err(AgentProtocolViolation::InvalidString);
    }
    Ok(value)
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct AgentEvidence {
    pub home: String,
    pub barrier: String,
    pub ambient_credentials_absent: bool,
    pub accepted: Vec<AgentMessage>,
    pub violations: Vec<AgentProtocolViolation>,
}

pub struct AgentSession {
    barrier: DeterministicBarrier,
    join: Option<JoinHandle<Result<AgentEvidence, AgentDoubleError>>>,
}

impl AgentSession {
    pub fn wait_for_barrier(&self) -> Result<(), AgentDoubleError> {
        self.barrier.wait_for_hit().map_err(AgentDoubleError::from)
    }

    pub fn release(&self) -> Result<(), AgentDoubleError> {
        self.barrier.release().map_err(AgentDoubleError::from)
    }

    pub fn join(mut self) -> Result<AgentEvidence, AgentDoubleError> {
        let join = self
            .join
            .take()
            .ok_or_else(|| AgentDoubleError::Thread("agent was already joined".to_owned()))?;
        join.join()
            .map_err(|_| AgentDoubleError::Thread("agent thread panicked".to_owned()))?
    }
}

impl Drop for AgentSession {
    fn drop(&mut self) {
        if self.join.is_none() {
            return;
        }
        self.barrier.abort();
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

pub struct AgentDouble;

impl AgentDouble {
    pub fn start(
        fixture: &mut LifecycleFixture,
        case_id: &str,
        output_lines: Vec<String>,
    ) -> Result<AgentSession, AgentDoubleError> {
        let barrier = fixture
            .barriers()
            .arm(format!("agent-{case_id}"))
            .map_err(AgentDoubleError::from)?;
        let home = fixture
            .environment()
            .value("HOME")
            .ok_or_else(|| AgentDoubleError::Protocol("fixture has no HOME".to_owned()))?
            .to_owned();
        let ambient_credentials_absent = fixture.environment().vars().keys().all(|key| {
            !matches!(
                key.as_str(),
                "SSH_AUTH_SOCK" | "AWS_ACCESS_KEY_ID" | "AWS_SECRET_ACCESS_KEY" | "GITHUB_TOKEN"
            )
        });
        let evidence_path = fixture.roots().resolve(
            super::lifecycle_fixture::RootKind::Artifacts,
            &format!("{case_id}.agent.evidence"),
        )?;
        fixture.track_ephemeral(&evidence_path)?;
        let barrier_for_thread = barrier.clone();
        let evidence_for_thread = evidence_path.clone();
        let join = thread::Builder::new()
            .name(format!("omnirepo-agent-{case_id}"))
            .spawn(move || {
                barrier_for_thread.hit().map_err(AgentDoubleError::from)?;
                let mut accepted = Vec::new();
                let mut violations = Vec::new();
                for line in output_lines {
                    match parse_agent_json(&line) {
                        Ok(message) => accepted.push(message),
                        Err(error) => violations.push(error),
                    }
                }
                let evidence = AgentEvidence {
                    home,
                    barrier: "released".to_owned(),
                    ambient_credentials_absent,
                    accepted,
                    violations,
                };
                fs::write(&evidence_for_thread, evidence_lines(&evidence))?;
                Ok(evidence)
            })
            .map_err(|error| AgentDoubleError::Thread(error.to_string()))?;
        fixture.record(
            "double.agent.start",
            format!("case={case_id};protocol=json-lines;barrier=agent-{case_id}"),
        );
        Ok(AgentSession {
            barrier,
            join: Some(join),
        })
    }
}

fn evidence_lines(evidence: &AgentEvidence) -> String {
    format!(
        "home={}\nbarrier={}\nambient_credentials_absent={}\naccepted={}\nviolations={}\n",
        evidence.home,
        evidence.barrier,
        evidence.ambient_credentials_absent,
        evidence.accepted.len(),
        evidence.violations.len()
    )
}
