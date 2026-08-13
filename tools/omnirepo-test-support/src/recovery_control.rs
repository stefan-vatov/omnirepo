#![allow(dead_code)]

// Shared hermetic recovery control; owned by the private test-support crate.

use std::{
    collections::BTreeMap,
    fs,
    io::{self, BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, ExitStatus, Stdio},
    thread::{self, JoinHandle},
};

use super::lifecycle_fixture::{
    BarrierController, Capability, DeterministicBarrier, FixtureError, LifecycleFixture, RootKind,
};

pub const RECOVERY_CONTRACT_VERSION: &str = "lifecycle-recovery/v1";

#[derive(Debug)]
pub enum RecoveryError {
    Io(io::Error),
    Fixture(FixtureError),
    Protocol(String),
    Thread(String),
    Writer(WriterError),
}

impl std::fmt::Display for RecoveryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "recovery control I/O error: {error}"),
            Self::Fixture(error) => write!(formatter, "recovery control fixture error: {error}"),
            Self::Protocol(message) => {
                write!(formatter, "recovery control protocol error: {message}")
            }
            Self::Thread(message) => write!(formatter, "recovery control thread error: {message}"),
            Self::Writer(error) => write!(formatter, "recovery control writer error: {error}"),
        }
    }
}

impl std::error::Error for RecoveryError {}

impl From<io::Error> for RecoveryError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<FixtureError> for RecoveryError {
    fn from(error: FixtureError) -> Self {
        Self::Fixture(error)
    }
}

