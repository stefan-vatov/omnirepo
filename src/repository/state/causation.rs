use super::{
    AuthorityIdentity, AuthorizedDelta, CanonicalRepresentation, DomainError,
    ManagedTargetIdentity, RepositorySnapshot, validate_text,
};
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ObservedFact<T>(T);

impl<T> ObservedFact<T> {
    pub const fn new(value: T) -> Self {
        Self(value)
    }

    pub fn value(&self) -> &T {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct OwnerDecision<T>(T);

impl<T> OwnerDecision<T> {
    pub const fn new(value: T) -> Self {
        Self(value)
    }

    pub fn value(&self) -> &T {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum CausationRelation {
    Direct,
    Unrelated,
    Uncertain,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum CausationBasis {
    BaselineComparison,
    FailureEvidence,
    NotEstablished,
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct BaselineIdentityProof {
    before: AuthorityIdentity,
    after: AuthorityIdentity,
    before_snapshot: CanonicalRepresentation,
    after_snapshot: CanonicalRepresentation,
}

impl BaselineIdentityProof {
    pub fn from_snapshot(
        expected: &RepositorySnapshot,
        observed: &RepositorySnapshot,
    ) -> Result<Self, DomainError> {
        let before_snapshot = expected.canonical_representation();
        let after_snapshot = observed.canonical_representation();
        if before_snapshot != after_snapshot {
            return Err(DomainError::InvalidCausation {
                relation: CausationRelation::Direct,
                basis: CausationBasis::BaselineComparison,
            });
        }
        Ok(Self {
            before: expected.identity(),
            after: observed.identity(),
            before_snapshot,
            after_snapshot,
        })
    }

    pub fn before(&self) -> &AuthorityIdentity {
        &self.before
    }

    pub fn after(&self) -> &AuthorityIdentity {
        &self.after
    }

    pub fn before_snapshot(&self) -> &CanonicalRepresentation {
        &self.before_snapshot
    }

    pub fn after_snapshot(&self) -> &CanonicalRepresentation {
        &self.after_snapshot
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ManagedPathFailureProof {
    snapshot_identity: CanonicalRepresentation,
    operation: CanonicalRepresentation,
    target: ManagedTargetIdentity,
    failure: String,
}

impl ManagedPathFailureProof {
    pub fn new(
        snapshot: &RepositorySnapshot,
        operation: &AuthorizedDelta,
        target: ManagedTargetIdentity,
        failure: impl Into<String>,
    ) -> Result<Self, DomainError> {
        let failure = failure.into();
        validate_text(&failure, "managed path failure")?;
        let snapshot_identity = snapshot.canonical_representation();
        if operation.snapshot_identity() != &snapshot_identity {
            return Err(DomainError::InvalidProofBinding { field: "snapshot" });
        }
        if !operation.frozen_targets().contains(&target)
            || !operation
                .changes()
                .iter()
                .any(|change| change.target() == &target)
        {
            return Err(DomainError::InvalidProofBinding { field: "operation" });
        }
        Ok(Self {
            snapshot_identity,
            operation: operation.canonical_representation(),
            target,
            failure,
        })
    }

    pub fn snapshot_identity(&self) -> &CanonicalRepresentation {
        &self.snapshot_identity
    }

    pub fn operation(&self) -> &CanonicalRepresentation {
        &self.operation
    }

    pub fn target(&self) -> &ManagedTargetIdentity {
        &self.target
    }

    pub fn failure(&self) -> &str {
        &self.failure
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum DirectCausationProof {
    Baseline(BaselineIdentityProof),
    ManagedPath(ManagedPathFailureProof),
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct InferredCausation {
    relation: CausationRelation,
    proof: Option<DirectCausationProof>,
}

pub type CausationAssessment = InferredCausation;

impl InferredCausation {
    pub fn new(relation: CausationRelation, basis: CausationBasis) -> Result<Self, DomainError> {
        if relation == CausationRelation::Direct || !matches!(basis, CausationBasis::NotEstablished)
        {
            return Err(DomainError::InvalidCausation { relation, basis });
        }
        Ok(Self {
            relation,
            proof: None,
        })
    }

    pub fn direct(proof: DirectCausationProof) -> Self {
        Self {
            relation: CausationRelation::Direct,
            proof: Some(proof),
        }
    }

    pub fn try_direct_without_proof() -> Result<Self, DomainError> {
        Err(DomainError::InvalidCausation {
            relation: CausationRelation::Direct,
            basis: CausationBasis::NotEstablished,
        })
    }

    pub fn uncertain() -> Self {
        Self {
            relation: CausationRelation::Uncertain,
            proof: None,
        }
    }

    pub fn relation(&self) -> CausationRelation {
        self.relation
    }

    pub fn basis(&self) -> CausationBasis {
        match self.proof.as_ref() {
            Some(DirectCausationProof::Baseline(_)) => CausationBasis::BaselineComparison,
            Some(DirectCausationProof::ManagedPath(_)) => CausationBasis::FailureEvidence,
            None => CausationBasis::NotEstablished,
        }
    }

    pub fn proof(&self) -> Option<&DirectCausationProof> {
        self.proof.as_ref()
    }

    pub fn is_repair_eligible(&self) -> bool {
        matches!(self.relation, CausationRelation::Direct) && self.proof.is_some()
    }
}
