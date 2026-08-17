#![allow(dead_code)]

// Shared hermetic lifecycle fixture; owned by the private test-support crate.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt, fs,
    io::{self, Write},
    path::{Component, Path, PathBuf},
    process::{Command, Output},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use tempfile::{Builder, TempDir};

pub const FIXTURE_CONTRACT_VERSION: &str = "lifecycle-fixtures/v1";

#[derive(Debug)]
pub enum FixtureError {
    Io(io::Error),
    InvalidPath {
        path: String,
        reason: &'static str,
    },
    EscapesRoot(PathBuf),
    Unsupported(UnsupportedCapability),
    Command {
        program: String,
        status: String,
        stderr: String,
    },
    Invariant(String),
}

impl fmt::Display for FixtureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "fixture I/O error: {error}"),
            Self::InvalidPath { path, reason } => {
                write!(f, "invalid fixture path {path:?}: {reason}")
            }
            Self::EscapesRoot(path) => {
                write!(f, "fixture path escapes its root: {}", path.display())
            }
            Self::Unsupported(capability) => {
                write!(f, "unsupported fixture capability: {capability}")
            }
            Self::Command {
                program,
                status,
                stderr,
            } => write!(f, "fixture command {program} failed ({status}): {stderr}"),
            Self::Invariant(message) => write!(f, "fixture invariant failed: {message}"),
        }
    }
}

impl std::error::Error for FixtureError {}

