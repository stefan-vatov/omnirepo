use super::{
    AuthorityIdentity, CheckWitness, DomainError, GitFacts, ManagedTargetIdentity, RepositoryId,
    RepositoryRoot, RevisionId, validate_text,
};
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum GitRepositoryState {
    NonGit,
    Git(GitFacts),
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RepositoryFacts {
    repository_id: RepositoryId,
    root: RepositoryRoot,
    git: GitRepositoryState,
}

impl RepositoryFacts {
    pub fn new(
        repository_id: RepositoryId,
        root: RepositoryRoot,
        git: GitRepositoryState,
    ) -> Result<Self, DomainError> {
        Ok(Self {
            repository_id,
            root,
            git,
        })
    }

    pub fn repository_id(&self) -> &RepositoryId {
        &self.repository_id
    }

    pub fn root(&self) -> &RepositoryRoot {
        &self.root
    }

    pub fn git(&self) -> &GitRepositoryState {
        &self.git
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FrozenWitnesses {
    authority: String,
    source: String,
    catalog: String,
    configuration: String,
    plan: String,
    checks: Vec<CheckWitness>,
    base_head: Option<RevisionId>,
}

impl FrozenWitnesses {
    pub fn new(
        authority: impl Into<String>,
        source: impl Into<String>,
        catalog: impl Into<String>,
        configuration: impl Into<String>,
        plan: impl Into<String>,
        checks: Vec<CheckWitness>,
        base_head: Option<RevisionId>,
    ) -> Result<Self, DomainError> {
        let authority = authority.into();
        let source = source.into();
        let catalog = catalog.into();
        let configuration = configuration.into();
        let plan = plan.into();
        validate_text(&authority, "authority witness")?;
        validate_text(&source, "source witness")?;
        validate_text(&catalog, "catalog witness")?;
        validate_text(&configuration, "configuration witness")?;
        validate_text(&plan, "plan witness")?;
        for (index, check) in checks.iter().enumerate() {
            if checks[..index].contains(check) {
                return Err(DomainError::DuplicateValue {
                    field: "check witness",
                    value: check.as_str().to_owned(),
                });
            }
        }
        Ok(Self {
            authority,
            source,
            catalog,
            configuration,
            plan,
            checks,
            base_head,
        })
    }

    pub fn authority(&self) -> &str {
        &self.authority
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn catalog(&self) -> &str {
        &self.catalog
    }

    pub fn configuration(&self) -> &str {
        &self.configuration
    }

    pub fn plan(&self) -> &str {
        &self.plan
    }

    pub fn checks(&self) -> &[CheckWitness] {
        &self.checks
    }

    pub fn base_head(&self) -> Option<&RevisionId> {
        self.base_head.as_ref()
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RepositorySnapshot {
    pub(crate) facts: RepositoryFacts,
    pub(crate) witnesses: FrozenWitnesses,
    pub(crate) targets: Vec<ManagedTargetIdentity>,
}

impl RepositorySnapshot {
    pub fn new(
        facts: RepositoryFacts,
        witnesses: FrozenWitnesses,
        mut targets: Vec<ManagedTargetIdentity>,
    ) -> Result<Self, DomainError> {
        targets.sort();
        for pair in targets.windows(2) {
            if pair[0] == pair[1] {
                return Err(DomainError::DuplicateValue {
                    field: "managed target",
                    value: String::from_utf8_lossy(pair[0].path().as_bytes()).into_owned(),
                });
            }
            if pair[0].conflicts_with(&pair[1]) {
                return Err(DomainError::ConflictingTarget {
                    path: String::from_utf8_lossy(pair[0].path().as_bytes()).into_owned(),
                });
            }
        }
        Ok(Self {
            facts,
            witnesses,
            targets,
        })
    }

    pub fn facts(&self) -> &RepositoryFacts {
        &self.facts
    }

    pub fn identity(&self) -> AuthorityIdentity {
        self.facts.root().authority().clone()
    }

    pub fn witnesses(&self) -> &FrozenWitnesses {
        &self.witnesses
    }

    pub fn targets(&self) -> &[ManagedTargetIdentity] {
        &self.targets
    }
}