impl From<WriterError> for RecoveryError {
    fn from(error: WriterError) -> Self {
        Self::Writer(error)
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum CrashDisposition {
    Exit(i32),
    Signal(u8),
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct CrashSpec {
    run_id: String,
    boundary: String,
    fields: BTreeMap<String, String>,
    disposition: CrashDisposition,
}

impl CrashSpec {
    pub fn at(boundary: impl Into<String>) -> Self {
        Self {
            run_id: "run-recovery".to_owned(),
            boundary: boundary.into(),
            fields: BTreeMap::new(),
            disposition: CrashDisposition::Exit(137),
        }
    }

    pub fn run_id(mut self, run_id: impl Into<String>) -> Self {
        self.run_id = run_id.into();
        self
    }

    pub fn with_state(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.fields.insert(key.into(), value.into());
        self
    }

    pub fn disposition(mut self, disposition: CrashDisposition) -> Self {
        self.disposition = disposition;
        self
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct RecoveryStatus {
    pub code: Option<i32>,
    pub signal: Option<i32>,
}

impl RecoveryStatus {
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
pub struct CrashEvidence {
    pub run_id: String,
    pub boundary: String,
    pub status: RecoveryStatus,
    pub journal_path: PathBuf,
}

pub struct CrashableParent {
    child: Option<Child>,
    stdout: BufReader<std::process::ChildStdout>,
    run_id: String,
    boundary: String,
    journal_path: PathBuf,
}

impl CrashableParent {
    pub fn spawn(fixture: &mut LifecycleFixture, spec: CrashSpec) -> Result<Self, RecoveryError> {
        fixture.require(Capability::UnixPermissions)?;
        validate_token("run_id", &spec.run_id)?;
        validate_token("boundary", &spec.boundary)?;
        for (key, value) in &spec.fields {
            validate_token("state key", key)?;
            validate_line_value("state value", value)?;
        }
        let identity = fixture.identities().next("recovery-parent");
        let journal_path = fixture
            .roots()
            .resolve(RootKind::Runs, &format!("{}.journal", spec.run_id))?;
        let script_path = fixture.publish_executable(
            &identity,
            crash_script(&spec.boundary, &spec.fields, spec.disposition).as_bytes(),
        )?;
        fixture.track_ephemeral(&journal_path)?;

        let mut command = Command::new("/bin/sh");
        fixture.environment().apply(&mut command);
        command
            .arg(&script_path)
            .current_dir(fixture.roots().destination())
            .env("OMNI_RECOVERY_RUN_ID", &spec.run_id)
            .env("OMNI_RECOVERY_BOUNDARY", &spec.boundary)
            .env("OMNI_RECOVERY_JOURNAL", &journal_path)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = command.spawn()?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| RecoveryError::Protocol("crash parent has no stdout".to_owned()))?;
        fixture.record(
            "recovery.parent.spawn",
            format!("run_id={};boundary={}", spec.run_id, spec.boundary),
        );
        Ok(Self {
            child: Some(child),
            stdout: BufReader::new(stdout),
            run_id: spec.run_id,
            boundary: spec.boundary,
            journal_path,
        })
    }

    pub fn wait_for_boundary(&mut self) -> Result<(), RecoveryError> {
        let mut marker = String::new();
        let read = self.stdout.read_line(&mut marker)?;
        if read == 0 || marker.trim_end_matches(['\r', '\n']) != "durable-boundary" {
            return Err(RecoveryError::Protocol(format!(
                "expected durable-boundary marker, got {marker:?}"
            )));
        }
        Ok(())
    }

    pub fn try_wait(&mut self) -> Result<Option<RecoveryStatus>, RecoveryError> {
        let child = self
            .child
            .as_mut()
            .ok_or_else(|| RecoveryError::Protocol("crash parent was already reaped".to_owned()))?;
        Ok(child.try_wait()?.map(RecoveryStatus::from_exit))
    }

    pub fn wait(&mut self) -> Result<CrashEvidence, RecoveryError> {
        let status = self
            .child
            .take()
            .ok_or_else(|| RecoveryError::Protocol("crash parent was already reaped".to_owned()))?
            .wait()?;
        Ok(CrashEvidence {
            run_id: self.run_id.clone(),
            boundary: self.boundary.clone(),
            status: RecoveryStatus::from_exit(status),
            journal_path: self.journal_path.clone(),
        })
    }
}

impl Drop for CrashableParent {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct RetainedState {
    pub path: PathBuf,
    pub run_id: String,
    pub boundary: String,
    pub fields: BTreeMap<String, String>,
}

impl RetainedState {
    pub fn restart(
        fixture: &LifecycleFixture,
        run_id: impl AsRef<str>,
    ) -> Result<Self, RecoveryError> {
        let run_id = run_id.as_ref();
        validate_token("run_id", run_id)?;
        let path = fixture
            .roots()
            .resolve(RootKind::Runs, &format!("{run_id}.journal"))?;
        let text = fs::read_to_string(&path)?;
        let values = parse_key_values(&text)?;
        let stored_run_id = values
            .get("run_id")
            .ok_or_else(|| RecoveryError::Protocol("retained state has no run_id".to_owned()))?;
        if stored_run_id != run_id {
            return Err(RecoveryError::Protocol(format!(
                "retained state run_id mismatch: expected {run_id}, got {stored_run_id}"
            )));
        }
        let boundary = values
            .get("boundary")
            .ok_or_else(|| RecoveryError::Protocol("retained state has no boundary".to_owned()))?
            .clone();
        let mut fields = values;
        fields.remove("version");
        fields.remove("run_id");
        fields.remove("boundary");
        Ok(Self {
            path,
            run_id: run_id.to_owned(),
            boundary,
            fields,
        })
    }

    pub fn field(&self, key: &str) -> Option<&str> {
        self.fields.get(key).map(String::as_str)
    }
}

fn crash_script(
    boundary: &str,
    fields: &BTreeMap<String, String>,
    disposition: CrashDisposition,
) -> String {
    let mut script = "#!/bin/sh\nset -eu\n".to_owned()
        + "journal=\"$OMNI_RECOVERY_JOURNAL\"\n"
        + "tmp=\"$journal.tmp\"\n"
        + "printf 'version=%s\\n' '"
        + RECOVERY_CONTRACT_VERSION
        + "' > \"$tmp\"\n"
        + "printf 'run_id=%s\\n' \"$OMNI_RECOVERY_RUN_ID\" >> \"$tmp\"\n"
        + "printf 'boundary=%s\\n' \"$OMNI_RECOVERY_BOUNDARY\" >> \"$tmp\"\n";
    for (key, value) in fields {
        script.push_str("printf '%s=%s\\n' ");
        script.push_str(&shell_quote(key));
        script.push(' ');
        script.push_str(&shell_quote(value));
        script.push_str(" >> \"$tmp\"\n");
    }
    script.push_str("mv \"$tmp\" \"$journal\"\n");
    script.push_str("printf 'durable-boundary\\n'\n");
    match disposition {
        CrashDisposition::Exit(code) => script.push_str(&format!("exit {code}\n")),
        CrashDisposition::Signal(signal) => {
            let name = signal_name(signal);
            script.push_str(&format!("kill -{name} \"$$\"\n"));
        }
    }
    let _ = boundary;
    script
}

fn signal_name(signal: u8) -> &'static str {
    match signal {
        1 => "HUP",
        2 => "INT",
        9 => "KILL",
        13 => "PIPE",
        15 => "TERM",
        _ => "TERM",
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd)]
pub struct ProcessIdentity {
    pub pid: u32,
    pub incarnation: u64,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PidReuseEvidence {
    pub pid: u32,
    pub previous_incarnation: u64,
    pub reused_incarnation: u64,
}

#[derive(Debug, Default)]
pub struct PidReuseControl {
    current: BTreeMap<u32, ProcessIdentity>,
    next_incarnation: BTreeMap<u32, u64>,
    evidence: Vec<PidReuseEvidence>,
}

impl PidReuseControl {
    pub fn allocate(&mut self, pid: u32) -> ProcessIdentity {
        let next = self.next_incarnation.entry(pid).or_insert(0);
        *next += 1;
        let identity = ProcessIdentity {
            pid,
            incarnation: *next,
        };
        self.current.insert(pid, identity.clone());
        identity
    }

    pub fn release(&mut self, identity: &ProcessIdentity) -> bool {
        if self.current.get(&identity.pid) == Some(identity) {
            self.current.remove(&identity.pid);
            true
        } else {
            false
        }
    }

    pub fn reuse(
        &mut self,
        pid: u32,
    ) -> Result<(ProcessIdentity, PidReuseEvidence), RecoveryError> {
        if self.current.contains_key(&pid) {
            return Err(RecoveryError::Protocol(format!(
                "PID {pid} is still active"
            )));
        }
        let previous_incarnation = self.next_incarnation.get(&pid).copied().unwrap_or(0);
        let identity = self.allocate(pid);
        let evidence = PidReuseEvidence {
            pid,
            previous_incarnation,
            reused_incarnation: identity.incarnation,
        };
        self.evidence.push(evidence.clone());
        Ok((identity, evidence))
    }

    pub fn current(&self, pid: u32) -> Option<&ProcessIdentity> {
        self.current.get(&pid)
    }

    pub fn evidence(&self) -> &[PidReuseEvidence] {
        &self.evidence
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum LeaseJournalState {
    Active,
    Unfinalized,
    Finalized,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct LeaseRecord {
    pub lease_id: String,
    pub repository: String,
    pub owner: ProcessIdentity,
    pub journal: LeaseJournalState,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct LeaseObservation {
    pub lease_id: String,
    pub owner_matches: bool,
    pub journal_unfinalized: bool,
    pub stale_candidate: bool,
}

#[derive(Debug, Default)]
pub struct LeaseControl {
    records: BTreeMap<String, LeaseRecord>,
}

impl LeaseControl {
    pub fn acquire(
        &mut self,
        repository: impl Into<String>,
        owner: ProcessIdentity,
    ) -> Result<LeaseRecord, RecoveryError> {
        let repository = repository.into();
        validate_token("repository", &repository)?;
        let lease_id = format!("{}-{}-{}", repository, owner.pid, owner.incarnation);
        if self.records.contains_key(&lease_id) {
            return Err(RecoveryError::Protocol(format!(
                "lease already exists: {lease_id}"
            )));
        }
        let record = LeaseRecord {
            lease_id: lease_id.clone(),
            repository,
            owner,
            journal: LeaseJournalState::Active,
        };
        self.records.insert(lease_id, record.clone());
        Ok(record)
    }

    pub fn mark_unfinalized(&mut self, lease_id: &str) -> Result<(), RecoveryError> {
        let record = self
            .records
            .get_mut(lease_id)
            .ok_or_else(|| RecoveryError::Protocol(format!("unknown lease: {lease_id}")))?;
        record.journal = LeaseJournalState::Unfinalized;
        Ok(())
    }

    pub fn inspect(
        &self,
        lease_id: &str,
        observed_owner: Option<&ProcessIdentity>,
    ) -> Result<LeaseObservation, RecoveryError> {
        let record = self
            .records
            .get(lease_id)
            .ok_or_else(|| RecoveryError::Protocol(format!("unknown lease: {lease_id}")))?;
        let owner_matches = observed_owner == Some(&record.owner);
        let journal_unfinalized = record.journal == LeaseJournalState::Unfinalized;
        Ok(LeaseObservation {
            lease_id: lease_id.to_owned(),
            owner_matches,
            journal_unfinalized,
            stale_candidate: !owner_matches && journal_unfinalized,
        })
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ConcurrentRunResult {
    pub run_id: String,
    pub status: RecoveryStatus,
    pub state: BTreeMap<String, String>,
}

struct ConcurrentRunChild {
    run_id: String,
    child: Option<Child>,
    stdin: ChildStdin,
    stdout: BufReader<std::process::ChildStdout>,
    state_path: PathBuf,
}

pub struct ConcurrentRunControl {
    children: BTreeMap<String, ConcurrentRunChild>,
}

impl ConcurrentRunControl {
    pub fn launch(
        fixture: &mut LifecycleFixture,
        run_ids: impl IntoIterator<Item = String>,
    ) -> Result<Self, RecoveryError> {
        fixture.require(Capability::UnixPermissions)?;
        let mut children = BTreeMap::new();
        for run_id in run_ids {
            if let Err(error) = validate_token("run_id", &run_id) {
                reap_concurrent_children(&mut children);
                return Err(error);
            }
            if children.contains_key(&run_id) {
                reap_concurrent_children(&mut children);
                return Err(RecoveryError::Protocol(format!(
                    "duplicate concurrent run: {run_id}"
                )));
            }
            let identity = fixture.identities().next("concurrent-run");
            let state_path = match fixture
                .roots()
                .resolve(RootKind::Runs, &format!("{run_id}.concurrent"))
            {
                Ok(path) => path,
                Err(error) => {
                    reap_concurrent_children(&mut children);
                    return Err(error.into());
                }
            };
            if state_path.exists() {
                reap_concurrent_children(&mut children);
                return Err(RecoveryError::Protocol(format!(
                    "concurrent state already exists: {}",
                    state_path.display()
                )));
            }
            let script_path =
                match fixture.publish_executable(&identity, concurrent_run_script().as_bytes()) {
                    Ok(path) => path,
                    Err(error) => {
                        reap_concurrent_children(&mut children);
                        return Err(error.into());
                    }
                };
            if let Err(error) = fixture.track_ephemeral(&state_path) {
                reap_concurrent_children(&mut children);
                return Err(error.into());
            }
            let mut command = Command::new("/bin/sh");
            fixture.environment().apply(&mut command);
            command
                .arg(&script_path)
                .current_dir(fixture.roots().destination())
                .env("OMNI_RUN_ID", &run_id)
                .env("OMNI_RUN_STATE", &state_path)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null());
            let mut child = match command.spawn() {
                Ok(child) => child,
                Err(error) => {
                    reap_concurrent_children(&mut children);
                    return Err(error.into());
                }
            };
            let stdin = match child.stdin.take() {
                Some(stdin) => stdin,
                None => {
                    reap_child(&mut child);
                    reap_concurrent_children(&mut children);
                    return Err(RecoveryError::Protocol(
                        "concurrent child has no stdin".to_owned(),
                    ));
                }
            };
            let stdout = match child.stdout.take() {
                Some(stdout) => stdout,
                None => {
                    reap_child(&mut child);
                    reap_concurrent_children(&mut children);
                    return Err(RecoveryError::Protocol(
                        "concurrent child has no stdout".to_owned(),
                    ));
                }
            };
            children.insert(
                run_id.clone(),
                ConcurrentRunChild {
                    run_id,
                    child: Some(child),
                    stdin,
                    stdout: BufReader::new(stdout),
                    state_path,
                },
            );
        }
        fixture.record(
            "recovery.concurrent.spawn",
            format!(
                "runs={}",
                children.keys().cloned().collect::<Vec<_>>().join(",")
            ),
        );
        Ok(Self { children })
    }

    pub fn wait_for_ready(&mut self) -> Result<(), RecoveryError> {
        for child in self.children.values_mut() {
            let mut marker = String::new();
            let read = child.stdout.read_line(&mut marker)?;
            if read == 0 || marker.trim_end_matches(['\r', '\n']) != "run-ready" {
                return Err(RecoveryError::Protocol(format!(
                    "run {} did not reach ready barrier: {marker:?}",
                    child.run_id
                )));
            }
        }
        Ok(())
    }

    pub fn release_all(&mut self) -> Result<(), RecoveryError> {
        for child in self.children.values_mut() {
            child.stdin.write_all(b"release\n")?;
            child.stdin.flush()?;
        }
        Ok(())
    }

    pub fn join(mut self) -> Result<Vec<ConcurrentRunResult>, RecoveryError> {
        let mut results = Vec::new();
        let children = std::mem::take(&mut self.children);
        for (_, mut child) in children {
            let status = child
                .child
                .take()
                .ok_or_else(|| RecoveryError::Protocol("run was already reaped".to_owned()))?
                .wait()?;
            let text = fs::read_to_string(&child.state_path)?;
            results.push(ConcurrentRunResult {
                run_id: child.run_id,
                status: RecoveryStatus::from_exit(status),
                state: parse_key_values(&text)?,
            });
        }
        Ok(results)
    }
}

fn reap_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn reap_concurrent_children(children: &mut BTreeMap<String, ConcurrentRunChild>) {
    for child in children.values_mut() {
        if let Some(mut process) = child.child.take() {
            reap_child(&mut process);
        }
        // The launch path rejects pre-existing state files before tracking the
        // path, so removing this path only removes residue owned by the
        // failed partial launch.
        let _ = fs::remove_file(&child.state_path);
    }
}

impl Drop for ConcurrentRunControl {
    fn drop(&mut self) {
        reap_concurrent_children(&mut self.children);
    }
}

fn concurrent_run_script() -> &'static str {
    "#!/bin/sh\nset -eu\nprintf 'run-ready\\n'\nIFS= read -r release\n[ \"$release\" = release ]\nprintf 'run_id=%s\\nstatus=completed\\n' \"$OMNI_RUN_ID\" > \"$OMNI_RUN_STATE\"\nexit 0\n"
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum JournalTail {
    Complete,
    Truncated,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct JournalEvidence {
    pub path: PathBuf,
    pub bytes: Vec<u8>,
    pub records: Vec<String>,
    pub tail: JournalTail,
}

pub struct JournalControl {
    path: PathBuf,
}

impl JournalControl {
    pub fn create(
        fixture: &mut LifecycleFixture,
        run_id: impl AsRef<str>,
    ) -> Result<Self, RecoveryError> {
        let run_id = run_id.as_ref();
        validate_token("run_id", run_id)?;
        let path = fixture
            .roots()
            .resolve(RootKind::Runs, &format!("{run_id}.jsonl"))?;
        fs::write(&path, b"")?;
        fixture.track_ephemeral(&path)?;
        Ok(Self { path })
    }

    pub fn append_record(&self, record: &str) -> Result<(), RecoveryError> {
        validate_line_value("journal record", record)?;
        let mut file = fs::OpenOptions::new().append(true).open(&self.path)?;
        writeln!(file, "{record}")?;
        Ok(())
    }

    pub fn truncate_tail(&self, bytes: u64) -> Result<(), RecoveryError> {
        let metadata = fs::metadata(&self.path)?;
        let new_len = metadata.len().saturating_sub(bytes);
        let file = fs::OpenOptions::new().write(true).open(&self.path)?;
        file.set_len(new_len)?;
        Ok(())
    }

    pub fn inspect(&self) -> Result<JournalEvidence, RecoveryError> {
        let bytes = fs::read(&self.path)?;
        let tail = if bytes.is_empty() || bytes.ends_with(b"\n") {
            JournalTail::Complete
        } else {
            JournalTail::Truncated
        };
        let records = String::from_utf8_lossy(&bytes)
            .lines()
            .map(str::to_owned)
            .collect();
        Ok(JournalEvidence {
            path: self.path.clone(),
            bytes,
            records,
            tail,
        })
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum WriterFault {
    EnospcAfter(usize),
    Error(String),
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum WriterError {
    Enospc { attempted: usize, written: usize },
    Injected(String),
}

impl std::fmt::Display for WriterError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Enospc { attempted, written } => {
                write!(
                    formatter,
                    "simulated ENOSPC after {written}/{attempted} bytes"
                )
            }
            Self::Injected(message) => write!(formatter, "simulated writer failure: {message}"),
        }
    }
}

impl std::error::Error for WriterError {}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct WriteEvidence {
    pub attempted: usize,
    pub written: usize,
    pub failed: bool,
}

pub struct JournalWriterControl {
    path: PathBuf,
    fault: Option<WriterFault>,
}

impl JournalWriterControl {
    pub fn create(
        fixture: &mut LifecycleFixture,
        name: impl AsRef<str>,
        fault: Option<WriterFault>,
    ) -> Result<Self, RecoveryError> {
        let name = name.as_ref();
        validate_token("writer name", name)?;
        let path = fixture
            .roots()
            .resolve(RootKind::Runs, &format!("{name}.writer"))?;
        fs::write(&path, b"")?;
        fixture.track_ephemeral(&path)?;
        Ok(Self { path, fault })
    }

    pub fn write(&mut self, bytes: &[u8]) -> Result<WriteEvidence, WriterError> {
        let fault = self.fault.take();
        match fault {
            Some(WriterFault::EnospcAfter(limit)) => {
                let written = limit.min(bytes.len());
                fs::write(&self.path, &bytes[..written]).map_err(|error| {
                    WriterError::Injected(format!("write setup failed: {error}"))
                })?;
                Err(WriterError::Enospc {
                    attempted: bytes.len(),
                    written,
                })
            }
            Some(WriterFault::Error(message)) => Err(WriterError::Injected(message)),
            None => {
                fs::write(&self.path, bytes)
                    .map_err(|error| WriterError::Injected(format!("write failed: {error}")))?;
                Ok(WriteEvidence {
                    attempted: bytes.len(),
                    written: bytes.len(),
                    failed: false,
                })
            }
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct CleanupObservation {
    pub existed_before_cleanup: bool,
    pub removed_before_writer: bool,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct CleanupRaceEvidence {
    pub writer_started: bool,
    pub writer_completed: bool,
    pub path: PathBuf,
}

pub struct CleanupRaceControl {
    path: PathBuf,
    barrier: DeterministicBarrier,
    join: Option<JoinHandle<Result<CleanupRaceEvidence, RecoveryError>>>,
}

impl CleanupRaceControl {
    pub fn start(
        fixture: &mut LifecycleFixture,
        name: impl AsRef<str>,
    ) -> Result<Self, RecoveryError> {
        let name = name.as_ref();
        validate_token("cleanup name", name)?;
        let barriers: BarrierController = fixture.barriers();
        let barrier = barriers.arm(format!("cleanup-{name}"))?;
        let path = fixture
            .roots()
            .resolve(RootKind::Artifacts, &format!("{name}.late"))?;
        fixture.track_ephemeral(&path)?;
        let path_for_thread = path.clone();
        let barrier_for_thread = barrier.clone();
        let join = thread::Builder::new()
            .name(format!("omnirepo-cleanup-{name}"))
            .spawn(move || {
                barrier_for_thread.hit()?;
                fs::write(&path_for_thread, b"late-writer\n")?;
                Ok(CleanupRaceEvidence {
                    writer_started: true,
                    writer_completed: true,
                    path: path_for_thread,
                })
            })
            .map_err(|error| RecoveryError::Thread(error.to_string()))?;
        fixture.record(
            "recovery.cleanup-race.spawn",
            format!("name={name};barrier=cleanup-{name}"),
        );
        Ok(Self {
            path,
            barrier,
            join: Some(join),
        })
    }

    pub fn wait_for_writer(&self) -> Result<(), RecoveryError> {
        self.barrier.wait_for_hit()?;
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn cleanup_before_writer(&self) -> Result<CleanupObservation, RecoveryError> {
        let existed_before_cleanup = self.path.exists();
        let removed_before_writer = if existed_before_cleanup {
            fs::remove_file(&self.path).is_ok()
        } else {
            false
        };
        Ok(CleanupObservation {
            existed_before_cleanup,
            removed_before_writer,
        })
    }

    pub fn release_writer(&self) -> Result<(), RecoveryError> {
        self.barrier.release()?;
        Ok(())
    }

    pub fn join(mut self) -> Result<CleanupRaceEvidence, RecoveryError> {
        let join = self
            .join
            .take()
            .ok_or_else(|| RecoveryError::Protocol("cleanup writer already joined".to_owned()))?;
        join.join()
            .map_err(|_| RecoveryError::Thread("cleanup writer panicked".to_owned()))?
    }
}

impl Drop for CleanupRaceControl {
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

fn parse_key_values(text: &str) -> Result<BTreeMap<String, String>, RecoveryError> {
    let mut values = BTreeMap::new();
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            return Err(RecoveryError::Protocol(format!(
                "record has no key/value separator: {line:?}"
            )));
        };
        if key.is_empty() || values.insert(key.to_owned(), value.to_owned()).is_some() {
            return Err(RecoveryError::Protocol(format!(
                "duplicate or empty record key: {key:?}"
            )));
        }
    }
    Ok(values)
}

fn validate_token(label: &str, value: &str) -> Result<(), RecoveryError> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || value.contains('/')
        || value.contains('\\')
        || value.contains('\n')
        || value.contains('\r')
    {
        return Err(RecoveryError::Protocol(format!(
            "invalid {label}: {value:?}"
        )));
    }
    Ok(())
}

fn validate_line_value(label: &str, value: &str) -> Result<(), RecoveryError> {
    if value.contains('\n') || value.contains('\r') {
        return Err(RecoveryError::Protocol(format!(
            "invalid {label}: newline is not allowed"
        )));
    }
    Ok(())
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}