impl From<io::Error> for FixtureError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum CleanupMode {
    RemoveOnSuccessRetainOnFailure,
    RetainAlways,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum FixtureOutcome {
    Success,
    Failure,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct FixtureSpec {
    pub case_id: String,
    pub seed: u64,
    pub cleanup: CleanupMode,
}

impl FixtureSpec {
    pub fn new(case_id: impl Into<String>, seed: u64) -> Self {
        Self {
            case_id: case_id.into(),
            seed,
            cleanup: CleanupMode::RemoveOnSuccessRetainOnFailure,
        }
    }

    pub fn retain_always(mut self) -> Self {
        self.cleanup = CleanupMode::RetainAlways;
        self
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd)]
pub enum RootKind {
    Root,
    Home,
    MachineConfig,
    Source,
    SourceConfig,
    SourceSnapshot,
    Destination,
    Runs,
    Artifacts,
    Remote,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct RootSet {
    root: PathBuf,
    home: PathBuf,
    machine_config: PathBuf,
    source: PathBuf,
    source_config: PathBuf,
    source_snapshot: PathBuf,
    destination: PathBuf,
    runs: PathBuf,
    artifacts: PathBuf,
    remote: PathBuf,
    git_config_global: PathBuf,
    git_config_system: PathBuf,
}

impl RootSet {
    fn create(root: PathBuf) -> Result<Self, FixtureError> {
        let home = root.join("home");
        let machine_config = home.join(".omnirepo");
        let source = root.join("source");
        let source_config = source.join(".omnirepo");
        let source_snapshot = root.join("source-snapshot");
        let destination = root.join("destination");
        let runs = home.join(".omnirepo/runs");
        let artifacts = root.join("artifacts");
        let remote = root.join("remote.git");
        let git_config_global = root.join("gitconfig.global");
        let git_config_system = root.join("gitconfig.system");

        for path in [
            &home,
            &machine_config,
            &source,
            &source_config,
            &source_snapshot,
            &destination,
            &runs,
            &artifacts,
            &remote,
        ] {
            fs::create_dir_all(path)?;
        }
        fs::File::create(&git_config_global)?;
        fs::File::create(&git_config_system)?;

        Ok(Self {
            root,
            home,
            machine_config,
            source,
            source_config,
            source_snapshot,
            destination,
            runs,
            artifacts,
            remote,
            git_config_global,
            git_config_system,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn home(&self) -> &Path {
        &self.home
    }

    pub fn machine_config_root(&self) -> &Path {
        &self.machine_config
    }

    pub fn machine_config(&self) -> PathBuf {
        self.machine_config.join("config.yaml")
    }

    pub fn source(&self) -> &Path {
        &self.source
    }

    pub fn source_config_root(&self) -> &Path {
        &self.source_config
    }

    pub fn source_config(&self) -> PathBuf {
        self.source_config.join("source.yaml")
    }

    pub fn source_snapshot(&self) -> &Path {
        &self.source_snapshot
    }

    pub fn destination(&self) -> &Path {
        &self.destination
    }

    pub fn destination_config(&self) -> PathBuf {
        self.destination.join(".omnirepo.yaml")
    }

    pub fn runs(&self) -> &Path {
        &self.runs
    }

    pub fn artifacts(&self) -> &Path {
        &self.artifacts
    }

    pub fn remote(&self) -> &Path {
        &self.remote
    }

    fn git_config_global(&self) -> &Path {
        &self.git_config_global
    }

    fn git_config_system(&self) -> &Path {
        &self.git_config_system
    }

    pub fn resolve(&self, root: RootKind, relative: &str) -> Result<PathBuf, FixtureError> {
        let base = match root {
            RootKind::Root => self.root(),
            RootKind::Home => self.home(),
            RootKind::MachineConfig => self.machine_config_root(),
            RootKind::Source => self.source(),
            RootKind::SourceConfig => self.source_config_root(),
            RootKind::SourceSnapshot => self.source_snapshot(),
            RootKind::Destination => self.destination(),
            RootKind::Runs => self.runs(),
            RootKind::Artifacts => self.artifacts(),
            RootKind::Remote => self.remote(),
        };
        let relative_path = Path::new(relative);
        if relative_path.is_absolute() {
            return Err(FixtureError::InvalidPath {
                path: relative.to_owned(),
                reason: "absolute paths are not accepted",
            });
        }
        if relative_path
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::Prefix(_)))
        {
            return Err(FixtureError::InvalidPath {
                path: relative.to_owned(),
                reason: "parent traversal is not accepted",
            });
        }
        let resolved = base.join(relative_path);
        self.confine(&resolved)
    }

    pub fn contains(&self, path: &Path) -> bool {
        self.confine(path).is_ok()
    }

    /// Validate a caller-provided path before it is used for filesystem work.
    ///
    /// The lexical check rejects absolute paths outside the fixture and parent
    /// traversal. The canonical check walks up to the nearest existing path,
    /// so a symlink in a missing path's parent cannot redirect a later create
    /// operation outside the fixture root.
    pub fn confine(&self, path: &Path) -> Result<PathBuf, FixtureError> {
        if !path.is_absolute() || !path.starts_with(&self.root) {
            return Err(FixtureError::EscapesRoot(path.to_path_buf()));
        }
        let relative = path
            .strip_prefix(&self.root)
            .map_err(|_| FixtureError::EscapesRoot(path.to_path_buf()))?;
        if relative
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::Prefix(_)))
        {
            return Err(FixtureError::InvalidPath {
                path: path.display().to_string(),
                reason: "parent traversal is not accepted",
            });
        }

        let canonical_root = fs::canonicalize(&self.root)?;
        let mut existing = path.to_path_buf();
        loop {
            match fs::symlink_metadata(&existing) {
                Ok(metadata) => {
                    let canonical = fs::canonicalize(&existing).map_err(|error| {
                        if metadata.file_type().is_symlink() {
                            FixtureError::EscapesRoot(existing.clone())
                        } else {
                            FixtureError::Io(error)
                        }
                    })?;
                    if !canonical.starts_with(&canonical_root) {
                        return Err(FixtureError::EscapesRoot(canonical));
                    }
                    return Ok(path.to_path_buf());
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    if !existing.pop() {
                        return Err(FixtureError::Io(error));
                    }
                }
                Err(error) => return Err(FixtureError::Io(error)),
            }
        }
    }

    pub fn identity(&self, path: &Path) -> Result<FileIdentity, FixtureError> {
        let path = self.confine(path)?;
        let canonical = fs::canonicalize(&path)?;
        if !canonical.starts_with(fs::canonicalize(&self.root)?) {
            return Err(FixtureError::EscapesRoot(canonical));
        }
        let metadata = fs::metadata(&path)?;
        Ok(FileIdentity::from_metadata(canonical, &metadata))
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct FileIdentity {
    pub canonical: PathBuf,
    pub device: Option<u64>,
    pub inode: Option<u64>,
}

impl FileIdentity {
    fn from_metadata(canonical: PathBuf, metadata: &fs::Metadata) -> Self {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            Self {
                canonical,
                device: Some(metadata.dev()),
                inode: Some(metadata.ino()),
            }
        }
        #[cfg(not(unix))]
        {
            let _ = metadata;
            Self {
                canonical,
                device: None,
                inode: None,
            }
        }
    }

    pub fn same_object(&self, other: &Self) -> bool {
        match ((self.device, self.inode), (other.device, other.inode)) {
            ((Some(device), Some(inode)), (Some(other_device), Some(other_inode))) => {
                device == other_device && inode == other_inode
            }
            _ => self.canonical == other.canonical,
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd)]
pub enum Capability {
    Git,
    Symlink,
    HardLink,
    Fifo,
    UnixPermissions,
}

impl fmt::Display for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Git => "git",
            Self::Symlink => "symlink",
            Self::HardLink => "hard-link",
            Self::Fifo => "fifo",
            Self::UnixPermissions => "unix-permissions",
        };
        f.write_str(name)
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct UnsupportedCapability {
    pub capability: Capability,
    pub reason: String,
}

impl fmt::Display for UnsupportedCapability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.capability, self.reason)
    }
}

