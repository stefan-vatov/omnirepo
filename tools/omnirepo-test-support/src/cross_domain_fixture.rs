//! Deterministic, pure fixtures for cross-domain contract tests.
//!
//! This module is a test model, not a product implementation.  It owns no
//! filesystem, process, clock, network, Git, or policy-selection effects.
//! Every fixture has an explicit version, case ID, and seed so a failure can
//! be replayed without ambient state.

#![allow(dead_code)]

use serde_json::Value;
use std::{error::Error, fmt};

/// The version of the serialized cross-domain fixture contract.
pub const CROSS_DOMAIN_FIXTURE_CONTRACT_VERSION: &str = "cross-domain-fixtures/v1";

/// The Bead that owns this shared fixture layer.
pub const FIXTURE_OWNER_BEAD: &str = "omni-constitutional-convergence-2r9.74.2";

/// Context retained by every fixture error.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FixtureError {
    pub module: &'static str,
    pub case_id: String,
    pub seed: u64,
    pub reason: String,
}

impl FixtureError {
    fn new(
        module: &'static str,
        case_id: impl Into<String>,
        seed: u64,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            module,
            case_id: case_id.into(),
            seed,
            reason: reason.into(),
        }
    }
}

impl fmt::Display for FixtureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "fixture module={} case_id={} seed={} failed: {}",
            self.module, self.case_id, self.seed, self.reason
        )
    }
}

impl Error for FixtureError {}

/// Stable identity supplied by a test case.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FixtureSpec {
    case_id: String,
    seed: u64,
}

impl FixtureSpec {
    pub fn new(case_id: impl Into<String>, seed: u64) -> Result<Self, FixtureError> {
        let case_id = case_id.into();
        if case_id == "."
            || case_id == ".."
            || case_id.is_empty()
            || !case_id.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._-".contains(&byte)
            })
        {
            return Err(FixtureError::new(
                "identity",
                case_id,
                seed,
                "case ID must use lowercase ASCII letters, digits, '.', '_', or '-' and must not be empty",
            ));
        }
        Ok(Self { case_id, seed })
    }

    pub fn case_id(&self) -> &str {
        &self.case_id
    }

    pub const fn seed(&self) -> u64 {
        self.seed
    }
}

/// Caller identity carried through every fallible fixture constructor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FixtureContext {
    pub module: &'static str,
    pub case_id: String,
    pub seed: u64,
}

impl FixtureContext {
    pub fn new(module: &'static str, case_id: impl Into<String>, seed: u64) -> Self {
        Self {
            module,
            case_id: case_id.into(),
            seed,
        }
    }

    fn error(&self, module: &'static str, reason: impl Into<String>) -> FixtureError {
        FixtureError::new(module, self.case_id.clone(), self.seed, reason)
    }
}

/// The stable identity embedded in a generated fixture.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FixtureIdentity {
    contract_version: String,
    fixture_id: String,
    case_id: String,
    seed: u64,
}

impl FixtureIdentity {
    pub fn contract_version(&self) -> &str {
        &self.contract_version
    }

    pub fn fixture_id(&self) -> &str {
        &self.fixture_id
    }

    pub fn case_id(&self) -> &str {
        &self.case_id
    }

    pub const fn seed(&self) -> u64 {
        self.seed
    }
}

/// The complete cross-domain fixture record.  Its values are pure test data;
/// no method below invokes a product adapter or chooses owner policy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CrossDomainFixture {
    identity: FixtureIdentity,
    machine: MachineConfigurationFixture,
    catalog: SourceCatalogFixture,
    policy: RepositoryPolicyFixture,
    plan: SynchronizationPlanFixture,
    content: ContentFixture,
    path: PathIdentityFixture,
    snapshot: SnapshotFixture,
    delta: AuthorizedDeltaFixture,
    journal: JournalFixture,
    process: ProcessOutcomeFixture,
    repair: RepairFixture,
    cli: CliStatusFixture,
    release: ReleaseIdentityFixture,
}

impl CrossDomainFixture {
    pub fn new(spec: FixtureSpec) -> Self {
        let context = FixtureContext::new("cross-domain", spec.case_id.clone(), spec.seed);
        let fixture_id = format!(
            "fixture-{:016x}",
            stable_hash(spec.seed, spec.case_id.as_bytes())
        );
        let machine = MachineConfigurationFixture::deterministic(&context);
        let catalog = SourceCatalogFixture::deterministic(&context);
        let policy = RepositoryPolicyFixture::deterministic(&context);
        let plan = SynchronizationPlanFixture::from_catalog(&context, &catalog, &policy)
            .expect("deterministic fixture catalog and policy are valid");
        let content = ContentFixture::deterministic(&context);
        let path = PathIdentityFixture::deterministic(&context);
        let snapshot = SnapshotFixture::deterministic(&context);
        let delta = AuthorizedDeltaFixture::deterministic(&context);
        let journal = JournalFixture::deterministic(&context);
        let process = ProcessOutcomeFixture::deterministic(&context);
        let repair = RepairFixture::deterministic(&context);
        let cli = CliStatusFixture::deterministic(&context);
        let release = ReleaseIdentityFixture::deterministic(&context);
        Self {
            identity: FixtureIdentity {
                contract_version: CROSS_DOMAIN_FIXTURE_CONTRACT_VERSION.to_owned(),
                fixture_id,
                case_id: spec.case_id,
                seed: spec.seed,
            },
            machine,
            catalog,
            policy,
            plan,
            content,
            path,
            snapshot,
            delta,
            journal,
            process,
            repair,
            cli,
            release,
        }
    }

    pub fn identity(&self) -> &FixtureIdentity {
        &self.identity
    }

    pub fn machine(&self) -> &MachineConfigurationFixture {
        &self.machine
    }

    pub fn catalog(&self) -> &SourceCatalogFixture {
        &self.catalog
    }

    pub fn policy(&self) -> &RepositoryPolicyFixture {
        &self.policy
    }

    pub fn plan(&self) -> &SynchronizationPlanFixture {
        &self.plan
    }

    pub fn content(&self) -> &ContentFixture {
        &self.content
    }

    pub fn path(&self) -> &PathIdentityFixture {
        &self.path
    }

    pub fn snapshot(&self) -> &SnapshotFixture {
        &self.snapshot
    }

    pub fn delta(&self) -> &AuthorizedDeltaFixture {
        &self.delta
    }

    pub fn journal(&self) -> &JournalFixture {
        &self.journal
    }

    pub fn process(&self) -> &ProcessOutcomeFixture {
        &self.process
    }

    pub fn repair(&self) -> &RepairFixture {
        &self.repair
    }

    pub fn cli(&self) -> &CliStatusFixture {
        &self.cli
    }

    pub fn release(&self) -> &ReleaseIdentityFixture {
        &self.release
    }

    pub fn context(&self) -> FixtureContext {
        FixtureContext::new(
            "cross-domain",
            self.identity.case_id.clone(),
            self.identity.seed,
        )
    }

    pub fn failure(&self, module: &'static str, reason: impl Into<String>) -> FixtureError {
        FixtureError::new(
            module,
            self.identity.case_id.clone(),
            self.identity.seed,
            reason,
        )
    }

    /// Serialize the fixture's identity as a strict, byte-stable record.
    pub fn to_bytes(&self) -> Vec<u8> {
        let payload = self.payload_digest();
        format!(
            "schema={}\ncase_id={}\nseed={}\nfixture_id={}\npayload_digest={:016x}\n",
            self.identity.contract_version,
            self.identity.case_id,
            self.identity.seed,
            self.identity.fixture_id,
            payload
        )
        .into_bytes()
    }

    pub fn serialize(&self) -> Vec<u8> {
        self.to_bytes()
    }

    pub fn round_trip(&self) -> Result<Self, FixtureError> {
        let context = self.context();
        Self::from_bytes(&context, &self.to_bytes())
    }

