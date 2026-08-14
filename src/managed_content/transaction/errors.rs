use super::*;
use std::{error::Error, fmt};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProofError {
    Incomplete,
    NotNewCompleteRecovery,
    MissingRecoveryBinding,
}

impl fmt::Display for ProofError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Incomplete => formatter.write_str("recovery durability proof is incomplete"),
            Self::NotNewCompleteRecovery => {
                formatter.write_str("transaction is not a recovered new-complete cleanup")
            }
            Self::MissingRecoveryBinding => {
                formatter.write_str("recovery durability binding is missing")
            }
        }
    }
}

impl Error for ProofError {}

/// A precise invalid transition or artifact mismatch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TransactionError {
    InvalidTransition {
        from: TransactionState,
        operation: &'static str,
    },
    CandidateMismatch,
    ForeignTempOwner,
    TempOutsideTargetDirectory,
    TempAttemptNotIncreasing,
    CleanupResultMismatch {
        outstanding: CleanupDisposition,
        reported: CleanupResult,
    },
    FailureNotRecorded,
    DurabilityProofRequired,
    DurabilityProofMismatch,
}

impl fmt::Display for TransactionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidTransition { from, operation } => {
                write!(formatter, "cannot {operation} from {from:?} state")
            }
            Self::CandidateMismatch => {
                formatter.write_str("temporary artifact does not match the reserved candidate")
            }
            Self::ForeignTempOwner => {
                formatter.write_str("temporary artifact belongs to another operation")
            }
            Self::TempOutsideTargetDirectory => {
                formatter.write_str("temporary candidate is outside the target directory")
            }
            Self::TempAttemptNotIncreasing => {
                formatter.write_str("temporary candidate attempt is not strictly increasing")
            }
            Self::CleanupResultMismatch {
                outstanding,
                reported,
            } => write!(
                formatter,
                "cleanup result {reported:?} does not match outstanding {outstanding:?}"
            ),
            Self::FailureNotRecorded => {
                formatter.write_str("cannot finalize failure without recorded failure evidence")
            }
            Self::DurabilityProofRequired => {
                formatter.write_str("recovered new-complete content requires durability proof")
            }
            Self::DurabilityProofMismatch => {
                formatter.write_str("recovery durability proof belongs to another transaction")
            }
        }
    }
}

impl Error for TransactionError {}