#[derive(Debug, Clone, Default)]
pub struct CapabilityProbe;

impl CapabilityProbe {
    pub fn status(&self, capability: Capability) -> Result<(), UnsupportedCapability> {
        let supported = match capability {
            Capability::Git => git_available(),
            Capability::Symlink | Capability::HardLink => cfg!(unix),
            Capability::Fifo => cfg!(unix) && mkfifo_available(),
            Capability::UnixPermissions => cfg!(unix),
        };
        if supported {
            Ok(())
        } else {
            Err(UnsupportedCapability {
                capability,
                reason: "capability is not available on this platform or PATH".to_owned(),
            })
        }
    }
}

#[derive(Debug, Clone)]
pub struct FixtureEnvironment {
    vars: BTreeMap<String, String>,
}

impl FixtureEnvironment {
    fn new(roots: &RootSet) -> Result<Self, FixtureError> {
        let mut vars = BTreeMap::new();
        let home = roots.home().display().to_string();
        let config_home = roots.home().join(".config");
        fs::create_dir_all(&config_home)?;
        vars.insert("HOME".to_owned(), home.clone());
        vars.insert("USERPROFILE".to_owned(), home);
        vars.insert(
            "XDG_CONFIG_HOME".to_owned(),
            config_home.display().to_string(),
        );
        vars.insert(
            "GIT_CONFIG_GLOBAL".to_owned(),
            roots.git_config_global().display().to_string(),
        );
        vars.insert(
            "GIT_CONFIG_SYSTEM".to_owned(),
            roots.git_config_system().display().to_string(),
        );
        vars.insert("GIT_CONFIG_NOSYSTEM".to_owned(), "1".to_owned());
        vars.insert("GIT_TERMINAL_PROMPT".to_owned(), "0".to_owned());
        vars.insert("GIT_AUTHOR_NAME".to_owned(), "Omnirepo Fixture".to_owned());
        vars.insert(
            "GIT_AUTHOR_EMAIL".to_owned(),
            "fixture@omnirepo.invalid".to_owned(),
        );
        vars.insert(
            "GIT_COMMITTER_NAME".to_owned(),
            "Omnirepo Fixture".to_owned(),
        );
        vars.insert(
            "GIT_COMMITTER_EMAIL".to_owned(),
            "fixture@omnirepo.invalid".to_owned(),
        );
        let path =
            std::env::var("OMNIREPO_TEST_PATH").unwrap_or_else(|_| safe_tool_path().to_owned());
        vars.insert("PATH".to_owned(), path);
        if let Ok(profile) = std::env::var("LLVM_PROFILE_FILE") {
            vars.insert("LLVM_PROFILE_FILE".to_owned(), profile);
        }
        Ok(Self { vars })
    }

    pub fn vars(&self) -> &BTreeMap<String, String> {
        &self.vars
    }

    pub fn value(&self, key: &str) -> Option<&str> {
        self.vars.get(key).map(String::as_str)
    }

    pub fn apply(&self, command: &mut Command) {
        command.env_clear();
        command.envs(self.vars.iter());
    }
}

#[derive(Debug, Clone)]
pub struct DeterministicClock {
    ticks: Arc<AtomicU64>,
}

impl Default for DeterministicClock {
    fn default() -> Self {
        Self::new()
    }
}