    pub fn from_bytes(context: &FixtureContext, bytes: &[u8]) -> Result<Self, FixtureError> {
        let text = std::str::from_utf8(bytes).map_err(|error| {
            context.error("identity", format!("fixture record is not UTF-8: {error}"))
        })?;
        let mut fields = Vec::new();
        for line in text.lines() {
            let (key, value) = line
                .split_once('=')
                .ok_or_else(|| context.error("identity", "fixture record has malformed line"))?;
            if fields
                .iter()
                .any(|(seen, _): &(String, String)| seen == key)
            {
                return Err(
                    context.error("identity", format!("fixture record repeats field {key:?}"))
                );
            }
            fields.push((key.to_owned(), value.to_owned()));
        }
        if fields.len() != 5 {
            return Err(context.error(
                "identity",
                format!(
                    "fixture record must have exactly 5 fields, found {}",
                    fields.len()
                ),
            ));
        }
        let field = |name: &str| {
            fields
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.as_str())
        };
        let schema = field("schema")
            .ok_or_else(|| context.error("identity", "fixture record has no schema"))?;
        if schema != CROSS_DOMAIN_FIXTURE_CONTRACT_VERSION {
            return Err(context.error("identity", format!("unsupported fixture schema {schema:?}")));
        }
        let case_id = field("case_id")
            .ok_or_else(|| context.error("identity", "fixture record has no case ID"))?;
        if case_id != context.case_id {
            return Err(context.error(
                "identity",
                format!(
                    "fixture case ID does not match caller context: expected {:?}, got {case_id:?}",
                    context.case_id
                ),
            ));
        }
        let seed = field("seed")
            .ok_or_else(|| context.error("identity", "fixture record has no seed"))?
            .parse::<u64>()
            .map_err(|error| {
                context.error(
                    "identity",
                    format!("fixture seed is not an unsigned integer: {error}"),
                )
            })?;
        if seed != context.seed {
            return Err(context.error(
                "identity",
                format!(
                    "fixture seed does not match caller context: expected {}, got {seed}",
                    context.seed
                ),
            ));
        }
        let spec = FixtureSpec::new(case_id.to_owned(), seed)
            .map_err(|error| context.error("identity", error.reason))?;
        let fixture = Self::new(spec);
        let fixture_id = field("fixture_id")
            .ok_or_else(|| context.error("identity", "fixture record has no fixture ID"))?;
        if fixture.identity.fixture_id != fixture_id {
            return Err(context.error(
                "identity",
                format!(
                    "fixture ID mismatch: expected {}, got {fixture_id}",
                    fixture.identity.fixture_id
                ),
            ));
        }
        let digest = field("payload_digest")
            .ok_or_else(|| context.error("identity", "fixture record has no payload digest"))?;
        let expected_digest = format!("{:016x}", fixture.payload_digest());
        if digest != expected_digest {
            return Err(context.error(
                "identity",
                format!("payload digest mismatch: expected {expected_digest}, got {digest}"),
            ));
        }
        if fixture.to_bytes() != bytes {
            return Err(context.error(
                "identity",
                "fixture record is valid but not in canonical byte order",
            ));
        }
        Ok(fixture)
    }

    pub fn deserialize(context: &FixtureContext, bytes: &[u8]) -> Result<Self, FixtureError> {
        Self::from_bytes(context, bytes)
    }

    fn payload_digest(&self) -> u64 {
        let mut payload = Vec::new();
        payload.extend_from_slice(self.identity.contract_version.as_bytes());
        payload.push(0);
        payload.extend_from_slice(self.identity.case_id.as_bytes());
        payload.push(0);
        payload.extend_from_slice(&self.identity.seed.to_le_bytes());
        payload.extend_from_slice(self.identity.fixture_id.as_bytes());
        payload.extend_from_slice(format!(
            "machine={:?};catalog={:?};policy={:?};plan={:?};content={:?};path={:?};snapshot={:?};delta={:?};journal={:?};process={:?};repair={:?};cli={:?};release={:?}",
            self.machine,
            self.catalog,
            self.policy,
            self.plan,
            self.content,
            self.path,
            self.snapshot,
            self.delta,
            self.journal,
            self.process,
            self.repair,
            self.cli,
            self.release
        ).as_bytes());
        stable_hash(self.identity.seed, &payload)
    }
}

/// Typed machine authority values used by fixture cases.
pub const MAX_MACHINE_REPOSITORIES: u16 = 32;
pub const MAX_MACHINE_CHILD_WORK: u16 = 64;
pub const MAX_MACHINE_REPAIR_ATTEMPTS: u8 = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MachineLimitsFixture {
    max_repositories: u16,
    max_child_work: u16,
}

impl MachineLimitsFixture {
    pub fn new(
        context: &FixtureContext,
        max_repositories: u16,
        max_child_work: u16,
    ) -> Result<Self, FixtureError> {
        if !(1..=MAX_MACHINE_REPOSITORIES).contains(&max_repositories) {
            return Err(context.error(
                "machine-configuration",
                format!(
                    "max repositories {max_repositories} must be in 1..={MAX_MACHINE_REPOSITORIES}"
                ),
            ));
        }
        if !(1..=MAX_MACHINE_CHILD_WORK).contains(&max_child_work) {
            return Err(context.error(
                "machine-configuration",
                format!("max child work {max_child_work} must be in 1..={MAX_MACHINE_CHILD_WORK}"),
            ));
        }
        Ok(Self {
            max_repositories,
            max_child_work,
        })
    }

    pub const fn max_repositories(self) -> u16 {
        self.max_repositories
    }

    pub const fn max_child_work(self) -> u16 {
        self.max_child_work
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdapterAttemptStateFixture {
    Disabled,
    Configured,
    Reserved,
    Exhausted,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdapterAttemptConfigFixture {
    adapter_id: String,
    state: AdapterAttemptStateFixture,
    maximum_attempts: u8,
}

impl AdapterAttemptConfigFixture {
    pub fn new(
        context: &FixtureContext,
        adapter_id: impl Into<String>,
        state: AdapterAttemptStateFixture,
        maximum_attempts: u8,
    ) -> Result<Self, FixtureError> {
        let adapter_id = adapter_id.into();
        if !valid_slug(&adapter_id) {
            return Err(context.error(
                "machine-configuration",
                format!("invalid adapter ID {adapter_id:?}"),
            ));
        }
        if maximum_attempts > MAX_MACHINE_REPAIR_ATTEMPTS {
            return Err(context.error(
                "machine-configuration",
                format!(
                    "adapter attempts {maximum_attempts} exceed maximum {MAX_MACHINE_REPAIR_ATTEMPTS}"
                ),
            ));
        }
        if matches!(state, AdapterAttemptStateFixture::Disabled) && maximum_attempts != 0 {
            return Err(context.error(
                "machine-configuration",
                "disabled adapters must have zero attempts",
            ));
        }
        if !matches!(state, AdapterAttemptStateFixture::Disabled) && maximum_attempts == 0 {
            return Err(context.error(
                "machine-configuration",
                "enabled adapter states require at least one attempt",
            ));
        }
        Ok(Self {
            adapter_id,
            state,
            maximum_attempts,
        })
    }

    pub fn adapter_id(&self) -> &str {
        &self.adapter_id
    }

    pub const fn state(&self) -> AdapterAttemptStateFixture {
        self.state
    }

    pub const fn maximum_attempts(&self) -> u8 {
        self.maximum_attempts
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MachineConfigurationFixture {
    version: u16,
    source_order: Vec<String>,
    repositories: Vec<String>,
    limits: MachineLimitsFixture,
    source_declarations: Vec<String>,
    adapter_attempts: Vec<AdapterAttemptConfigFixture>,
    repair_attempts: u8,
}

impl MachineConfigurationFixture {
    pub const SCHEMA_VERSION: u16 = 1;

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        context: &FixtureContext,
        version: u16,
        source_order: Vec<String>,
        repositories: Vec<String>,
        limits: MachineLimitsFixture,
        source_declarations: Vec<String>,
        adapter_attempts: Vec<AdapterAttemptConfigFixture>,
        repair_attempts: u8,
    ) -> Result<Self, FixtureError> {
        if version != Self::SCHEMA_VERSION {
            return Err(context.error(
                "machine-configuration",
                format!("unsupported schema version {version}"),
            ));
        }
        validate_unique_slugs(context, "source order", &source_order)?;
        validate_unique_slugs(context, "repositories", &repositories)?;
        validate_unique_slugs(context, "source declarations", &source_declarations)?;
        if source_declarations != source_order {
            return Err(context.error(
                "machine-configuration",
                "source declarations must match source order exactly and in order",
            ));
        }
        if repositories.len() > usize::from(limits.max_repositories()) {
            return Err(context.error(
                "machine-configuration",
                format!(
                    "repository count {} exceeds configured limit {}",
                    repositories.len(),
                    limits.max_repositories()
                ),
            ));
        }
        for (index, adapter) in adapter_attempts.iter().enumerate() {
            if adapter_attempts[..index]
                .iter()
                .any(|previous| previous.adapter_id() == adapter.adapter_id())
            {
                return Err(context.error(
                    "machine-configuration",
                    format!("duplicate adapter ID {:?}", adapter.adapter_id()),
                ));
            }
        }
        if repair_attempts > MAX_MACHINE_REPAIR_ATTEMPTS {
            return Err(context.error(
                "machine-configuration",
                format!(
                    "repair attempts {repair_attempts} exceed maximum {MAX_MACHINE_REPAIR_ATTEMPTS}"
                ),
            ));
        }
        Ok(Self {
            version,
            source_order,
            repositories,
            limits,
            source_declarations,
            adapter_attempts,
            repair_attempts,
        })
    }

    fn deterministic(context: &FixtureContext) -> Self {
        let source_order = vec![
            "source.high".to_owned(),
            "source.low".to_owned(),
            "source.partial".to_owned(),
        ];
        let limits =
            MachineLimitsFixture::new(context, 4, 8).expect("deterministic machine limits");
        let adapter = AdapterAttemptConfigFixture::new(
            context,
            "codex",
            AdapterAttemptStateFixture::Configured,
            3,
        )
        .expect("deterministic adapter attempt");
        Self::new(
            context,
            Self::SCHEMA_VERSION,
            source_order.clone(),
            vec![format!(
                "repository-{:x}",
                stable_hash(context.seed, b"repository") % 16
            )],
            limits,
            source_order,
            vec![adapter],
            3,
        )
        .expect("deterministic machine")
    }

    pub const fn version(&self) -> u16 {
        self.version
    }

    pub fn source_order(&self) -> &[String] {
        &self.source_order
    }

    pub fn repositories(&self) -> &[String] {
        &self.repositories
    }

    pub const fn limits(&self) -> MachineLimitsFixture {
        self.limits
    }

    pub fn source_declarations(&self) -> &[String] {
        &self.source_declarations
    }

    pub fn adapter_attempts(&self) -> &[AdapterAttemptConfigFixture] {
        &self.adapter_attempts
    }

    pub const fn repair_attempts(&self) -> u8 {
        self.repair_attempts
    }
}

/// Whole-file or named-partial content declaration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SourceModeFixture {
    WholeFile,
    PartialSection { section_id: String },
}

impl SourceModeFixture {
    pub fn partial(
        context: &FixtureContext,
        section_id: impl Into<String>,
    ) -> Result<Self, FixtureError> {
        let section_id = section_id.into();
        if !valid_slug(&section_id) {
            return Err(context.error(
                "source-catalog",
                format!("invalid section ID {section_id:?}"),
            ));
        }
        Ok(Self::PartialSection { section_id })
    }

    pub fn section_id(&self) -> Option<&str> {
        match self {
            Self::WholeFile => None,
            Self::PartialSection { section_id } => Some(section_id),
        }
    }
}

/// One ordered source catalog declaration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceDeclarationFixture {
    source_id: String,
    item_id: String,
    destination: String,
    mode: SourceModeFixture,
}

impl SourceDeclarationFixture {
    pub fn new(
        context: &FixtureContext,
        source_id: impl Into<String>,
        item_id: impl Into<String>,
        destination: impl Into<String>,
        mode: SourceModeFixture,
    ) -> Result<Self, FixtureError> {
        let source_id = source_id.into();
        let item_id = item_id.into();
        let destination = destination.into();
        for (field, value) in [
            ("source ID", source_id.as_str()),
            ("managed item ID", item_id.as_str()),
            ("destination", destination.as_str()),
        ] {
            if !valid_slug(value) && field != "destination" {
                return Err(context.error("source-catalog", format!("invalid {field} {value:?}")));
            }
            if field == "destination"
                && (value.is_empty()
                    || value.starts_with('/')
                    || value.split('/').any(|part| part == ".." || part.is_empty()))
            {
                return Err(context.error(
                    "source-catalog",
                    format!("invalid contained destination {value:?}"),
                ));
            }
        }
        Ok(Self {
            source_id,
            item_id,
            destination,
            mode,
        })
    }

    pub fn source_id(&self) -> &str {
        &self.source_id
    }

    pub fn item_id(&self) -> &str {
        &self.item_id
    }

    pub fn destination(&self) -> &str {
        &self.destination
    }

    pub fn mode(&self) -> &SourceModeFixture {
        &self.mode
    }
}

/// Result of resolving one managed item in configured source order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CatalogResolutionFixture {
    NotFound,
    Resolved {
        winner: String,
        shadowed: Vec<String>,
    },
    Collision {
        item_id: String,
        candidates: Vec<String>,
    },
}

/// Ordered source declarations with explicit precedence and collision truth.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceCatalogFixture {
    entries: Vec<SourceDeclarationFixture>,
    source_order: Vec<String>,
}

