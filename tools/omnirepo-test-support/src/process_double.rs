#![allow(dead_code)]

// Shared hermetic process double; owned by the private test-support crate.

// Shared hermetic process double; owned by the private test-support crate.

use std::{
    fs,
    io::{self, BufRead, BufReader, Read, Write},
    path::PathBuf,
    process::{Child, ChildStdin, Command, ExitStatus, Stdio},
};

use super::lifecycle_fixture::{Capability, FixtureError, LifecycleFixture};

const DEFAULT_OUTPUT_LIMIT: usize = 1024 * 1024;

#[derive(Debug)]
pub enum ProcessDoubleError {
    Io(io::Error),
    Fixture(FixtureError),
    Protocol(String),
}

impl std::fmt::Display for ProcessDoubleError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "process double I/O error: {error}"),
            Self::Fixture(error) => write!(formatter, "process double fixture error: {error}"),
            Self::Protocol(message) => {
                write!(formatter, "process double protocol error: {message}")
            }
        }
    }
}

impl std::error::Error for ProcessDoubleError {}

impl From<io::Error> for ProcessDoubleError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<FixtureError> for ProcessDoubleError {
    fn from(error: FixtureError) -> Self {
        Self::Fixture(error)
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum ProcessBehavior {
    Barrier,
    Hang,
    ForkAndLateWrite,
    Signal {
        number: u8,
    },
    OversizedChunked {
        chunks: Vec<String>,
    },
    OversizedStdoutAndStderr {
        stdout_chunks: Vec<String>,
        stderr_chunks: Vec<String>,
    },
}

impl ProcessBehavior {
    fn needs_barrier(&self) -> bool {
        true
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ProcessSpec {
    pub case_id: String,
    pub behavior: ProcessBehavior,
    pub output_limit: usize,
}

impl ProcessSpec {
    pub fn new(case_id: impl Into<String>, behavior: ProcessBehavior) -> Self {
        Self {
            case_id: case_id.into(),
            behavior,
            output_limit: DEFAULT_OUTPUT_LIMIT,
        }
    }

    pub fn with_output_limit(mut self, output_limit: usize) -> Self {
        self.output_limit = output_limit;
        self
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ProcessEvidence {
    pub home: String,
    pub ssh_auth_sock_absent: bool,
    pub ambient_credentials_absent: bool,
    pub barrier: String,
    pub late_write: bool,
}

impl ProcessEvidence {
    fn read(path: &PathBuf) -> Result<Self, ProcessDoubleError> {
        let text = fs::read_to_string(path)?;
        let mut values = std::collections::BTreeMap::new();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                return Err(ProcessDoubleError::Protocol(format!(
                    "evidence line has no separator: {line:?}"
                )));
            };
            values.insert(key, value);
        }
        Ok(Self {
            home: values
                .remove("home")
                .ok_or_else(|| ProcessDoubleError::Protocol("evidence has no home".to_owned()))?
                .to_owned(),
            ssh_auth_sock_absent: parse_bool(&mut values, "ssh_auth_sock_absent")?,
            ambient_credentials_absent: parse_bool(&mut values, "ambient_credentials_absent")?,
            barrier: values
                .remove("barrier")
                .unwrap_or("not-released")
                .to_owned(),
            late_write: values
                .remove("late_write")
                .map(parse_bool_value)
                .transpose()?
                .unwrap_or(false),
        })
    }
}

fn parse_bool(
    values: &mut std::collections::BTreeMap<&str, &str>,
    key: &str,
) -> Result<bool, ProcessDoubleError> {
    let value = values
        .remove(key)
        .ok_or_else(|| ProcessDoubleError::Protocol(format!("evidence has no {key}")))?;
    parse_bool_value(value)
}

fn parse_bool_value(value: &str) -> Result<bool, ProcessDoubleError> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(ProcessDoubleError::Protocol(format!(
            "invalid evidence boolean: {value:?}"
        ))),
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct CapturedOutput {
    pub bytes: Vec<u8>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ProcessStatus {
    pub code: Option<i32>,
    pub signal: Option<i32>,
}

impl ProcessStatus {
    pub fn success(&self) -> bool {
        self.code == Some(0) && self.signal.is_none()
    }

    fn from_exit(status: ExitStatus) -> Self {
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            Self {
                code: status.code(),
                signal: status.signal(),
            }
        }
        #[cfg(not(unix))]
        {
            Self {
                code: status.code(),
                signal: None,
            }
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ProcessResult {
    pub status: ProcessStatus,
    pub stdout: CapturedOutput,
    pub stderr: CapturedOutput,
    pub evidence: ProcessEvidence,
}

pub struct FakeExecutable {
    child: Option<Child>,
    stdin: ChildStdin,
    stdout: BufReader<std::process::ChildStdout>,
    stderr: BufReader<std::process::ChildStderr>,
    evidence_path: PathBuf,
    spec: ProcessSpec,
    barrier_seen: bool,
    released: bool,
}

impl FakeExecutable {
    pub fn spawn(
        fixture: &mut LifecycleFixture,
        spec: ProcessSpec,
    ) -> Result<Self, ProcessDoubleError> {
        fixture.require(Capability::UnixPermissions)?;
        let identity = fixture.identities().next("fake-executable");
        let evidence_path = fixture
            .roots()
            .artifacts()
            .join(format!("{identity}.evidence"));
        let script_path =
            fixture.publish_executable(&identity, script_for(&spec.behavior).as_bytes())?;
        fixture.track_ephemeral(&evidence_path)?;

        let mut command = Command::new("/bin/sh");
        fixture.environment().apply(&mut command);
        command
            .arg(&script_path)
            .current_dir(fixture.roots().destination())
            .env("OMNI_DOUBLE_EVIDENCE", &evidence_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn()?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| ProcessDoubleError::Protocol("child has no stdin".to_owned()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| ProcessDoubleError::Protocol("child has no stdout".to_owned()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| ProcessDoubleError::Protocol("child has no stderr".to_owned()))?;
        fixture.record(
            "double.process.spawn",
            format!("case={};behavior={:?}", spec.case_id, spec.behavior),
        );
        Ok(Self {
            child: Some(child),
            stdin,
            stdout: BufReader::new(stdout),
            stderr: BufReader::new(stderr),
            evidence_path,
            spec,
            barrier_seen: false,
            released: false,
        })
    }

    pub fn wait_for_barrier(&mut self) -> Result<(), ProcessDoubleError> {
        if !self.spec.behavior.needs_barrier() {
            return Ok(());
        }
        let mut marker = String::new();
        let read = self.stdout.read_line(&mut marker)?;
        if read == 0 || marker.trim_end_matches(['\r', '\n']) != "barrier-hit" {
            return Err(ProcessDoubleError::Protocol(format!(
                "expected barrier-hit marker, got {marker:?}"
            )));
        }
        self.barrier_seen = true;
        Ok(())
    }

    pub fn release(&mut self) -> Result<(), ProcessDoubleError> {
        if !self.barrier_seen {
            return Err(ProcessDoubleError::Protocol(
                "release called before barrier hit".to_owned(),
            ));
        }
        if self.released {
            return Err(ProcessDoubleError::Protocol(
                "release called more than once".to_owned(),
            ));
        }
        self.stdin.write_all(b"release\n")?;
        self.stdin.flush()?;
        self.released = true;
        Ok(())
    }

    pub fn try_wait(&mut self) -> Result<Option<ProcessStatus>, ProcessDoubleError> {
        let child = self
            .child
            .as_mut()
            .ok_or_else(|| ProcessDoubleError::Protocol("child was already reaped".to_owned()))?;
        Ok(child.try_wait()?.map(ProcessStatus::from_exit))
    }

    pub fn wait(&mut self) -> Result<ProcessResult, ProcessDoubleError> {
        if self.spec.behavior.needs_barrier() && !self.released {
            return Err(ProcessDoubleError::Protocol(
                "wait called before deterministic release".to_owned(),
            ));
        }
        let stdout = read_limited(&mut self.stdout, self.spec.output_limit)?;
        let stderr = read_limited(&mut self.stderr, self.spec.output_limit)?;
        let status = self
            .child
            .take()
            .ok_or_else(|| ProcessDoubleError::Protocol("child was already reaped".to_owned()))?
            .wait()?;
        let evidence = ProcessEvidence::read(&self.evidence_path)?;
        Ok(ProcessResult {
            status: ProcessStatus::from_exit(status),
            stdout,
            stderr,
            evidence,
        })
    }
}

impl Drop for FakeExecutable {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn read_limited<R: Read>(reader: &mut R, limit: usize) -> io::Result<CapturedOutput> {
    let mut bytes = Vec::new();
    let mut truncated = false;
    let mut buffer = [0_u8; 8192];
    loop {
        let count = reader.read(&mut buffer)?;
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
    Ok(CapturedOutput { bytes, truncated })
}

fn script_for(behavior: &ProcessBehavior) -> String {
    let mut script = "#!/bin/sh\nset -eu\n".to_owned()
        + "e=\"$OMNI_DOUBLE_EVIDENCE\"\n"
        + "printf 'home=%s\\n' \"$HOME\" > \"$e\"\n"
        + "if [ -z \"${SSH_AUTH_SOCK-}\" ]; then printf 'ssh_auth_sock_absent=true\\n' >> \"$e\"; else printf 'ssh_auth_sock_absent=false\\n' >> \"$e\"; fi\n"
        + "if [ -z \"${AWS_ACCESS_KEY_ID-}\" ] && [ -z \"${AWS_SECRET_ACCESS_KEY-}\" ] && [ -z \"${GITHUB_TOKEN-}\" ]; then printf 'ambient_credentials_absent=true\\n' >> \"$e\"; else printf 'ambient_credentials_absent=false\\n' >> \"$e\"; fi\n"
        + "printf 'barrier=waiting\\n' >> \"$e\"\n"
        + "printf 'late_write=false\\n' >> \"$e\"\n"
        + "printf 'barrier-hit\\n'\n"
        + "IFS= read -r release\n"
        + "[ \"$release\" = release ]\n"
        + "printf 'barrier=released\\n' >> \"$e\"\n";
    match behavior {
        ProcessBehavior::Barrier | ProcessBehavior::Hang => {
            script.push_str("exit 0\n");
        }
        ProcessBehavior::ForkAndLateWrite => {
            script.push_str(
                &("( printf 'late_write=true\\n' >> \"$e\" ) &\n".to_owned() + "wait\nexit 0\n"),
            );
        }
        ProcessBehavior::Signal { number } => {
            let signal = match number {
                1 => "HUP",
                2 => "INT",
                9 => "KILL",
                13 => "PIPE",
                15 => "TERM",
                _ => "TERM",
            };
            script.push_str(&format!("kill -{signal} \"$$\"\n"));
        }
        ProcessBehavior::OversizedChunked { chunks } => {
            for chunk in chunks {
                script.push_str("printf '%s' ");
                script.push_str(&shell_quote(chunk));
                script.push('\n');
            }
            script.push_str("exit 0\n");
        }
        ProcessBehavior::OversizedStdoutAndStderr {
            stdout_chunks,
            stderr_chunks,
        } => {
            for chunk in stdout_chunks {
                script.push_str("printf '%s' ");
                script.push_str(&shell_quote(chunk));
                script.push('\n');
            }
            for chunk in stderr_chunks {
                script.push_str("printf '%s' ");
                script.push_str(&shell_quote(chunk));
                script.push_str(" >&2\n");
            }
            script.push_str("exit 0\n");
        }
    }
    script
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        io::{self, Read},
        process::{Command, Stdio},
    };

    fn protocol_message(error: ProcessDoubleError) -> String {
        match error {
            ProcessDoubleError::Protocol(message) => message,
            other => panic!("expected protocol error, got {other:?}"),
        }
    }

    #[test]
    fn evidence_accepts_defaults_and_preserves_values() {
        let file = tempfile::NamedTempFile::new().expect("evidence fixture should open");
        fs::write(
            file.path(),
            b"home=/fixture/home\nssh_auth_sock_absent=true\nambient_credentials_absent=false\n",
        )
        .expect("evidence fixture should write");

        let evidence = ProcessEvidence::read(&file.path().to_path_buf())
            .expect("minimal evidence should parse");
        assert_eq!(evidence.home, "/fixture/home");
        assert!(evidence.ssh_auth_sock_absent);
        assert!(!evidence.ambient_credentials_absent);
        assert_eq!(evidence.barrier, "not-released");
        assert!(!evidence.late_write);
    }

    #[test]
    fn evidence_rejects_malformed_missing_and_invalid_fields() {
        let cases = [
            (
                b"home=/fixture/home\nmalformed\nssh_auth_sock_absent=true\nambient_credentials_absent=true\n"
                    .as_slice(),
                "evidence line has no separator: \"malformed\"",
            ),
            (
                b"ssh_auth_sock_absent=true\nambient_credentials_absent=true\n".as_slice(),
                "evidence has no home",
            ),
            (
                b"home=/fixture/home\nambient_credentials_absent=true\n".as_slice(),
                "evidence has no ssh_auth_sock_absent",
            ),
            (
                b"home=/fixture/home\nssh_auth_sock_absent=true\n".as_slice(),
                "evidence has no ambient_credentials_absent",
            ),
            (
                b"home=/fixture/home\nssh_auth_sock_absent=TRUE\nambient_credentials_absent=true\n"
                    .as_slice(),
                "invalid evidence boolean: \"TRUE\"",
            ),
            (
                b"home=/fixture/home\nssh_auth_sock_absent=true\nambient_credentials_absent=true\nlate_write=yes\n"
                    .as_slice(),
                "invalid evidence boolean: \"yes\"",
            ),
        ];

        for (contents, expected) in cases {
            let file = tempfile::NamedTempFile::new().expect("evidence fixture should open");
            fs::write(file.path(), contents).expect("evidence fixture should write");
            let error = ProcessEvidence::read(&file.path().to_path_buf())
                .expect_err("invalid evidence should fail closed");
            assert_eq!(protocol_message(error), expected);
        }
    }

    #[test]
    fn evidence_rejects_non_utf8_and_preserves_equals_in_values() {
        let file = tempfile::NamedTempFile::new().expect("evidence fixture should open");
        fs::write(
            file.path(),
            b"home=/fixture/home?a=b\nssh_auth_sock_absent=false\nambient_credentials_absent=true\nbarrier=released\nlate_write=true\n",
        )
        .expect("evidence fixture should write");
        let evidence = ProcessEvidence::read(&file.path().to_path_buf())
            .expect("values after the first separator should be preserved");
        assert_eq!(evidence.home, "/fixture/home?a=b");
        assert_eq!(evidence.barrier, "released");
        assert!(evidence.late_write);

        fs::write(file.path(), [0xff, 0xfe]).expect("invalid UTF-8 fixture should write");
        let error = ProcessEvidence::read(&file.path().to_path_buf())
            .expect_err("invalid UTF-8 evidence should fail");
        assert!(matches!(error, ProcessDoubleError::Io(_)));
    }

    #[test]
    fn errors_format_and_status_success_is_exact() {
        let io_error: ProcessDoubleError =
            io::Error::new(io::ErrorKind::BrokenPipe, "closed").into();
        assert_eq!(io_error.to_string(), "process double I/O error: closed");

        let fixture_error: ProcessDoubleError =
            FixtureError::Invariant("broken rule".to_owned()).into();
        assert_eq!(
            fixture_error.to_string(),
            "process double fixture error: fixture invariant failed: broken rule"
        );

        let protocol = ProcessDoubleError::Protocol("bad marker".to_owned());
        assert_eq!(
            protocol.to_string(),
            "process double protocol error: bad marker"
        );

        assert!(
            (ProcessStatus {
                code: Some(0),
                signal: None,
            })
            .success()
        );
        for status in [
            ProcessStatus {
                code: Some(1),
                signal: None,
            },
            ProcessStatus {
                code: None,
                signal: Some(9),
            },
            ProcessStatus {
                code: Some(0),
                signal: Some(9),
            },
        ] {
            assert!(!status.success(), "non-zero or signalled status must fail");
        }
    }

    #[cfg(unix)]
    #[test]
    fn exit_status_maps_code_and_signal_without_guessing() {
        use std::os::unix::process::ExitStatusExt;

        assert_eq!(
            ProcessStatus::from_exit(ExitStatus::from_raw(0)),
            ProcessStatus {
                code: Some(0),
                signal: None,
            }
        );
        assert_eq!(
            ProcessStatus::from_exit(ExitStatus::from_raw(9)),
            ProcessStatus {
                code: None,
                signal: Some(9),
            }
        );
    }

    struct ChunkReader {
        chunks: Vec<Vec<u8>>,
        next: usize,
    }

    impl Read for ChunkReader {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let Some(chunk) = self.chunks.get(self.next) else {
                return Ok(0);
            };
            self.next += 1;
            buffer[..chunk.len()].copy_from_slice(chunk);
            Ok(chunk.len())
        }
    }

    struct ErrorReader;

    impl Read for ErrorReader {
        fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "read failed"))
        }
    }

    #[test]
    fn read_limited_handles_multiple_reads_zero_limit_and_read_errors() {
        let mut reader = ChunkReader {
            chunks: vec![b"abc".to_vec(), b"def".to_vec(), b"gh".to_vec()],
            next: 0,
        };
        let captured = read_limited(&mut reader, 5).expect("chunked output should be captured");
        assert_eq!(captured.bytes, b"abcde");
        assert!(captured.truncated);

        let mut empty = ChunkReader {
            chunks: vec![],
            next: 0,
        };
        assert_eq!(
            read_limited(&mut empty, 5).expect("empty output should be valid"),
            CapturedOutput {
                bytes: vec![],
                truncated: false,
            }
        );

        let mut zero = ChunkReader {
            chunks: vec![b"x".to_vec()],
            next: 0,
        };
        assert_eq!(
            read_limited(&mut zero, 0).expect("zero limit should still drain output"),
            CapturedOutput {
                bytes: vec![],
                truncated: true,
            }
        );

        let mut failed = ErrorReader;
        let error = read_limited(&mut failed, 10).expect_err("reader errors must propagate");
        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    }

    fn raw_process(script: &str) -> FakeExecutable {
        let mut child = Command::new("/bin/sh")
            .args(["-c", script])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("raw process should spawn on Unix");
        let stdin = child.stdin.take().expect("raw child stdin");
        let stdout = child.stdout.take().expect("raw child stdout");
        let stderr = child.stderr.take().expect("raw child stderr");
        FakeExecutable {
            child: Some(child),
            stdin,
            stdout: BufReader::new(stdout),
            stderr: BufReader::new(stderr),
            evidence_path: std::env::temp_dir().join("omnirepo-process-double-test.evidence"),
            spec: ProcessSpec::new("raw", ProcessBehavior::Barrier),
            barrier_seen: false,
            released: false,
        }
    }

    #[cfg(unix)]
    #[test]
    fn barrier_rejects_wrong_marker_and_eof() {
        let mut wrong = raw_process("printf 'wrong-marker\\n'");
        assert_eq!(
            protocol_message(
                wrong
                    .wait_for_barrier()
                    .expect_err("wrong marker must be rejected")
            ),
            "expected barrier-hit marker, got \"wrong-marker\\n\""
        );

        let mut eof = raw_process(":");
        assert_eq!(
            protocol_message(eof.wait_for_barrier().expect_err("EOF must be rejected")),
            "expected barrier-hit marker, got \"\""
        );
    }

    #[test]
    fn script_and_quote_helpers_cover_all_behaviors() {
        assert!(script_for(&ProcessBehavior::Barrier).contains("exit 0"));
        assert!(script_for(&ProcessBehavior::Hang).contains("exit 0"));
        assert!(script_for(&ProcessBehavior::ForkAndLateWrite).contains("late_write=true"));
        assert!(
            script_for(&ProcessBehavior::OversizedChunked {
                chunks: vec!["chunk".to_owned()],
            })
            .contains("printf '%s' 'chunk'")
        );
        assert!(
            script_for(&ProcessBehavior::OversizedStdoutAndStderr {
                stdout_chunks: vec!["out".to_owned()],
                stderr_chunks: vec!["err".to_owned()],
            })
            .contains("printf '%s' 'err' >&2")
        );
        for (number, signal) in [
            (1, "HUP"),
            (2, "INT"),
            (9, "KILL"),
            (13, "PIPE"),
            (15, "TERM"),
            (42, "TERM"),
        ] {
            assert!(
                script_for(&ProcessBehavior::Signal { number })
                    .contains(&format!("kill -{signal} \"$$\"")),
                "signal {number} should map to {signal}"
            );
        }
        assert_eq!(shell_quote("it's ready"), "'it'\"'\"'s ready'");
    }
}