impl DeterministicClock {
    pub fn new() -> Self {
        Self {
            ticks: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn now(&self) -> u64 {
        self.ticks.load(Ordering::SeqCst)
    }

    pub fn advance(&self, duration: Duration) -> u64 {
        let millis = u64::try_from(duration.as_millis()).expect("fixture duration fits u64");
        self.ticks.fetch_add(millis, Ordering::SeqCst) + millis
    }

    pub fn set(&self, ticks: u64) {
        self.ticks.store(ticks, Ordering::SeqCst);
    }
}

#[derive(Debug, Clone)]
pub struct DeterministicIdentity {
    seed: u64,
    counters: Arc<Mutex<BTreeMap<String, u64>>>,
    collisions: Arc<Mutex<BTreeMap<String, String>>>,
}

impl DeterministicIdentity {
    pub fn new(seed: u64) -> Self {
        Self {
            seed,
            counters: Arc::new(Mutex::new(BTreeMap::new())),
            collisions: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    pub fn next(&self, namespace: &str) -> String {
        let mut counters = self
            .counters
            .lock()
            .expect("identity mutex is not poisoned");
        let occurrence = counters.entry(namespace.to_owned()).or_insert(0);
        *occurrence += 1;
        let occurrence = *occurrence;
        if let Some(value) = self
            .collisions
            .lock()
            .expect("collision mutex is not poisoned")
            .get(namespace)
            .cloned()
        {
            return value;
        }
        let first = stable_hash(self.seed, namespace, occurrence);
        let second = stable_hash(self.seed ^ u64::MAX, namespace, occurrence);
        format!("{namespace}-{first:016x}{second:016x}")
    }

    pub fn run_id(&self) -> String {
        self.next("run")
    }

    pub fn force_collision(&self, namespace: &str, value: impl Into<String>) {
        self.collisions
            .lock()
            .expect("collision mutex is not poisoned")
            .insert(namespace.to_owned(), value.into());
    }
}

fn stable_hash(seed: u64, namespace: &str, occurrence: u64) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64 ^ seed;
    for byte in namespace
        .as_bytes()
        .iter()
        .copied()
        .chain(occurrence.to_le_bytes())
    {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct FixtureEvent {
    pub sequence: u64,
    pub logical_time: u64,
    pub kind: String,
    pub detail: String,
}

#[derive(Debug, Clone, Default)]
pub struct EventLog {
    state: Arc<Mutex<Vec<FixtureEvent>>>,
}

impl EventLog {
    pub fn record(
        &self,
        clock: &DeterministicClock,
        kind: impl Into<String>,
        detail: impl Into<String>,
    ) {
        let mut events = self.state.lock().expect("event log mutex is not poisoned");
        let sequence = events.len() as u64 + 1;
        events.push(FixtureEvent {
            sequence,
            logical_time: clock.now(),
            kind: kind.into(),
            detail: detail.into(),
        });
    }

    pub fn snapshot(&self) -> Vec<FixtureEvent> {
        self.state
            .lock()
            .expect("event log mutex is not poisoned")
            .clone()
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum BarrierState {
    Armed,
    Hit,
    Released,
    Aborted,
}

#[derive(Debug, Clone)]
pub struct DeterministicBarrier {
    state: Arc<(Mutex<BarrierState>, Condvar)>,
}

impl DeterministicBarrier {
    pub fn new() -> Self {
        Self {
            state: Arc::new((Mutex::new(BarrierState::Armed), Condvar::new())),
        }
    }

    pub fn hit(&self) -> Result<(), FixtureError> {
        let (lock, wake) = &*self.state;
        let mut state = lock.lock().expect("barrier mutex is not poisoned");
        if *state != BarrierState::Armed {
            return Err(FixtureError::Invariant(
                "barrier was hit more than once".to_owned(),
            ));
        }
        *state = BarrierState::Hit;
        wake.notify_all();
        while *state == BarrierState::Hit {
            state = wake.wait(state).expect("barrier mutex is not poisoned");
        }
        if *state == BarrierState::Aborted {
            Err(FixtureError::Invariant("barrier was aborted".to_owned()))
        } else {
            Ok(())
        }
    }

    pub fn wait_for_hit(&self) -> Result<(), FixtureError> {
        let (lock, wake) = &*self.state;
        let mut state = lock.lock().expect("barrier mutex is not poisoned");
        while *state == BarrierState::Armed {
            state = wake.wait(state).expect("barrier mutex is not poisoned");
        }
        if *state == BarrierState::Aborted {
            Err(FixtureError::Invariant("barrier was aborted".to_owned()))
        } else {
            Ok(())
        }
    }

    pub fn release(&self) -> Result<(), FixtureError> {
        let (lock, wake) = &*self.state;
        let mut state = lock.lock().expect("barrier mutex is not poisoned");
        if *state != BarrierState::Hit {
            return Err(FixtureError::Invariant("barrier was not hit".to_owned()));
        }
        *state = BarrierState::Released;
        wake.notify_all();
        Ok(())
    }

    pub fn abort(&self) {
        let (lock, wake) = &*self.state;
        *lock.lock().expect("barrier mutex is not poisoned") = BarrierState::Aborted;
        wake.notify_all();
    }
}

impl Default for DeterministicBarrier {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Default)]
pub struct BarrierController {
    barriers: Arc<Mutex<BTreeMap<String, DeterministicBarrier>>>,
}

impl BarrierController {
    pub fn arm(&self, id: impl Into<String>) -> Result<DeterministicBarrier, FixtureError> {
        let id = id.into();
        let mut barriers = self
            .barriers
            .lock()
            .expect("barrier registry mutex is not poisoned");
        if barriers.contains_key(&id) {
            return Err(FixtureError::Invariant(format!(
                "barrier already armed: {id}"
            )));
        }
        let barrier = DeterministicBarrier::new();
        barriers.insert(id, barrier.clone());
        Ok(barrier)
    }

    pub fn wait_for_hit(&self, id: &str) -> Result<(), FixtureError> {
        self.get(id)?.wait_for_hit()
    }

    pub fn release(&self, id: &str) -> Result<(), FixtureError> {
        self.get(id)?.release()
    }

    pub fn abort(&self, id: &str) -> Result<(), FixtureError> {
        self.get(id)?.abort();
        Ok(())
    }

    fn get(&self, id: &str) -> Result<DeterministicBarrier, FixtureError> {
        self.barriers
            .lock()
            .expect("barrier registry mutex is not poisoned")
            .get(id)
            .cloned()
            .ok_or_else(|| FixtureError::Invariant(format!("barrier is not armed: {id}")))
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd)]
pub struct FaultPoint {
    pub point_id: String,
    pub phase: String,
    pub actor: String,
    pub occurrence: u32,
}

impl FaultPoint {
    pub fn new(
        point_id: impl Into<String>,
        phase: impl Into<String>,
        actor: impl Into<String>,
        occurrence: u32,
    ) -> Self {
        Self {
            point_id: point_id.into(),
            phase: phase.into(),
            actor: actor.into(),
            occurrence,
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum FaultAction {
    ReturnError(String),
    PartialWrite {
        requested_bytes: u64,
        delivered_bytes: u64,
    },
    SyncFailure(String),
    Collision(String),
    AcceptThenDisconnect,
    ReplaceIdentity,
    LeaveDescendant,
    InterveneWorktree,
    Cancel,
    Crash,
}

#[derive(Debug, Clone, Default)]
pub struct FaultController {
    rules: Arc<Mutex<BTreeMap<FaultPoint, FaultAction>>>,
    hits: Arc<Mutex<BTreeSet<FaultPoint>>>,
}

impl FaultController {
    pub fn arm(&self, point: FaultPoint, action: FaultAction) -> Result<(), FixtureError> {
        let mut rules = self
            .rules
            .lock()
            .expect("fault registry mutex is not poisoned");
        if rules.insert(point.clone(), action).is_some() {
            return Err(FixtureError::Invariant(format!(
                "fault point already armed: {}:{}:{}:{}",
                point.point_id, point.phase, point.actor, point.occurrence
            )));
        }
        Ok(())
    }

    pub fn hit(&self, point: &FaultPoint) -> Option<FaultAction> {
        let action = self
            .rules
            .lock()
            .expect("fault registry mutex is not poisoned")
            .get(point)
            .cloned();
        if action.is_some() {
            self.hits
                .lock()
                .expect("fault hit mutex is not poisoned")
                .insert(point.clone());
        }
        action
    }

    pub fn assert_consumed(&self) -> Result<(), FixtureError> {
        let rules = self
            .rules
            .lock()
            .expect("fault registry mutex is not poisoned");
        let hits = self.hits.lock().expect("fault hit mutex is not poisoned");
        let unhit = rules.keys().find(|point| !hits.contains(*point));
        match unhit {
            Some(point) => Err(FixtureError::Invariant(format!(
                "fault point was not reached: {}:{}:{}:{}",
                point.point_id, point.phase, point.actor, point.occurrence
            ))),
            None => Ok(()),
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum AliasKind {
    Symlink,
    HardLink,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum DirtyGitState {
    Clean,
    Untracked,
    Modified,
    Staged,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct GitSnapshot {
    pub root: PathBuf,
    pub porcelain: String,
    pub state: DirtyGitState,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct CleanupReport {
    pub root: PathBuf,
    pub removed: bool,
    pub retained: bool,
    pub expected_residue: Vec<PathBuf>,
    pub leaks: Vec<PathBuf>,
}

pub struct LifecycleFixture {
    temp: Option<TempDir>,
    spec: FixtureSpec,
    roots: RootSet,
    environment: FixtureEnvironment,
    clock: DeterministicClock,
    identities: DeterministicIdentity,
    log: EventLog,
    capabilities: CapabilityProbe,
    barriers: BarrierController,
    faults: FaultController,
    ephemeral: BTreeSet<PathBuf>,
    expected_residue: BTreeSet<PathBuf>,
}

impl LifecycleFixture {
    pub fn new(spec: FixtureSpec) -> Result<Self, FixtureError> {
        Self::create(spec)
    }

    pub fn create(spec: FixtureSpec) -> Result<Self, FixtureError> {
        // The system temp dir on macOS lives under /var/folders, and /var
        // is a symlink there; the runner's evidence store validates its
        // root as symlink-free, so fixtures live under the crate's target.
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
        std::fs::create_dir_all(&base).map_err(FixtureError::Io)?;
        let temp = Builder::new()
            .prefix("omnirepo-fixture-")
            .tempdir_in(&base)?;
        let roots = RootSet::create(temp.path().to_path_buf())?;
        let environment = FixtureEnvironment::new(&roots)?;
        let clock = DeterministicClock::new();
        let identities = DeterministicIdentity::new(spec.seed);
        let log = EventLog::default();
        log.record(
            &clock,
            "fixture.create",
            format!(
                "version={FIXTURE_CONTRACT_VERSION};fixture_id=fixture-{:016x};case={};seed={}",
                stable_hash(spec.seed, &spec.case_id, 0),
                spec.case_id,
                spec.seed
            ),
        );
        Ok(Self {
            temp: Some(temp),
            spec,
            roots,
            environment,
            clock,
            identities,
            log,
            capabilities: CapabilityProbe,
            barriers: BarrierController::default(),
            faults: FaultController::default(),
            ephemeral: BTreeSet::new(),
            expected_residue: BTreeSet::new(),
        })
    }

    pub fn spec(&self) -> &FixtureSpec {
        &self.spec
    }

    pub fn fixture_id(&self) -> String {
        format!(
            "fixture-{:016x}",
            stable_hash(self.spec.seed, &self.spec.case_id, 0)
        )
    }

    pub fn roots(&self) -> &RootSet {
        &self.roots
    }

    pub fn environment(&self) -> &FixtureEnvironment {
        &self.environment
    }

    pub fn clock(&self) -> DeterministicClock {
        self.clock.clone()
    }

    pub fn identities(&self) -> DeterministicIdentity {
        self.identities.clone()
    }

    pub fn log(&self) -> EventLog {
        self.log.clone()
    }

    pub fn capabilities(&self) -> &CapabilityProbe {
        &self.capabilities
    }

    pub fn barriers(&self) -> BarrierController {
        self.barriers.clone()
    }

    pub fn faults(&self) -> FaultController {
        self.faults.clone()
    }

    pub fn require(&self, capability: Capability) -> Result<(), FixtureError> {
        self.capabilities
            .status(capability)
            .map_err(FixtureError::Unsupported)
    }

    pub fn record(&self, kind: impl Into<String>, detail: impl Into<String>) {
        self.log.record(&self.clock, kind, detail);
    }

    pub fn track_ephemeral(&mut self, path: impl Into<PathBuf>) -> Result<(), FixtureError> {
        let path = self.roots.confine(&path.into())?;
        self.ephemeral.insert(path);
        Ok(())
    }

    /// Publish a fixture-owned executable through a closed, durable handle.
    ///
    /// The temporary file is created exclusively, written and synced while
    /// owned by the fixture, then closed before the atomic rename. The final
    /// read-open is the explicit readiness boundary observed by callers.
    pub fn publish_executable(
        &mut self,
        identity: impl AsRef<str>,
        contents: &[u8],
    ) -> Result<PathBuf, FixtureError> {
        let identity = identity.as_ref();
        if identity.is_empty()
            || identity == "."
            || identity == ".."
            || identity.contains('/')
            || identity.contains('\\')
            || identity.contains('\n')
            || identity.contains('\r')
        {
            return Err(FixtureError::InvalidPath {
                path: identity.to_owned(),
                reason: "executable identity must be a single path component",
            });
        }
        let script_path = self
            .roots
            .resolve(RootKind::Artifacts, &format!("{identity}.sh"))?;
        let script_temp_path = self
            .roots
            .resolve(RootKind::Artifacts, &format!("{identity}.script.tmp"))?;

        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&script_temp_path)
            .map_err(|error| publication_io("create temporary executable", error))?;
        file.write_all(contents)
            .map_err(|error| publication_io("write temporary executable", error))?;
        file.sync_all()
            .map_err(|error| publication_io("sync temporary executable", error))?;
        drop(file);
        self.track_ephemeral(&script_temp_path)?;
        self.set_mode(&script_temp_path, 0o755)?;
        fs::rename(&script_temp_path, &script_path)
            .map_err(|error| publication_io("rename executable", error))?;
        self.track_ephemeral(&script_path)?;

        // Re-open only for the readiness observation. The handle is closed
        // before the path is returned, so no writer can race the first spawn.
        let ready = fs::File::open(&script_path)
            .map_err(|error| publication_io("open executable readiness handle", error))?;
        let metadata = ready
            .metadata()
            .map_err(|error| publication_io("read executable readiness metadata", error))?;
        if !metadata.is_file() {
            return Err(FixtureError::Invariant(format!(
                "published executable is not a regular file: {}",
                script_path.display()
            )));
        }
        drop(ready);
        #[cfg(target_os = "linux")]
        {
            fs::File::open(self.roots.artifacts())
                .and_then(|directory| directory.sync_all())
                .map_err(|error| publication_io("sync executable publication directory", error))?;
        }
        self.record(
            "fixture.executable.publish",
            format!(
                "path={};readiness=closed",
                relative_path(self.roots.root(), &script_path)
            ),
        );
        Ok(script_path)
    }

    pub fn expect_residue(&mut self, path: impl Into<PathBuf>) -> Result<(), FixtureError> {
        let path = self.roots.confine(&path.into())?;
        self.expected_residue.insert(path);
        Ok(())
    }

    pub fn create_alias(
        &mut self,
        kind: AliasKind,
        target: impl Into<PathBuf>,
        alias: impl Into<PathBuf>,
    ) -> Result<FileIdentity, FixtureError> {
        let target = target.into();
        let alias = self.roots.confine(&alias.into())?;
        let target = match self.roots.confine(&target) {
            Ok(path) => path,
            Err(_) => return Err(FixtureError::EscapesRoot(alias)),
        };
        self.require(match kind {
            AliasKind::Symlink => Capability::Symlink,
            AliasKind::HardLink => Capability::HardLink,
        })?;
        if !target.exists() {
            return Err(FixtureError::Io(io::Error::new(
                io::ErrorKind::NotFound,
                format!("alias target does not exist: {}", target.display()),
            )));
        }
        self.roots.identity(&target)?;
        if let Some(parent) = alias.parent() {
            fs::create_dir_all(parent)?;
        }
        match kind {
            AliasKind::Symlink => create_symlink(&target, &alias)?,
            AliasKind::HardLink => fs::hard_link(&target, &alias)?,
        }
        self.track_ephemeral(&alias)?;
        self.record(
            "fixture.alias",
            format!(
                "kind={kind:?};target={};alias={}",
                relative_path(self.roots.root(), &target),
                relative_path(self.roots.root(), &alias)
            ),
        );
        self.roots.identity(&alias)
    }

    pub fn set_mode(&mut self, path: impl Into<PathBuf>, mode: u32) -> Result<(), FixtureError> {
        self.require(Capability::UnixPermissions)?;
        let path = self.roots.confine(&path.into())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = fs::metadata(&path)?.permissions();
            permissions.set_mode(mode);
            fs::set_permissions(&path, permissions)?;
        }
        self.record(
            "fixture.permissions",
            format!(
                "path={};mode={mode:o}",
                relative_path(self.roots.root(), &path)
            ),
        );
        Ok(())
    }

    pub fn create_fifo(&mut self, path: impl Into<PathBuf>) -> Result<PathBuf, FixtureError> {
        self.require(Capability::Fifo)?;
        let path = self.roots.confine(&path.into())?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut command = Command::new("mkfifo");
        self.environment.apply(&mut command);
        let output = command.arg(&path).output()?;
        if !output.status.success() {
            return Err(command_error("mkfifo", output));
        }
        self.track_ephemeral(&path)?;
        self.record(
            "fixture.special-file",
            format!("fifo={}", relative_path(self.roots.root(), &path)),
        );
        Ok(path)
    }

    pub fn create_git_repository(
        &mut self,
        path: impl Into<PathBuf>,
        state: DirtyGitState,
    ) -> Result<GitSnapshot, FixtureError> {
        self.require(Capability::Git)?;
        let path = self.roots.confine(&path.into())?;
        fs::create_dir_all(&path)?;
        self.run_git(&path, &["init", "--quiet"])?;
        fs::write(path.join("tracked.txt"), b"initial\n")?;
        self.run_git(&path, &["add", "tracked.txt"])?;
        self.run_git(
            &path,
            &["commit", "--quiet", "--no-gpg-sign", "-m", "fixture"],
        )?;
        match state {
            DirtyGitState::Clean => {}
            DirtyGitState::Untracked => fs::write(path.join("untracked.txt"), b"untracked\n")?,
            DirtyGitState::Modified => fs::write(path.join("tracked.txt"), b"modified\n")?,
            DirtyGitState::Staged => {
                fs::write(path.join("tracked.txt"), b"staged\n")?;
                self.run_git(&path, &["add", "tracked.txt"])?;
            }
        }
        let output = self.run_git(&path, &["status", "--porcelain=v1"])?;
        let porcelain = String::from_utf8_lossy(&output.stdout).into_owned();
        self.record(
            "fixture.git-state",
            format!(
                "path={};state={state:?};status={porcelain:?}",
                relative_path(self.roots.root(), &path)
            ),
        );
        Ok(GitSnapshot {
            root: path,
            porcelain,
            state,
        })
    }

    pub fn run_git(&self, root: &Path, args: &[&str]) -> Result<Output, FixtureError> {
        let mut command = Command::new("git");
        self.environment.apply(&mut command);
        let output = command.current_dir(root).args(args).output()?;
        if output.status.success() {
            Ok(output)
        } else {
            Err(command_error("git", output))
        }
    }

    pub fn cleanup(mut self, outcome: FixtureOutcome) -> CleanupReport {
        let root = self
            .temp
            .as_ref()
            .expect("fixture temp directory is present")
            .path()
            .to_path_buf();
        let retain = self.spec.cleanup == CleanupMode::RetainAlways
            || (self.spec.cleanup == CleanupMode::RemoveOnSuccessRetainOnFailure
                && outcome == FixtureOutcome::Failure);
        if !retain {
            for path in &self.ephemeral {
                if path.exists() {
                    let _ = fs::remove_file(path);
                }
            }
        }
        let expected_residue = self
            .expected_residue
            .iter()
            .filter(|path| path.exists())
            .cloned()
            .collect::<Vec<_>>();
        let leaks = self
            .ephemeral
            .iter()
            .filter(|path| path.exists() && !self.expected_residue.contains(*path))
            .cloned()
            .collect::<Vec<_>>();
        self.record(
            "fixture.cleanup",
            format!(
                "outcome={outcome:?};retain={retain};expected={};leaks={}",
                expected_residue.len(),
                leaks.len()
            ),
        );
        if retain {
            let retained_root = self
                .temp
                .take()
                .expect("fixture temp directory is present")
                .keep();
            debug_assert_eq!(root, retained_root);
            CleanupReport {
                root: retained_root,
                removed: false,
                retained: true,
                expected_residue,
                leaks,
            }
        } else {
            drop(self.temp.take());
            CleanupReport {
                root,
                removed: true,
                retained: false,
                expected_residue,
                leaks,
            }
        }
    }
}

fn publication_io(operation: &str, error: io::Error) -> FixtureError {
    FixtureError::Io(io::Error::new(
        error.kind(),
        format!("{operation}: {error}"),
    ))
}

fn command_error(program: &str, output: Output) -> FixtureError {
    FixtureError::Command {
        program: program.to_owned(),
        status: output.status.to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
    }
}

fn relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .map(|relative| relative.display().to_string())
        .unwrap_or_else(|_| "<outside>".to_owned())
}

fn git_available() -> bool {
    let mut command = Command::new("git");
    command.env_clear().env("PATH", safe_tool_path());
    command
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn mkfifo_available() -> bool {
    let mut command = Command::new("mkfifo");
    command.env_clear().env("PATH", safe_tool_path());
    command
        .arg("--help")
        .output()
        .map(|output| output.status.success() || output.status.code() == Some(1))
        .unwrap_or(false)
}

fn safe_tool_path() -> &'static str {
    #[cfg(unix)]
    {
        "/usr/local/bin:/usr/bin:/bin"
    }
    #[cfg(windows)]
    {
        r"C:\Windows\System32"
    }
}

#[cfg(unix)]
fn create_symlink(target: &Path, alias: &Path) -> Result<(), FixtureError> {
    std::os::unix::fs::symlink(target, alias)?;
    Ok(())
}

#[cfg(windows)]
fn create_symlink(target: &Path, alias: &Path) -> Result<(), FixtureError> {
    std::os::windows::fs::symlink_file(target, alias)?;
    Ok(())
}