impl SourceCatalogFixture {
    pub fn new(
        context: &FixtureContext,
        entries: Vec<SourceDeclarationFixture>,
    ) -> Result<Self, FixtureError> {
        let source_order = entries
            .iter()
            .map(|entry| entry.source_id().to_owned())
            .collect();
        Self::with_source_order(context, source_order, entries)
    }

    pub fn with_source_order(
        context: &FixtureContext,
        source_order: Vec<String>,
        entries: Vec<SourceDeclarationFixture>,
    ) -> Result<Self, FixtureError> {
        validate_unique_slugs(context, "source order", &source_order)?;
        let mut source_ids = Vec::new();
        for entry in &entries {
            if source_ids.iter().any(|id| id == entry.source_id()) {
                return Err(context.error(
                    "source-catalog",
                    format!("duplicate source ID {:?}", entry.source_id()),
                ));
            }
            source_ids.push(entry.source_id().to_owned());
        }
        if source_ids.len() != source_order.len()
            || source_ids.iter().any(|id| !source_order.contains(id))
        {
            return Err(context.error(
                "source-catalog",
                "source order must name each declaration exactly once",
            ));
        }
        Ok(Self {
            entries,
            source_order,
        })
    }

    fn deterministic(context: &FixtureContext) -> Self {
        let whole = SourceModeFixture::WholeFile;
        Self::new(
            context,
            vec![
                SourceDeclarationFixture::new(
                    context,
                    "source.high",
                    "item.common",
                    "config/omnirepo.yaml",
                    whole.clone(),
                )
                .expect("deterministic source declaration"),
                SourceDeclarationFixture::new(
                    context,
                    "source.low",
                    "item.common",
                    "config/omnirepo.yaml",
                    whole,
                )
                .expect("deterministic source declaration"),
                SourceDeclarationFixture::new(
                    context,
                    "source.partial",
                    "item.partial",
                    "config/settings.toml",
                    SourceModeFixture::partial(context, "sync.settings")
                        .expect("deterministic section"),
                )
                .expect("deterministic source declaration"),
            ],
        )
        .expect("deterministic catalog")
    }

    pub fn entries(&self) -> &[SourceDeclarationFixture] {
        &self.entries
    }

    pub fn source_order(&self) -> &[String] {
        &self.source_order
    }

    pub fn item_ids(&self) -> Vec<String> {
        let mut ids = Vec::new();
        for entry in &self.entries {
            if !ids.iter().any(|id| id == entry.item_id()) {
                ids.push(entry.item_id().to_owned());
            }
        }
        ids
    }

    pub fn resolve(&self, item_id: &str) -> CatalogResolutionFixture {
        let matches = self
            .source_order
            .iter()
            .filter_map(|source_id| self.declaration(source_id))
            .filter(|entry| entry.item_id() == item_id)
            .collect::<Vec<_>>();
        let Some(first) = matches.first() else {
            return CatalogResolutionFixture::NotFound;
        };
        let compatible = matches.iter().all(|entry| {
            entry.destination() == first.destination() && entry.mode() == first.mode()
        });
        let candidates = matches
            .iter()
            .map(|entry| entry.source_id().to_owned())
            .collect::<Vec<_>>();
        if compatible {
            CatalogResolutionFixture::Resolved {
                winner: first.source_id().to_owned(),
                shadowed: candidates.into_iter().skip(1).collect(),
            }
        } else {
            CatalogResolutionFixture::Collision {
                item_id: item_id.to_owned(),
                candidates,
            }
        }
    }

    pub fn declaration(&self, source_id: &str) -> Option<&SourceDeclarationFixture> {
        self.entries
            .iter()
            .find(|entry| entry.source_id() == source_id)
    }
}

/// An explicit verification command.  It is an argv value, never a shell string.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerificationCommandFixture {
    argv: Vec<String>,
}

impl VerificationCommandFixture {
    pub fn new<I, S>(context: &FixtureContext, argv: I) -> Result<Self, FixtureError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let argv = argv.into_iter().map(Into::into).collect::<Vec<_>>();
        if argv.first().is_none_or(String::is_empty) {
            return Err(context.error(
                "repository-policy",
                "verification command has no executable",
            ));
        }
        if argv.iter().any(|argument| argument.as_bytes().contains(&0)) {
            return Err(context.error("repository-policy", "verification command contains NUL"));
        }
        Ok(Self { argv })
    }

    pub fn argv(&self) -> &[String] {
        &self.argv
    }
}

/// Present policy is distinct from absent, invalid, and ambiguous policy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolicyDocumentFixture {
    all: bool,
    allow: Vec<String>,
    exclude: Vec<String>,
    commands: Vec<VerificationCommandFixture>,
}

impl PolicyDocumentFixture {
    pub fn new(
        context: &FixtureContext,
        all: bool,
        allow: Vec<String>,
        exclude: Vec<String>,
        commands: Vec<VerificationCommandFixture>,
    ) -> Result<Self, FixtureError> {
        validate_unique_slugs(context, "allow selectors", &allow)?;
        validate_unique_slugs(context, "exclude selectors", &exclude)?;
        for (index, command) in commands.iter().enumerate() {
            if commands[..index].contains(command) {
                return Err(context.error(
                    "repository-policy",
                    "verification commands contain an exact duplicate",
                ));
            }
        }
        Ok(Self {
            all,
            allow,
            exclude,
            commands,
        })
    }

    pub const fn all(&self) -> bool {
        self.all
    }

    pub fn allow(&self) -> &[String] {
        &self.allow
    }

    pub fn exclude(&self) -> &[String] {
        &self.exclude
    }

