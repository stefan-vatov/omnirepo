#![allow(dead_code)]

// Shared hermetic Git double; owned by the private test-support crate.

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
pub enum GitDoubleError {
    Io(io::Error),
    Fixture(FixtureError),
    Protocol(String),
    Thread(String),
}

impl std::fmt::Display for GitDoubleError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "Git double I/O error: {error}"),
            Self::Fixture(error) => write!(formatter, "Git double fixture error: {error}"),
            Self::Protocol(message) => write!(formatter, "Git double protocol error: {message}"),
            Self::Thread(message) => write!(formatter, "Git double thread error: {message}"),
        }
    }
}

impl std::error::Error for GitDoubleError {}

impl From<io::Error> for GitDoubleError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<FixtureError> for GitDoubleError {
    fn from(error: FixtureError) -> Self {
        Self::Fixture(error)
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct GitPushEvidence {
    pub payload: Vec<u8>,
    pub accepted: bool,
    pub disconnected: bool,
}

pub struct GitPushAttempt {
    join: Option<JoinHandle<Result<Vec<u8>, GitDoubleError>>>,
}

impl GitPushAttempt {
    pub fn join(mut self) -> Result<Vec<u8>, GitDoubleError> {
        let join = self
            .join
            .take()
            .ok_or_else(|| GitDoubleError::Protocol("push attempt already joined".to_owned()))?;
        join.join()
            .map_err(|_| GitDoubleError::Thread("push client panicked".to_owned()))?
    }
}

impl Drop for GitPushAttempt {
    fn drop(&mut self) {
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

pub struct LocalGitRemoteDouble {
    endpoint: String,
    attempt_rx: Receiver<GitPushEvidence>,
    release_tx: SyncSender<()>,
    join: Option<JoinHandle<Result<GitPushEvidence, GitDoubleError>>>,
    evidence_path: PathBuf,
}

impl LocalGitRemoteDouble {
    pub fn bind(fixture: &mut LifecycleFixture, case_id: &str) -> Result<Self, GitDoubleError> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let endpoint = format!("git://{}", listener.local_addr()?);
        let evidence_path = fixture
            .roots()
            .artifacts()
            .join(format!("{case_id}.git.evidence"));
        fixture.track_ephemeral(&evidence_path)?;
        let (attempt_tx, attempt_rx) = mpsc::sync_channel(1);
        let (release_tx, release_rx) = mpsc::sync_channel(1);
        let evidence_for_thread = evidence_path.clone();
        let join = thread::Builder::new()
            .name(format!("omnirepo-git-remote-{case_id}"))
            .spawn(move || {
                let (mut stream, _) = listener.accept()?;
                let payload = read_all(&mut stream)?;
                let evidence = GitPushEvidence {
                    payload,
                    accepted: true,
                    disconnected: false,
                };
                attempt_tx
                    .send(evidence.clone())
                    .map_err(|_| GitDoubleError::Protocol("attempt receiver dropped".to_owned()))?;
                release_rx.recv().map_err(|_| {
                    GitDoubleError::Protocol("disconnect was not released".to_owned())
                })?;
                drop(stream);
                let final_evidence = GitPushEvidence {
                    disconnected: true,
                    ..evidence
                };
                fs::write(&evidence_for_thread, evidence_lines(&final_evidence))?;
                Ok(final_evidence)
            })
            .map_err(|error| GitDoubleError::Thread(error.to_string()))?;
        fixture.record(
            "double.git.bind",
            format!("case={case_id};endpoint={endpoint};outcome=accepted-then-disconnected"),
        );
        Ok(Self {
            endpoint,
            attempt_rx,
            release_tx,
            join: Some(join),
            evidence_path,
        })
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub fn begin_attempt(
        &self,
        payload: impl Into<Vec<u8>>,
    ) -> Result<GitPushAttempt, GitDoubleError> {
        let address = self
            .endpoint
            .strip_prefix("git://")
            .ok_or_else(|| GitDoubleError::Protocol("endpoint is not local Git".to_owned()))?
            .to_owned();
        let payload = payload.into();
        let join = thread::Builder::new()
            .name("omnirepo-git-push-attempt".to_owned())
            .spawn(move || {
                let mut stream = TcpStream::connect(address)?;
                stream.write_all(&payload)?;
                stream.shutdown(Shutdown::Write)?;
                let mut response = Vec::new();
                stream.read_to_end(&mut response)?;
                Ok(response)
            })
            .map_err(|error| GitDoubleError::Thread(error.to_string()))?;
        Ok(GitPushAttempt { join: Some(join) })
    }

    pub fn wait_for_accept(&self) -> Result<GitPushEvidence, GitDoubleError> {
        self.attempt_rx.recv().map_err(|_| {
            GitDoubleError::Protocol("remote ended before accepting payload".to_owned())
        })
    }

    pub fn disconnect(&self) -> Result<(), GitDoubleError> {
        self.release_tx
            .send(())
            .map_err(|_| GitDoubleError::Protocol("remote was already disconnected".to_owned()))
    }

    pub fn finish(mut self) -> Result<GitPushEvidence, GitDoubleError> {
        let join = self
            .join
            .take()
            .ok_or_else(|| GitDoubleError::Protocol("remote was already joined".to_owned()))?;
        join.join()
            .map_err(|_| GitDoubleError::Thread("Git remote panicked".to_owned()))?
    }
}

impl Drop for LocalGitRemoteDouble {
    fn drop(&mut self) {
        if self.join.is_none() {
            return;
        }
        let _ = self.release_tx.send(());
        let address = self.endpoint.strip_prefix("git://").unwrap_or_default();
        if let Ok(stream) = TcpStream::connect(address) {
            let _ = stream.shutdown(Shutdown::Both);
        }
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

fn read_all(stream: &mut TcpStream) -> io::Result<Vec<u8>> {
    let mut payload = Vec::new();
    stream.read_to_end(&mut payload)?;
    Ok(payload)
}

fn evidence_lines(evidence: &GitPushEvidence) -> String {
    format!(
        "accepted={}\ndisconnected={}\npayload_len={}\n",
        evidence.accepted,
        evidence.disconnected,
        evidence.payload.len()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lifecycle_fixture::{FixtureOutcome, FixtureSpec};
    use std::sync::mpsc;

    fn inert_remote(
        endpoint: &str,
        attempt_rx: Receiver<GitPushEvidence>,
        release_tx: SyncSender<()>,
    ) -> LocalGitRemoteDouble {
        LocalGitRemoteDouble {
            endpoint: endpoint.to_owned(),
            attempt_rx,
            release_tx,
            join: None,
            evidence_path: PathBuf::new(),
        }
    }

    #[test]
    fn push_attempt_reports_already_joined_and_panicked_children() {
        let already_joined = GitPushAttempt { join: None };
        assert_eq!(
            already_joined.join().unwrap_err().to_string(),
            "Git double protocol error: push attempt already joined"
        );

        let panicked = thread::spawn(|| -> Result<Vec<u8>, GitDoubleError> {
            panic!("injected push panic");
        });
        let error = GitPushAttempt {
            join: Some(panicked),
        }
        .join()
        .expect_err("panicked push client should become a typed error");
        assert_eq!(
            error.to_string(),
            "Git double thread error: push client panicked"
        );
    }

    #[test]
    fn push_attempt_drop_reaps_a_completed_child() {
        let child = thread::spawn(|| Ok::<Vec<u8>, GitDoubleError>(Vec::new()));
        drop(GitPushAttempt { join: Some(child) });
    }

    #[test]
    fn remote_rejects_invalid_endpoint_and_reports_disconnected_channels() {
        let (attempt_tx, attempt_rx) = mpsc::sync_channel(1);
        drop(attempt_tx);
        let (release_tx, release_rx) = mpsc::sync_channel(1);
        drop(release_rx);
        let remote = inert_remote("http://not-a-local-git-endpoint", attempt_rx, release_tx);
        let error = match remote.begin_attempt(b"payload") {
            Ok(_) => panic!("invalid endpoint should be rejected"),
            Err(error) => error,
        };
        assert_eq!(
            error.to_string(),
            "Git double protocol error: endpoint is not local Git"
        );
        assert_eq!(
            remote.wait_for_accept().unwrap_err().to_string(),
            "Git double protocol error: remote ended before accepting payload"
        );
        assert_eq!(
            remote.disconnect().unwrap_err().to_string(),
            "Git double protocol error: remote was already disconnected"
        );
        assert_eq!(
            remote.finish().unwrap_err().to_string(),
            "Git double protocol error: remote was already joined"
        );
    }

    #[test]
    fn remote_join_reports_a_panicked_child() {
        let (attempt_tx, attempt_rx) = mpsc::sync_channel(1);
        drop(attempt_tx);
        let (release_tx, _release_rx) = mpsc::sync_channel(1);
        let join = thread::spawn(|| -> Result<GitPushEvidence, GitDoubleError> {
            panic!("injected remote panic");
        });
        let remote = LocalGitRemoteDouble {
            endpoint: "git://127.0.0.1:0".to_owned(),
            attempt_rx,
            release_tx,
            join: Some(join),
            evidence_path: PathBuf::new(),
        };
        assert_eq!(
            remote.finish().unwrap_err().to_string(),
            "Git double thread error: Git remote panicked"
        );
    }

    #[test]
    fn bind_rejects_evidence_paths_outside_the_fixture_root() {
        let mut fixture = LifecycleFixture::create(FixtureSpec::new("git-bind-path", 8_105))
            .expect("fixture should be created");
        let error = match LocalGitRemoteDouble::bind(&mut fixture, "/tmp") {
            Ok(_) => panic!("absolute case IDs must not escape the fixture"),
            Err(error) => error,
        };
        assert!(matches!(error, GitDoubleError::Fixture(_)));
        let report = fixture.cleanup(FixtureOutcome::Success);
        assert!(report.removed);
        assert!(report.leaks.is_empty());
    }

    #[test]
    fn accepting_after_the_attempt_receiver_is_dropped_reports_all_channel_failures() {
        let mut fixture = LifecycleFixture::create(FixtureSpec::new("git-channel-faults", 8_106))
            .expect("fixture should be created");
        let mut remote = LocalGitRemoteDouble::bind(&mut fixture, "channel-faults")
            .expect("local remote should bind");
        let (replacement_tx, replacement_rx) = mpsc::sync_channel(1);
        drop(replacement_tx);
        let old_attempt_rx = std::mem::replace(&mut remote.attempt_rx, replacement_rx);
        drop(old_attempt_rx);

        let attempt = remote
            .begin_attempt(b"channel-fault")
            .expect("push attempt should start");
        assert_eq!(
            remote.wait_for_accept().unwrap_err().to_string(),
            "Git double protocol error: remote ended before accepting payload"
        );
        assert!(
            attempt
                .join()
                .expect("client should observe remote closure")
                .is_empty()
        );
        // The client observes socket EOF before the remote thread necessarily
        // drops its release receiver. Join the remote before checking its
        // typed failure; the standalone inert-remote test covers disconnect's
        // closed-channel projection without relying on scheduling.
        assert_eq!(
            remote.finish().unwrap_err().to_string(),
            "Git double protocol error: attempt receiver dropped"
        );

        let report = fixture.cleanup(FixtureOutcome::Success);
        assert!(report.removed);
        assert!(report.leaks.is_empty());
    }
}