    pub fn commands(&self) -> &[VerificationCommandFixture] {
        &self.commands
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PolicyPresenceFixture {
    Absent,
    Present(PolicyDocumentFixture),
    Invalid { reason: String },
    Ambiguous { reason: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PolicyDecisionFixture {
    Infer,
    Selected,
    NotSelected,
    Rejected { reason: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepositoryPolicyFixture {
    presence: PolicyPresenceFixture,
}

impl RepositoryPolicyFixture {
    pub fn absent() -> Self {
        Self {
            presence: PolicyPresenceFixture::Absent,
        }
    }

    pub fn present(document: PolicyDocumentFixture) -> Self {
        Self {
            presence: PolicyPresenceFixture::Present(document),
        }
    }

    pub fn invalid(reason: impl Into<String>) -> Self {
        Self {
            presence: PolicyPresenceFixture::Invalid {
                reason: reason.into(),
            },
        }
    }

    pub fn ambiguous(reason: impl Into<String>) -> Self {
        Self {
            presence: PolicyPresenceFixture::Ambiguous {
                reason: reason.into(),
            },
        }
    }

    fn deterministic(context: &FixtureContext) -> Self {
        Self::present(
            PolicyDocumentFixture::new(
                context,
                false,
                vec!["item.common".to_owned()],
                Vec::new(),
                vec![
                    VerificationCommandFixture::new(context, ["cargo", "test"])
                        .expect("deterministic command"),
                ],
            )
            .expect("deterministic policy"),
        )
    }

    pub fn presence(&self) -> &PolicyPresenceFixture {
        &self.presence
    }

    pub fn decision(&self, item_id: &str) -> PolicyDecisionFixture {
        match &self.presence {
            PolicyPresenceFixture::Absent => PolicyDecisionFixture::Infer,
            PolicyPresenceFixture::Invalid { reason }
            | PolicyPresenceFixture::Ambiguous { reason } => PolicyDecisionFixture::Rejected {
                reason: reason.clone(),
            },
            PolicyPresenceFixture::Present(document) => {
                if document.exclude().iter().any(|item| item == item_id) {
                    PolicyDecisionFixture::NotSelected
                } else if document.all() || document.allow().iter().any(|item| item == item_id) {
                    PolicyDecisionFixture::Selected
                } else {
                    PolicyDecisionFixture::NotSelected
                }
            }
        }
    }
}

/// One resolved managed item in a deterministic plan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlanEntryFixture {
    source_id: String,
    item_id: String,
    destination: String,
    mode: SourceModeFixture,
}

impl PlanEntryFixture {
    pub fn source_id(&self) -> &str {
        &self.source_id
    }

    pub fn item_id(&self) -> &str {
        &self.item_id
    }

    pub fn destination(&self) -> &str {
        &self.destination
    }

    pub fn mode(&self) -> &SourceModeFixture {
        &self.mode
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SynchronizationPlanFixture {
    entries: Vec<PlanEntryFixture>,
}

impl SynchronizationPlanFixture {
    pub fn from_catalog(
        context: &FixtureContext,
        catalog: &SourceCatalogFixture,
        policy: &RepositoryPolicyFixture,
    ) -> Result<Self, FixtureError> {
        let mut entries = Vec::new();
        for item_id in catalog.item_ids() {
            let resolution = catalog.resolve(&item_id);
            let source_id = match resolution {
                CatalogResolutionFixture::Resolved { winner, .. } => winner,
                CatalogResolutionFixture::NotFound => continue,
                CatalogResolutionFixture::Collision {
                    item_id,
                    candidates,
                } => {
                    return Err(context.error(
                        "synchronization-plan",
                        format!("collision for {item_id:?}: {candidates:?}"),
                    ));
                }
            };
            match policy.decision(&item_id) {
                PolicyDecisionFixture::Infer | PolicyDecisionFixture::Selected => {
                    let declaration = catalog
                        .declaration(&source_id)
                        .expect("resolved source declaration exists");
                    entries.push(PlanEntryFixture {
                        source_id,
                        item_id,
                        destination: declaration.destination().to_owned(),
                        mode: declaration.mode().clone(),
                    });
                }
                PolicyDecisionFixture::NotSelected => {}
                PolicyDecisionFixture::Rejected { reason } => {
                    return Err(context.error(
                        "synchronization-plan",
                        format!("policy rejected plan: {reason}"),
                    ));
                }
            }
        }
        Ok(Self { entries })
    }

    pub fn entries(&self) -> &[PlanEntryFixture] {
        &self.entries
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Marker topology derived from exact fixture bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MarkerTopologyFixture {
    Absent,
    Paired,
    MissingStart,
    MissingEnd,
    Reversed,
    Duplicate,
    Nested,
    Interleaved,
    Mismatched,
    Unknown,
    WhitespaceAltered,
    PayloadLike,
    Unsupported,
}

impl MarkerTopologyFixture {
    pub const fn is_valid(self) -> bool {
        matches!(self, Self::Absent | Self::Paired)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ContentModeFixture {
    WholeFile,
    PartialSection { section_id: String },
}

impl ContentModeFixture {
    pub fn partial(
        context: &FixtureContext,
        section_id: impl Into<String>,
    ) -> Result<Self, FixtureError> {
        let section_id = section_id.into();
        if !valid_slug(&section_id) {
            return Err(context.error(
                "content-topology",
                format!("invalid section ID {section_id:?}"),
            ));
        }
        Ok(Self::PartialSection { section_id })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ContentDecisionFixture {
    Unchanged,
    ReplaceWholeFile,
    ReplacePartialSection,
    AppendPartialSection,
    InvalidMarkers(MarkerTopologyFixture),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContentFixture {
    mode: ContentModeFixture,
    source_marker_topology: MarkerTopologyFixture,
    marker_topology: MarkerTopologyFixture,
    source_bytes: Vec<u8>,
    destination_bytes: Vec<u8>,
}

impl ContentFixture {
    pub fn new(
        context: &FixtureContext,
        mode: ContentModeFixture,
        source_bytes: Vec<u8>,
        destination_bytes: Vec<u8>,
    ) -> Result<Self, FixtureError> {
        let source_marker_topology = match &mode {
            ContentModeFixture::WholeFile => MarkerTopologyFixture::Absent,
            ContentModeFixture::PartialSection { section_id } => {
                derive_marker_topology(&source_bytes, section_id)
            }
        };
        let marker_topology = match &mode {
            ContentModeFixture::WholeFile => MarkerTopologyFixture::Absent,
            ContentModeFixture::PartialSection { section_id } => {
                if !matches!(source_marker_topology, MarkerTopologyFixture::Paired) {
                    source_marker_topology
                } else {
                    derive_marker_topology(&destination_bytes, section_id)
                }
            }
        };
        if !matches!(mode, ContentModeFixture::WholeFile)
            && matches!(marker_topology, MarkerTopologyFixture::Unsupported)
        {
            return Err(context.error(
                "content-topology",
                "marker bytes use an unsupported encoding",
            ));
        }
        Ok(Self {
            mode,
            source_marker_topology,
            marker_topology,
            source_bytes,
            destination_bytes,
        })
    }

    fn deterministic(context: &FixtureContext) -> Self {
        Self::new(
            context,
            ContentModeFixture::PartialSection {
                section_id: "sync.settings".to_owned(),
            },
            b"# omnirepo:start sync.settings\nsource=true\n# omnirepo:end sync.settings\n".to_vec(),
            b"local=true\n".to_vec(),
        )
        .expect("deterministic content")
    }

    pub fn mode(&self) -> &ContentModeFixture {
        &self.mode
    }

    pub const fn marker_topology(&self) -> MarkerTopologyFixture {
        self.marker_topology
    }

    pub fn source_bytes(&self) -> &[u8] {
        &self.source_bytes
    }

    pub fn destination_bytes(&self) -> &[u8] {
        &self.destination_bytes
    }

    pub fn bytes_equal(&self) -> bool {
        self.source_bytes == self.destination_bytes
    }

    pub fn marker_lines(&self) -> Option<(String, String)> {
        match &self.mode {
            ContentModeFixture::WholeFile => None,
            ContentModeFixture::PartialSection { section_id } => Some((
                format!("# omnirepo:start {section_id}"),
                format!("# omnirepo:end {section_id}"),
            )),
        }
    }

    pub fn source_is_utf8(&self) -> bool {
        std::str::from_utf8(&self.source_bytes).is_ok()
    }

    pub fn decision(&self) -> ContentDecisionFixture {
        if matches!(self.mode, ContentModeFixture::WholeFile) {
            return if self.bytes_equal() {
                ContentDecisionFixture::Unchanged
            } else {
                ContentDecisionFixture::ReplaceWholeFile
            };
        }
        if !matches!(self.source_marker_topology, MarkerTopologyFixture::Paired) {
            return ContentDecisionFixture::InvalidMarkers(self.source_marker_topology);
        }
        if !self.marker_topology.is_valid() {
            return ContentDecisionFixture::InvalidMarkers(self.marker_topology);
        }
        if self.bytes_equal() {
            ContentDecisionFixture::Unchanged
        } else if matches!(self.marker_topology, MarkerTopologyFixture::Absent) {
            ContentDecisionFixture::AppendPartialSection
        } else {
            ContentDecisionFixture::ReplacePartialSection
        }
    }
}

fn derive_marker_topology(bytes: &[u8], expected_id: &str) -> MarkerTopologyFixture {
    #[derive(Clone, Copy)]
    enum MarkerKind {
        Start,
        End,
    }

    let mut markers = Vec::new();
    for (line_index, raw_line) in bytes.split(|byte| *byte == b'\n').enumerate() {
        let line = raw_line.strip_suffix(&b"\r"[..]).unwrap_or(raw_line);
        let Ok(text) = std::str::from_utf8(line) else {
            if line
                .windows(b"omnirepo:".len())
                .any(|window| window == b"omnirepo:")
            {
                return MarkerTopologyFixture::Unsupported;
            }
            continue;
        };
        if !text.contains("omnirepo") {
            continue;
        }
        let exact_prefix = text.starts_with("# omnirepo:");
        let whitespace_altered = text.starts_with("#  ")
            || (exact_prefix
                && (text.ends_with(' ')
                    || text.contains('\t')
                    || text
                        .strip_prefix("# omnirepo:")
                        .is_some_and(|rest| rest.contains("  "))));
        if whitespace_altered {
            return MarkerTopologyFixture::WhitespaceAltered;
        }
        let exact = [
            (format!("# omnirepo:start {expected_id}"), MarkerKind::Start),
            (format!("# omnirepo:end {expected_id}"), MarkerKind::End),
        ];
        if let Some((_, kind)) = exact.iter().find(|(marker, _)| text == marker) {
            markers.push((*kind, expected_id.to_owned(), line_index));
            continue;
        }
        if text.contains("omnirepo:") && !exact_prefix {
            return MarkerTopologyFixture::PayloadLike;
        }
        if !exact_prefix {
            return MarkerTopologyFixture::Unknown;
        }
        if text.contains("omnirepo:") {
            let Some((operation, id)) = text
                .strip_prefix("# omnirepo:")
                .and_then(|rest| rest.split_once(' '))
            else {
                return MarkerTopologyFixture::Unknown;
            };
            if !matches!(operation, "start" | "end") || !valid_slug(id) {
                return MarkerTopologyFixture::Unknown;
            }
            let kind = if operation == "start" {
                MarkerKind::Start
            } else {
                MarkerKind::End
            };
            markers.push((kind, id.to_owned(), line_index));
        }
    }

    if markers.is_empty() {
        return MarkerTopologyFixture::Absent;
    }
    let starts = markers
        .iter()
        .filter(|(kind, _, _)| matches!(kind, MarkerKind::Start))
        .collect::<Vec<_>>();
    let ends = markers
        .iter()
        .filter(|(kind, _, _)| matches!(kind, MarkerKind::End))
        .collect::<Vec<_>>();
    if starts.is_empty() {
        return MarkerTopologyFixture::MissingStart;
    }
    if ends.is_empty() {
        return MarkerTopologyFixture::MissingEnd;
    }
    if markers[0].0 as u8 == MarkerKind::End as u8 {
        return MarkerTopologyFixture::Reversed;
    }
    if starts.len() == 1 && ends.len() == 1 {
        if starts[0].1 != ends[0].1 {
            return MarkerTopologyFixture::Mismatched;
        }
        return if starts[0].2 < ends[0].2 {
            MarkerTopologyFixture::Paired
        } else {
            MarkerTopologyFixture::Reversed
        };
    }
    let distinct_ids = markers
        .iter()
        .map(|(_, id, _)| id)
        .collect::<std::collections::BTreeSet<_>>();
    if distinct_ids.len() == 1 {
        return MarkerTopologyFixture::Duplicate;
    }
    let starts_before_ends = starts[0].2 < ends[0].2;
    if starts_before_ends {
        let first_end = ends.iter().map(|(_, _, line)| *line).min().unwrap_or(0);
        let last_start = starts.iter().map(|(_, _, line)| *line).max().unwrap_or(0);
        if last_start < first_end {
            let first_start_id = starts
                .iter()
                .min_by_key(|(_, _, line)| *line)
                .map(|(_, id, _)| id);
            let first_end_id = ends
                .iter()
                .min_by_key(|(_, _, line)| *line)
                .map(|(_, id, _)| id);
            if first_start_id == first_end_id {
                MarkerTopologyFixture::Interleaved
            } else {
                MarkerTopologyFixture::Nested
            }
        } else {
            MarkerTopologyFixture::Interleaved
        }
    } else {
        MarkerTopologyFixture::Reversed
    }
}

/// Platform-aware identity values represented without touching a filesystem.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FilesystemKindFixture {
    LinuxExt,
    MacOsApfs,
    Unsupported,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FilesystemIdentityFixture {
    kind: FilesystemKindFixture,
    device: u64,
    mount: u64,
}

impl FilesystemIdentityFixture {
    pub fn new(
        context: &FixtureContext,
        kind: FilesystemKindFixture,
        device: u64,
        mount: u64,
    ) -> Result<Self, FixtureError> {
        if matches!(kind, FilesystemKindFixture::Unsupported) {
            return Err(context.error("path-identity", "unsupported filesystem identity"));
        }
        Ok(Self {
            kind,
            device,
            mount,
        })
    }

    pub const fn kind(self) -> FilesystemKindFixture {
        self.kind
    }

    pub const fn device(self) -> u64 {
        self.device
    }

    pub const fn mount(self) -> u64 {
        self.mount
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObjectKindFixture {
    Regular,
    Symlink,
    Mount,
    HardLink,
    NonRegular,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObjectIdentityFixture {
    device: u64,
    inode: u64,
    kind: ObjectKindFixture,
}

impl ObjectIdentityFixture {
    pub const fn new(device: u64, inode: u64) -> Self {
        Self {
            device,
            inode,
            kind: ObjectKindFixture::Regular,
        }
    }

    pub const fn with_kind(device: u64, inode: u64, kind: ObjectKindFixture) -> Self {
        Self {
            device,
            inode,
            kind,
        }
    }

    pub const fn device(self) -> u64 {
        self.device
    }

    pub const fn inode(self) -> u64 {
        self.inode
    }

    pub const fn kind(self) -> ObjectKindFixture {
        self.kind
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthorityIdentityFixture {
    lexical_path: String,
    filesystem: FilesystemIdentityFixture,
    object: ObjectIdentityFixture,
}

impl AuthorityIdentityFixture {
    pub fn new(
        context: &FixtureContext,
        lexical_path: impl Into<String>,
        filesystem: FilesystemIdentityFixture,
        object: ObjectIdentityFixture,
    ) -> Result<Self, FixtureError> {
        let lexical_path = lexical_path.into();
        if lexical_path.is_empty() || !lexical_path.starts_with('/') {
            return Err(context.error(
                "path-identity",
                format!("authority path must be absolute: {lexical_path:?}"),
            ));
        }
        if lexical_path
            .split('/')
            .skip(1)
            .any(|component| component == ".." || component.is_empty())
        {
            return Err(context.error(
                "path-identity",
                format!("authority path contains parent or empty component: {lexical_path:?}"),
            ));
        }
        if lexical_path.chars().any(char::is_control) {
            return Err(context.error(
                "path-identity",
                "authority path contains a control character",
            ));
        }
        if filesystem.device() != object.device() {
            return Err(context.error(
                "path-identity",
                format!(
                    "filesystem/object device mismatch: {} != {}",
                    filesystem.device(),
                    object.device()
                ),
            ));
        }
        Ok(Self {
            lexical_path,
            filesystem,
            object,
        })
    }

    pub fn lexical_path(&self) -> &str {
        &self.lexical_path
    }

    pub const fn filesystem(&self) -> FilesystemIdentityFixture {
        self.filesystem
    }

    pub const fn object(&self) -> ObjectIdentityFixture {
        self.object
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PathComparisonFixture {
    SameObject,
    DistinctObject,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PathIdentityFixture {
    authority: AuthorityIdentityFixture,
    alias: AuthorityIdentityFixture,
}

impl PathIdentityFixture {
    pub fn new(
        context: &FixtureContext,
        authority: AuthorityIdentityFixture,
        alias: AuthorityIdentityFixture,
    ) -> Result<Self, FixtureError> {
        if authority.filesystem().mount() != alias.filesystem().mount() {
            return Err(context.error("path-identity", "alias crosses a mount boundary"));
        }
        let alias_kind = if authority.object().kind() != ObjectKindFixture::Regular {
            authority.object().kind()
        } else {
            alias.object().kind()
        };
        if alias_kind != ObjectKindFixture::Regular {
            let reason = match alias_kind {
                ObjectKindFixture::Symlink => "symlink alias is not authoritative",
                ObjectKindFixture::Mount => "mount alias is not authoritative",
                ObjectKindFixture::HardLink => "hard link alias is unsafe",
                ObjectKindFixture::NonRegular => "non-regular alias is unsupported",
                ObjectKindFixture::Regular => unreachable!(),
            };
            return Err(context.error("path-identity", reason));
        }
        Ok(Self { authority, alias })
    }

    fn deterministic(context: &FixtureContext) -> Self {
        let filesystem = FilesystemIdentityFixture::new(
            context,
            if context.seed % 2 == 0 {
                FilesystemKindFixture::LinuxExt
            } else {
                FilesystemKindFixture::MacOsApfs
            },
            7,
            11,
        )
        .expect("deterministic filesystem identity");
        let authority = AuthorityIdentityFixture::new(
            context,
            "/fixture/destination",
            filesystem,
            ObjectIdentityFixture::new(7, 101),
        )
        .expect("deterministic authority identity");
        let alias = AuthorityIdentityFixture::new(
            context,
            "/fixture/destination-alias",
            filesystem,
            ObjectIdentityFixture::new(7, 101),
        )
        .expect("deterministic alias identity");
        Self::new(context, authority, alias).expect("deterministic path identity")
    }

    pub fn authority(&self) -> &AuthorityIdentityFixture {
        &self.authority
    }

    pub fn alias(&self) -> &AuthorityIdentityFixture {
        &self.alias
    }

    pub fn compare(&self) -> PathComparisonFixture {
        if self.authority.object() == self.alias.object()
            && self.authority.filesystem() == self.alias.filesystem()
        {
            PathComparisonFixture::SameObject
        } else {
            PathComparisonFixture::DistinctObject
        }
    }
}

/// Snapshot state transitions are pure and explicit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SnapshotStateFixture {
    Empty,
    Acquiring,
    Staged,
    Published,
    Failed,
    Interrupted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SnapshotEventFixture {
    BeginAcquire,
    StageReady,
    Publish,
    Fail,
    Interrupt,
    Reset,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SnapshotTransitionError {
    pub from: SnapshotStateFixture,
    pub event: SnapshotEventFixture,
}

pub fn transition_snapshot(
    state: SnapshotStateFixture,
    event: SnapshotEventFixture,
) -> Result<SnapshotStateFixture, SnapshotTransitionError> {
    let next = match (state, event) {
        (SnapshotStateFixture::Empty, SnapshotEventFixture::BeginAcquire) => {
            SnapshotStateFixture::Acquiring
        }
        (SnapshotStateFixture::Acquiring, SnapshotEventFixture::StageReady) => {
            SnapshotStateFixture::Staged
        }
        (SnapshotStateFixture::Staged, SnapshotEventFixture::Publish) => {
            SnapshotStateFixture::Published
        }
        (
            SnapshotStateFixture::Acquiring | SnapshotStateFixture::Staged,
            SnapshotEventFixture::Fail,
        ) => SnapshotStateFixture::Failed,
        (
            SnapshotStateFixture::Acquiring | SnapshotStateFixture::Staged,
            SnapshotEventFixture::Interrupt,
        ) => SnapshotStateFixture::Interrupted,
        (
            SnapshotStateFixture::Failed | SnapshotStateFixture::Interrupted,
            SnapshotEventFixture::Reset,
        ) => SnapshotStateFixture::Empty,
        _ => return Err(SnapshotTransitionError { from: state, event }),
    };
    Ok(next)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotFixture {
    state: SnapshotStateFixture,
    history: Vec<SnapshotStateFixture>,
}

impl SnapshotFixture {
    fn deterministic(_context: &FixtureContext) -> Self {
        let mut state = SnapshotStateFixture::Empty;
        let mut history = vec![state];
        for event in [
            SnapshotEventFixture::BeginAcquire,
            SnapshotEventFixture::StageReady,
            SnapshotEventFixture::Publish,
        ] {
            state = transition_snapshot(state, event).expect("deterministic transition");
            history.push(state);
        }
        Self { state, history }
    }

    pub const fn state(&self) -> SnapshotStateFixture {
        self.state
    }

    pub fn history(&self) -> &[SnapshotStateFixture] {
        &self.history
    }
}

/// One exact before/after target mutation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileChangeFixture {
    path: String,
    before: Vec<u8>,
    after: Vec<u8>,
}

impl FileChangeFixture {
    pub fn new(
        context: &FixtureContext,
        path: impl Into<String>,
        before: Vec<u8>,
        after: Vec<u8>,
    ) -> Result<Self, FixtureError> {
        let path = path.into();
        if path.is_empty() || path.starts_with('/') || path.split('/').any(|part| part == "..") {
            return Err(context.error(
                "authorized-delta",
                format!("invalid contained target path {path:?}"),
            ));
        }
        Ok(Self {
            path,
            before,
            after,
        })
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn before(&self) -> &[u8] {
        &self.before
    }

    pub fn after(&self) -> &[u8] {
        &self.after
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DeltaClassificationFixture {
    Authorized,
    Unauthorized { path: String },
    Collision { path: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthorizedDeltaFixture {
    frozen_targets: Vec<String>,
    changes: Vec<FileChangeFixture>,
    classification: DeltaClassificationFixture,
}

impl AuthorizedDeltaFixture {
    pub fn classify(
        context: &FixtureContext,
        frozen_targets: Vec<String>,
        changes: Vec<FileChangeFixture>,
    ) -> Result<Self, FixtureError> {
        validate_unique_paths(context, &frozen_targets)?;
        let mut seen = Vec::new();
        let mut classification = DeltaClassificationFixture::Authorized;
        for change in &changes {
            if seen.iter().any(|path| path == change.path()) {
                classification = DeltaClassificationFixture::Collision {
                    path: change.path().to_owned(),
                };
                break;
            }
            seen.push(change.path().to_owned());
            if !frozen_targets.iter().any(|path| path == change.path()) {
                classification = DeltaClassificationFixture::Unauthorized {
                    path: change.path().to_owned(),
                };
                break;
            }
        }
        Ok(Self {
            frozen_targets,
            changes,
            classification,
        })
    }

    fn deterministic(context: &FixtureContext) -> Self {
        Self::classify(
            context,
            vec!["config/omnirepo.yaml".to_owned()],
            vec![
                FileChangeFixture::new(
                    context,
                    "config/omnirepo.yaml",
                    b"old\n".to_vec(),
                    b"new\n".to_vec(),
                )
                .expect("deterministic change"),
            ],
        )
        .expect("deterministic delta")
    }

    pub fn frozen_targets(&self) -> &[String] {
        &self.frozen_targets
    }

    pub fn changes(&self) -> &[FileChangeFixture] {
        &self.changes
    }

    pub fn classification(&self) -> &DeltaClassificationFixture {
        &self.classification
    }
}

/// A journal event keeps enough context to replay a unit failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JournalOutcomeFixture {
    Started,
    Completed,
    Failed,
    Cancelled,
    Skipped,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JournalEventFixture {
    sequence: u64,
    case_id: String,
    seed: u64,
    stage: String,
    outcome: JournalOutcomeFixture,
}

impl JournalEventFixture {
    pub fn new(
        context: &FixtureContext,
        sequence: u64,
        case_id: impl Into<String>,
        seed: u64,
        stage: impl Into<String>,
        outcome: JournalOutcomeFixture,
    ) -> Result<Self, FixtureError> {
        let case_id = case_id.into();
        let stage = stage.into();
        if sequence == 0 || case_id.is_empty() || stage.is_empty() {
            return Err(context.error(
                "journal",
                "event sequence, case ID, and stage must be present",
            ));
        }
        if case_id.chars().any(char::is_control) || stage.chars().any(char::is_control) {
            return Err(context.error(
                "journal",
                "event identity and stage cannot contain control characters",
            ));
        }
        Ok(Self {
            sequence,
            case_id,
            seed,
            stage,
            outcome,
        })
    }

    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    pub fn case_id(&self) -> &str {
        &self.case_id
    }

    pub const fn seed(&self) -> u64 {
        self.seed
    }

    pub fn stage(&self) -> &str {
        &self.stage
    }

    pub const fn outcome(&self) -> JournalOutcomeFixture {
        self.outcome
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JournalFixture {
    events: Vec<JournalEventFixture>,
}

impl JournalFixture {
    pub fn new(
        context: &FixtureContext,
        events: Vec<JournalEventFixture>,
    ) -> Result<Self, FixtureError> {
        if events.is_empty() {
            return Err(context.error("journal", "journal must contain a starting event"));
        }
        if !matches!(events[0].outcome(), JournalOutcomeFixture::Started) {
            return Err(context.error("journal", "journal must begin with a Started event"));
        }
        for (index, event) in events.iter().enumerate() {
            let expected = index as u64 + 1;
            if event.sequence() != expected {
                return Err(context.error(
                    "journal",
                    format!("event sequence {} is not {expected}", event.sequence()),
                ));
            }
            if event.case_id() != context.case_id || event.seed() != context.seed {
                return Err(context.error(
                    "journal",
                    "journal event context does not match fixture context",
                ));
            }
            if events[..index]
                .iter()
                .any(|previous| is_terminal_journal_outcome(previous.outcome()))
            {
                return Err(context.error(
                    "journal",
                    "journal cannot append events after a terminal outcome",
                ));
            }
            if index > 0 && matches!(event.outcome(), JournalOutcomeFixture::Started) {
                return Err(context.error("journal", "Started is valid only as the first event"));
            }
        }
        Ok(Self { events })
    }

    fn deterministic(context: &FixtureContext) -> Self {
        Self::new(
            context,
            vec![
                JournalEventFixture::new(
                    context,
                    1,
                    context.case_id.clone(),
                    context.seed,
                    "fixture.start",
                    JournalOutcomeFixture::Started,
                )
                .expect("deterministic journal event"),
                JournalEventFixture::new(
                    context,
                    2,
                    context.case_id.clone(),
                    context.seed,
                    "fixture.complete",
                    JournalOutcomeFixture::Completed,
                )
                .expect("deterministic journal event"),
            ],
        )
        .expect("deterministic journal")
    }

    pub fn events(&self) -> &[JournalEventFixture] {
        &self.events
    }

    pub fn is_final(&self) -> bool {
        self.events
            .last()
            .is_some_and(|event| is_terminal_journal_outcome(event.outcome()))
    }
}

const fn is_terminal_journal_outcome(outcome: JournalOutcomeFixture) -> bool {
    matches!(
        outcome,
        JournalOutcomeFixture::Completed
            | JournalOutcomeFixture::Failed
            | JournalOutcomeFixture::Cancelled
            | JournalOutcomeFixture::Skipped
    )
}

/// Process outcomes remain distinct until a caller maps them to a CLI status.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProcessDispositionFixture {
    Success,
    ExitFailure { code: i32 },
    Signaled { signal: u8 },
    TimedOut,
    Cancelled,
    SpawnFailed { reason: String },
}

pub type ProcessOutcome = ProcessDispositionFixture;

impl ProcessDispositionFixture {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::ExitFailure { .. } => "exit-failure",
            Self::Signaled { .. } => "signaled",
            Self::TimedOut => "timeout",
            Self::Cancelled => "cancelled",
            Self::SpawnFailed { .. } => "spawn-failed",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessStatusFixture {
    pub success: bool,
    pub exit_code: Option<i32>,
    pub signal: Option<u8>,
    pub label: &'static str,
}

impl ProcessDispositionFixture {
    pub fn status(&self) -> ProcessStatusFixture {
        match self {
            Self::Success => ProcessStatusFixture {
                success: true,
                exit_code: Some(0),
                signal: None,
                label: "success",
            },
            Self::ExitFailure { code } => ProcessStatusFixture {
                success: false,
                exit_code: Some(*code),
                signal: None,
                label: "exit-failure",
            },
            Self::Signaled { signal } => ProcessStatusFixture {
                success: false,
                exit_code: None,
                signal: Some(*signal),
                label: "signaled",
            },
            Self::TimedOut => ProcessStatusFixture {
                success: false,
                exit_code: None,
                signal: None,
                label: "timeout",
            },
            Self::Cancelled => ProcessStatusFixture {
                success: false,
                exit_code: None,
                signal: None,
                label: "cancelled",
            },
            Self::SpawnFailed { .. } => ProcessStatusFixture {
                success: false,
                exit_code: None,
                signal: None,
                label: "spawn-failed",
            },
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessOutcomeFixture {
    outcome: ProcessDispositionFixture,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

impl ProcessOutcomeFixture {
    pub fn new(outcome: ProcessDispositionFixture, stdout: Vec<u8>, stderr: Vec<u8>) -> Self {
        Self {
            outcome,
            stdout,
            stderr,
        }
    }

    fn deterministic(_context: &FixtureContext) -> Self {
        Self::new(
            ProcessDispositionFixture::Success,
            b"ok\n".to_vec(),
            Vec::new(),
        )
    }

    pub fn outcome(&self) -> &ProcessDispositionFixture {
        &self.outcome
    }

    pub fn stdout(&self) -> &[u8] {
        &self.stdout
    }

    pub fn stderr(&self) -> &[u8] {
        &self.stderr
    }

    pub fn status(&self) -> ProcessStatusFixture {
        self.outcome.status()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RepairCausationFixture {
    CurrentManagedPath,
    PriorPassingBaseline,
    Unrelated,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepairInputFixture {
    causation: RepairCausationFixture,
    attempts_used: u8,
    maximum_attempts: u8,
}

impl RepairInputFixture {
    pub fn new(
        context: &FixtureContext,
        causation: RepairCausationFixture,
        attempts_used: u8,
        maximum_attempts: u8,
    ) -> Result<Self, FixtureError> {
        if maximum_attempts > MAX_MACHINE_REPAIR_ATTEMPTS {
            return Err(context.error(
                "repair",
                format!(
                    "repair maximum {maximum_attempts} exceeds maximum {MAX_MACHINE_REPAIR_ATTEMPTS}"
                ),
            ));
        }
        if attempts_used > maximum_attempts {
            return Err(context.error(
                "repair",
                format!("repair attempts used {attempts_used} exceeds maximum {maximum_attempts}"),
            ));
        }
        Ok(Self {
            causation,
            attempts_used,
            maximum_attempts,
        })
    }

    pub const fn causation(&self) -> RepairCausationFixture {
        self.causation
    }

    pub const fn attempts_used(&self) -> u8 {
        self.attempts_used
    }

    pub const fn maximum_attempts(&self) -> u8 {
        self.maximum_attempts
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RepairEligibilityFixture {
    Eligible { next_attempt: u8 },
    Ineligible { reason: &'static str },
}

pub fn repair_eligibility(input: RepairInputFixture) -> RepairEligibilityFixture {
    if !matches!(
        input.causation,
        RepairCausationFixture::CurrentManagedPath | RepairCausationFixture::PriorPassingBaseline
    ) {
        return RepairEligibilityFixture::Ineligible {
            reason: "causation is not established",
        };
    }
    if input.attempts_used >= input.maximum_attempts {
        return RepairEligibilityFixture::Ineligible {
            reason: "repair attempts are exhausted",
        };
    }
    RepairEligibilityFixture::Eligible {
        next_attempt: input.attempts_used + 1,
    }
}

pub fn cli_status_for_process(
    outcome: &ProcessDispositionFixture,
    selected_repositories: usize,
    successful_repositories: usize,
    record_finalized: bool,
) -> CliCodeFixture {
    if !record_finalized {
        return CliCodeFixture::RecordFailure;
    }
    if matches!(outcome, ProcessDispositionFixture::Cancelled) {
        return CliCodeFixture::Cancelled;
    }
    if selected_repositories == 0 {
        return CliCodeFixture::InvocationError;
    }
    if successful_repositories == selected_repositories
        && matches!(outcome, ProcessDispositionFixture::Success)
    {
        return CliCodeFixture::Success;
    }
    if successful_repositories == 0 {
        CliCodeFixture::AllFailed
    } else {
        CliCodeFixture::PartialFailure
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepairFixture {
    input: RepairInputFixture,
    eligibility: RepairEligibilityFixture,
}

impl RepairFixture {
    fn deterministic(context: &FixtureContext) -> Self {
        let input =
            RepairInputFixture::new(context, RepairCausationFixture::CurrentManagedPath, 0, 3)
                .expect("deterministic repair input");
        let eligibility = repair_eligibility(input.clone());
        Self { input, eligibility }
    }

    pub fn input(&self) -> &RepairInputFixture {
        &self.input
    }

    pub fn eligibility(&self) -> &RepairEligibilityFixture {
        &self.eligibility
    }
}

/// Stable public process outcomes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CliCodeFixture {
    Success,
    InvocationError,
    PartialFailure,
    AllFailed,
    RecordFailure,
    Cancelled,
}

pub type CliStatus = CliCodeFixture;

impl CliCodeFixture {
    pub const fn code(self) -> u8 {
        match self {
            Self::Success => 0,
            Self::InvocationError => 2,
            Self::PartialFailure => 3,
            Self::AllFailed => 4,
            Self::RecordFailure => 5,
            Self::Cancelled => 130,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::InvocationError => "invocation-error",
            Self::PartialFailure => "partial-failure",
            Self::AllFailed => "all-failed",
            Self::RecordFailure => "record-failure",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CliStatusFixture {
    status: CliCodeFixture,
}

impl CliStatusFixture {
    pub fn new(status: CliCodeFixture) -> Self {
        Self { status }
    }

    fn deterministic(_context: &FixtureContext) -> Self {
        Self::new(CliCodeFixture::Success)
    }

    pub const fn status(&self) -> CliCodeFixture {
        self.status
    }
}

/// A release identity binds version, protected tag, commit, and digest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReleaseIdentityFixture {
    version: String,
    tag: String,
    commit: String,
    digest: String,
}

impl ReleaseIdentityFixture {
    pub fn new(
        context: &FixtureContext,
        version: impl Into<String>,
        tag: impl Into<String>,
        commit: impl Into<String>,
        digest: impl Into<String>,
    ) -> Result<Self, FixtureError> {
        let version = version.into();
        let tag = tag.into();
        let commit = commit.into();
        let digest = digest.into();
        let version_valid = version.split('.').count() == 3
            && version.split('.').all(|part| {
                !part.is_empty()
                    && part.bytes().all(|byte| byte.is_ascii_digit())
                    && (part == "0" || !part.starts_with('0'))
            });
        let commit_valid = commit.len() == 40 && commit.bytes().all(is_ascii_hex);
        let digest_valid = digest.len() == 71
            && digest.starts_with("sha256:")
            && digest[7..].bytes().all(is_ascii_hex);
        if !version_valid
            || version.chars().any(char::is_control)
            || tag != format!("v{version}")
            || tag.chars().any(char::is_control)
            || !commit_valid
            || !digest_valid
            || commit.chars().any(char::is_control)
            || digest.chars().any(char::is_control)
        {
            return Err(context.error(
                "release-identity",
                format!("release identity is not internally consistent: {version:?}, {tag:?}"),
            ));
        }
        Ok(Self {
            version,
            tag,
            commit,
            digest,
        })
    }

    fn deterministic(context: &FixtureContext) -> Self {
        let commit = format!(
            "{:016x}{:016x}{:08x}",
            stable_hash(context.seed, b"commit-a"),
            stable_hash(context.seed, b"commit-b"),
            stable_hash(context.seed, b"commit-c") as u32
        );
        let digest = format!(
            "sha256:{:016x}{:016x}{:016x}{:016x}",
            stable_hash(context.seed, b"digest-a"),
            stable_hash(context.seed, b"digest-b"),
            stable_hash(context.seed, b"digest-c"),
            stable_hash(context.seed, b"digest-d")
        );
        Self::new(context, "0.8.3", "v0.8.3", commit, digest)
            .expect("deterministic release identity")
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn tag(&self) -> &str {
        &self.tag
    }

    pub fn commit(&self) -> &str {
        &self.commit
    }

    pub fn digest(&self) -> &str {
        &self.digest
    }
}

/// A typed view over rows selected from the canonical traceability matrix.
///
/// The matrix owns every identity below.  This view adds only the fixture
/// domain and expectation needed by these pure tests; it does not copy fixture
/// IDs, case IDs, evidence IDs, replay IDs, or owners.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContractDomainFixture {
    MachineConfiguration,
    SourceCatalog,
    RepositoryPolicyPresence,
    Selectors,
    Precedence,
    SynchronizationPlan,
    ContentTopology,
    PathIdentity,
    SnapshotState,
    AuthorizedDelta,
    JournalEvents,
    ProcessOutcomes,
    RepairEligibility,
    CliStatuses,
    ReleaseIdentity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContractExpectationFixture {
    RoundTrip,
    Valid,
    Absent,
    Invalid,
    Ambiguous,
    Collision,
    Transition,
    Mapping,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContractCaseFixture {
    row_id: String,
    reference: String,
    fixture_id: String,
    case_id: String,
    evidence_id: String,
    replay_id: String,
    primary_owner: String,
    implementation_bead: String,
    downstream_bead: String,
    domain: ContractDomainFixture,
    expectation: ContractExpectationFixture,
}

impl ContractCaseFixture {
    fn from_matrix(
        context: &FixtureContext,
        row: &Value,
        domain: ContractDomainFixture,
        expectation: ContractExpectationFixture,
        index: usize,
    ) -> Result<Self, FixtureError> {
        let field = |name: &str| {
            row.get(name)
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .ok_or_else(|| {
                    context.error(
                        "traceability",
                        format!("canonical matrix row {index} has no non-empty {name}"),
                    )
                })
        };
        Ok(Self {
            row_id: field("id")?,
            reference: field("reference")?,
            fixture_id: field("fixture")?,
            case_id: field("case_id")?,
            evidence_id: field("evidence_id")?,
            replay_id: field("replay_id")?,
            primary_owner: field("primary_owner")?,
            implementation_bead: field("implementation_bead")?,
            downstream_bead: field("downstream_bead")?,
            domain,
            expectation,
        })
    }

    pub fn row_id(&self) -> &str {
        &self.row_id
    }

    pub fn reference(&self) -> &str {
        &self.reference
    }

    pub fn fixture_id(&self) -> &str {
        &self.fixture_id
    }

    pub fn case_id(&self) -> &str {
        &self.case_id
    }

    pub const fn domain(&self) -> ContractDomainFixture {
        self.domain
    }

    pub const fn expectation(&self) -> ContractExpectationFixture {
        self.expectation
    }

    pub fn evidence_id(&self) -> &str {
        &self.evidence_id
    }

    pub fn replay_id(&self) -> &str {
        &self.replay_id
    }

    pub fn primary_owner(&self) -> &str {
        &self.primary_owner
    }

    pub fn implementation_bead(&self) -> &str {
        &self.implementation_bead
    }

    pub fn downstream_bead(&self) -> &str {
        &self.downstream_bead
    }
}

const CANONICAL_TRACEABILITY_MATRIX: &[u8] =
    include_bytes!("../../../tests/traceability/matrix.json");

const CONTRACT_ROW_VIEW: &[(&str, ContractDomainFixture, ContractExpectationFixture)] = &[
    (
        "behavior:configuration-authority",
        ContractDomainFixture::MachineConfiguration,
        ContractExpectationFixture::Valid,
    ),
    (
        "failure:source-catalog",
        ContractDomainFixture::SourceCatalog,
        ContractExpectationFixture::RoundTrip,
    ),
    (
        "behavior:repository-policy",
        ContractDomainFixture::RepositoryPolicyPresence,
        ContractExpectationFixture::Absent,
    ),
    (
        "constitution:tension.2",
        ContractDomainFixture::Selectors,
        ContractExpectationFixture::Ambiguous,
    ),
    (
        "constitution:principle.6",
        ContractDomainFixture::Precedence,
        ContractExpectationFixture::Collision,
    ),
    (
        "failure:planning",
        ContractDomainFixture::SynchronizationPlan,
        ContractExpectationFixture::Valid,
    ),
    (
        "constitution:principle.3",
        ContractDomainFixture::ContentTopology,
        ContractExpectationFixture::Invalid,
    ),
    (
        "behavior:containment",
        ContractDomainFixture::PathIdentity,
        ContractExpectationFixture::Collision,
    ),
    (
        "failure:synchronization",
        ContractDomainFixture::SnapshotState,
        ContractExpectationFixture::Transition,
    ),
    (
        "behavior:git-delivery",
        ContractDomainFixture::AuthorizedDelta,
        ContractExpectationFixture::Mapping,
    ),
    (
        "behavior:run-record",
        ContractDomainFixture::JournalEvents,
        ContractExpectationFixture::RoundTrip,
    ),
    (
        "behavior:verification",
        ContractDomainFixture::ProcessOutcomes,
        ContractExpectationFixture::Mapping,
    ),
    (
        "behavior:repair-causation",
        ContractDomainFixture::RepairEligibility,
        ContractExpectationFixture::Invalid,
    ),
    (
        "behavior:fleet-progress",
        ContractDomainFixture::CliStatuses,
        ContractExpectationFixture::Mapping,
    ),
    (
        "behavior:packaging",
        ContractDomainFixture::ReleaseIdentity,
        ContractExpectationFixture::RoundTrip,
    ),
];

fn canonical_rows(context: &FixtureContext) -> Result<Vec<Value>, FixtureError> {
    let matrix =
        serde_json::from_slice::<Value>(CANONICAL_TRACEABILITY_MATRIX).map_err(|error| {
            context.error(
                "traceability",
                format!("canonical traceability matrix is invalid JSON: {error}"),
            )
        })?;
    if matrix.get("schema").and_then(Value::as_str) != Some("omnirepo.traceability-matrix.v1")
        || matrix.get("status").and_then(Value::as_str) != Some("canonical")
    {
        return Err(context.error(
            "traceability",
            "canonical traceability matrix schema/status is not selected",
        ));
    }
    matrix
        .get("rows")
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| context.error("traceability", "canonical matrix has no rows array"))
}

pub fn contract_cases(context: &FixtureContext) -> Result<Vec<ContractCaseFixture>, FixtureError> {
    let rows = canonical_rows(context)?;
    let mut selected = Vec::with_capacity(CONTRACT_ROW_VIEW.len());
    for (reference, domain, expectation) in CONTRACT_ROW_VIEW {
        let matches = rows
            .iter()
            .enumerate()
            .filter(|(_, row)| row.get("reference").and_then(Value::as_str) == Some(*reference))
            .collect::<Vec<_>>();
        let Some((index, row)) = matches.first() else {
            return Err(context.error(
                "traceability",
                format!("canonical row reference {reference:?} is missing"),
            ));
        };
        if matches.len() != 1 {
            return Err(context.error(
                "traceability",
                format!("canonical row reference {reference:?} is duplicated"),
            ));
        }
        selected.push(ContractCaseFixture::from_matrix(
            context,
            row,
            *domain,
            *expectation,
            *index,
        )?);
    }
    for (index, row) in selected.iter().enumerate() {
        if selected[..index].iter().any(|previous| {
            previous.row_id() == row.row_id() || previous.case_id() == row.case_id()
        }) {
            return Err(context.error(
                "traceability",
                "contract view repeats a canonical row or case identity",
            ));
        }
    }
    Ok(selected)
}

pub fn contract_case(
    context: &FixtureContext,
    case_id: &str,
) -> Result<Option<ContractCaseFixture>, FixtureError> {
    Ok(contract_cases(context)?
        .into_iter()
        .find(|case| case.case_id() == case_id))
}

fn valid_slug(value: &str) -> bool {
    value != "."
        && value != ".."
        && !value.is_empty()
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._-".contains(&byte)
        })
}

fn is_ascii_hex(byte: u8) -> bool {
    byte.is_ascii_digit() || matches!(byte, b'a'..=b'f' | b'A'..=b'F')
}

fn validate_unique_slugs(
    context: &FixtureContext,
    field: &'static str,
    values: &[String],
) -> Result<(), FixtureError> {
    for (index, value) in values.iter().enumerate() {
        if !valid_slug(value) {
            return Err(context.error(field, format!("invalid value {value:?}")));
        }
        if values[..index].contains(value) {
            return Err(context.error(field, format!("duplicate value {value:?}")));
        }
    }
    Ok(())
}

fn validate_unique_paths(context: &FixtureContext, values: &[String]) -> Result<(), FixtureError> {
    for (index, value) in values.iter().enumerate() {
        if value.is_empty() || value.starts_with('/') || value.split('/').any(|part| part == "..") {
            return Err(context.error(
                "authorized-delta",
                format!("invalid frozen target path {value:?}"),
            ));
        }
        if values[..index].contains(value) {
            return Err(context.error(
                "authorized-delta",
                format!("duplicate frozen target path {value:?}"),
            ));
        }
    }
    Ok(())
}

fn stable_hash(seed: u64, bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64 ^ seed;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}
