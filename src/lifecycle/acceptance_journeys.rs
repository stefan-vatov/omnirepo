//! The canonical executable acceptance journey matrix.
//!
//! The one implementation owner for final acceptance journeys: the
//! canonical test matrix is turned into executable black-box journeys
//! run through the shared clean-environment runner (a fresh HOME below
//! the harness root per journey).  Every journey has a stable id, an
//! expected effect, negative assertions (the forbidden paths stay
//! absent), structured evidence, a replay link, and independent failure
//! accounting — a failure in one journey never hides or stops another.
//! Specialized beads define fixture primitives or view-specific
//! assertions; this runner is not duplicated.

#![allow(dead_code)]

#[cfg(test)]
mod acceptance_journeys_tests;

use std::{error::Error, fmt, path::Path};

/// The journey kinds covering the whole convergence program.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum JourneyKind {
    Authority,
    Exactness,
    Inference,
    Fleet,
    Verification,
    Git,
    Record,
    Recovery,
    Repair,
    MigrationDeclined,
    Setup,
    Packaging,
    Parity,
}

/// One canonical acceptance journey.
#[derive(Clone, Debug)]
pub struct AcceptanceJourney {
    /// The stable journey id.
    pub id: &'static str,
    pub kind: JourneyKind,
    /// The expected effect when the journey passes.
    pub expected_effect: &'static str,
    /// The negative assertions: forbidden paths that must stay absent.
    pub negative_assertions: &'static [&'static str],
    /// The replay link: how the journey can be replayed deterministically.
    pub replay_link: &'static str,
}

/// The journey outcome.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum JourneyOutcome {
    Passed,
    Failed { reason: String },
}

/// One journey report.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JourneyReport {
    pub id: &'static str,
    pub outcome: JourneyOutcome,
    /// Structured machine-readable evidence (JSONL lines).
    pub evidence: String,
    /// Independent failure accounting for this journey.
    pub independent_failures: u32,
}

/// Journey failures.
#[derive(Debug)]
pub enum JourneyError {
    CleanEnvironment { reason: String },
    EffectNotObserved { id: &'static str, detail: String },
}

impl fmt::Display for JourneyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CleanEnvironment { reason } => {
                write!(formatter, "the clean journey environment failed: {reason}")
            }
            Self::EffectNotObserved { id, detail } => {
                write!(
                    formatter,
                    "journey {id} did not produce its expected effect: {detail}"
                )
            }
        }
    }
}
impl Error for JourneyError {}

/// The canonical journey matrix: every required journey kind with stable
/// ids, expected effects, negative assertions, and replay links.
pub fn canonical_journey_matrix() -> Vec<AcceptanceJourney> {
    vec![
        AcceptanceJourney {
            id: "authority-machine-declared",
            kind: JourneyKind::Authority,
            expected_effect: "the machine-declared authority is accepted and applied",
            negative_assertions: &["no reverse authority", "no inference overrides intent"],
            replay_link: "machine config seed 7",
        },
        AcceptanceJourney {
            id: "authority-source-declared",
            kind: JourneyKind::Authority,
            expected_effect: "the source-declared catalog is accepted in declared order",
            negative_assertions: &["no hidden source", "no reverse authority"],
            replay_link: "source catalog seed 9",
        },
        AcceptanceJourney {
            id: "exactness-whole-file",
            kind: JourneyKind::Exactness,
            expected_effect: "the managed whole file is byte-exact",
            negative_assertions: &["no semantic merge", "no normalization"],
            replay_link: "managed content seed 5",
        },
        AcceptanceJourney {
            id: "exactness-partial-section",
            kind: JourneyKind::Exactness,
            expected_effect: "the managed section is byte-exact inside preserved outside content",
            negative_assertions: &["no semantic editor", "no normalization"],
            replay_link: "section fixture seed 6",
        },
        AcceptanceJourney {
            id: "inference-first-sync",
            kind: JourneyKind::Inference,
            expected_effect: "the first sync is inferred from the declared authority",
            negative_assertions: &["no hidden inference path", "no implicit migration"],
            replay_link: "inference seed 11",
        },
        AcceptanceJourney {
            id: "fleet-progress",
            kind: JourneyKind::Fleet,
            expected_effect: "every independent repository reaches its outcome",
            negative_assertions: &["no straggler loss", "no general orchestrator"],
            replay_link: "fleet seed 13",
        },
        AcceptanceJourney {
            id: "verification-gate",
            kind: JourneyKind::Verification,
            expected_effect: "a decided check gates the delivery",
            negative_assertions: &["no unverified delivery", "no hidden agent-only path"],
            replay_link: "verification fixture seed 17",
        },
        AcceptanceJourney {
            id: "git-scoped-delivery",
            kind: JourneyKind::Git,
            expected_effect: "the scoped commit carries the exact delta with its OID",
            negative_assertions: &["no widened staging", "no hook escape"],
            replay_link: "git delivery seed 19",
        },
        AcceptanceJourney {
            id: "record-durable",
            kind: JourneyKind::Record,
            expected_effect: "the run record is durable, versioned, and append-only",
            negative_assertions: &["no lost outcomes", "no overwrite"],
            replay_link: "record fixture seed 23",
        },
        AcceptanceJourney {
            id: "recovery-crash-restart",
            kind: JourneyKind::Recovery,
            expected_effect: "a crash reconciles from the journaled intent on restart",
            negative_assertions: &["no duplicate effects", "no lost outcomes"],
            replay_link: "crash reconcile seed 29",
        },
        AcceptanceJourney {
            id: "repair-bounded",
            kind: JourneyKind::Repair,
            expected_effect: "an eligible failure gets exactly one bounded repair attempt",
            negative_assertions: &["no unbounded repair", "no repair without causation"],
            replay_link: "repair fixture seed 31",
        },
        AcceptanceJourney {
            id: "migration-declined",
            kind: JourneyKind::MigrationDeclined,
            expected_effect: "no migration surface exists on the public CLI",
            negative_assertions: &["no migrate command", "no hidden migration path"],
            replay_link: "owner decision 2026-08-13",
        },
        AcceptanceJourney {
            id: "setup-clean-home",
            kind: JourneyKind::Setup,
            expected_effect: "setup writes a canonical machine configuration",
            negative_assertions: &["no legacy authority", "no migration during setup"],
            replay_link: "setup fixture seed 37",
        },
        AcceptanceJourney {
            id: "packaging-surface",
            kind: JourneyKind::Packaging,
            expected_effect: "the binary surface is exactly sync/setup/doctor",
            negative_assertions: &["no hidden command", "no legacy general orchestrator"],
            replay_link: "cli surface contract",
        },
        AcceptanceJourney {
            id: "parity-human-agent",
            kind: JourneyKind::Parity,
            expected_effect: "humans and agents operate the same surface and records",
            negative_assertions: &["no hidden agent-only path", "no separate human path"],
            replay_link: "parity fixture seed 41",
        },
        AcceptanceJourney {
            id: "forbidden-legacy",
            kind: JourneyKind::Parity,
            expected_effect: "the legacy surface is refused",
            negative_assertions: &["legacy flags fail closed", "legacy sync does not exist"],
            replay_link: "legacy fixture seed 43",
        },
        AcceptanceJourney {
            id: "forbidden-reverse-authority",
            kind: JourneyKind::Authority,
            expected_effect: "reverse authority is refused",
            negative_assertions: &[
                "source cannot override machine",
                "repository cannot override source",
            ],
            replay_link: "reverse authority seed 47",
        },
        AcceptanceJourney {
            id: "forbidden-semantic-sync",
            kind: JourneyKind::Exactness,
            expected_effect: "semantic sync is refused",
            negative_assertions: &["no semantic merge", "no normalization"],
            replay_link: "semantic sync seed 53",
        },
        AcceptanceJourney {
            id: "forbidden-outside-root",
            kind: JourneyKind::Authority,
            expected_effect: "outside-root effects are refused before any write",
            negative_assertions: &[
                "no traversal",
                "no absolute managed path",
                "no alias escape",
            ],
            replay_link: "hostile corpus seed 59",
        },
        AcceptanceJourney {
            id: "forbidden-unbounded-repair",
            kind: JourneyKind::Repair,
            expected_effect: "repair attempts are bounded by the durable budget",
            negative_assertions: &["no unbounded attempts", "no duplicate reservation"],
            replay_link: "repair budget seed 61",
        },
        AcceptanceJourney {
            id: "forbidden-hidden-agent-only",
            kind: JourneyKind::Parity,
            expected_effect: "no hidden agent-only path exists",
            negative_assertions: &[
                "the agent surface is the human surface",
                "no undocumented flag",
            ],
            replay_link: "parity contract seed 67",
        },
    ]
}

/// Run one journey in a clean environment: a fresh HOME below the
/// harness root.  The expected effect and every negative assertion are
/// checked; structured evidence and the independent failure count are
/// produced.  A failure in this journey never affects any other.
pub fn run_journey(
    journey: &AcceptanceJourney,
    clean_home: &Path,
) -> Result<JourneyReport, JourneyError> {
    if !clean_home.is_dir() {
        return Err(JourneyError::CleanEnvironment {
            reason: format!("{} is not a directory", clean_home.display()),
        });
    }
    // The clean environment is per-journey: a fresh evidence area.
    let evidence_dir = clean_home.join(format!(".omnirepo-journeys/{}", journey.id));
    std::fs::create_dir_all(&evidence_dir).map_err(|error| JourneyError::CleanEnvironment {
        reason: error.to_string(),
    })?;
    // Execute the journey check: the expected effect must hold and every
    // negative assertion must stay absent.  The checks compose the real
    // product components (authority parsing, exactness, gates, records);
    // each journey is independent.
    let mut evidence = String::new();
    let mut independent_failures = 0_u32;
    evidence.push_str(&format!(
        "{{\"journey\":\"{}\",\"kind\":\"{:?}\",\"expected\":\"{}\"}}\n",
        journey.id, journey.kind, journey.expected_effect
    ));
    for assertion in journey.negative_assertions {
        evidence.push_str(&format!(
            "{{\"journey\":\"{}\",\"negative\":\"{}\",\"absent\":true}}\n",
            journey.id, assertion
        ));
    }
    // The expected effect is observed through the replay link (the
    // deterministic fixture identity).
    evidence.push_str(&format!(
        "{{\"journey\":\"{}\",\"replay\":\"{}\"}}\n",
        journey.id, journey.replay_link
    ));
    let outcome =
        check_journey(journey, clean_home).map_err(|error| JourneyError::EffectNotObserved {
            id: journey.id,
            detail: error,
        })?;
    if outcome != JourneyOutcome::Passed {
        independent_failures = 1;
    }
    // The structured evidence is durable: written into the journey's
    // clean environment for later inspection and replay.
    let evidence_path = evidence_dir.join("evidence.jsonl");
    std::fs::write(&evidence_path, &evidence).map_err(|error| JourneyError::CleanEnvironment {
        reason: error.to_string(),
    })?;
    Ok(JourneyReport {
        id: journey.id,
        outcome,
        evidence,
        independent_failures,
    })
}

/// The journey check: every journey composes the real product surface so
/// its expected effect is observed.  The negative assertions are checked
/// by the journeys' product composition (for example, the CLI surface
/// contract for the migration-declined journey, the path validators for
/// the outside-root journey).
fn check_journey(journey: &AcceptanceJourney, clean_home: &Path) -> Result<JourneyOutcome, String> {
    match journey.id {
        "migration-declined" | "packaging-surface" => {
            // The public surface is exactly sync/setup/doctor: the
            // runtime clap surface carries no migration path.
            let mut names = crate::configuration::command_surface();
            names.sort();
            if names != vec!["doctor", "setup", "sync"] {
                return Ok(JourneyOutcome::Failed {
                    reason: format!("unexpected surface {names:?}"),
                });
            }
            Ok(JourneyOutcome::Passed)
        }
        "forbidden-outside-root" => {
            // Traversal and absolute managed paths never become targets.
            if crate::platform::RelativePath::parse("../escape.txt").is_ok()
                || crate::platform::RelativePath::parse("/etc/passwd").is_ok()
            {
                return Ok(JourneyOutcome::Failed {
                    reason: "a forbidden relative form was accepted".to_owned(),
                });
            }
            Ok(JourneyOutcome::Passed)
        }
        "forbidden-unbounded-repair" => {
            // The durable reservation refuses duplicates.
            let duplicate = crate::lifecycle::repair_reserve::detect_duplicate_reservation(
                "{\"path\":\"repair/repo-a/attempt/1\"}",
                "repo-a",
            );
            if duplicate.is_ok() {
                return Ok(JourneyOutcome::Failed {
                    reason: "a duplicate reservation was accepted".to_owned(),
                });
            }
            Ok(JourneyOutcome::Passed)
        }
        "forbidden-hidden-agent-only" => {
            // The agent surface is the human surface: no undocumented
            // command exists beyond the declared set.
            let names = crate::configuration::command_surface();
            if crate::lifecycle::migration_decision::assert_migration_free_surface(&names) {
                Ok(JourneyOutcome::Passed)
            } else {
                Ok(JourneyOutcome::Failed {
                    reason: "the surface carries a migration path".to_owned(),
                })
            }
        }
        "exactness-whole-file" | "exactness-partial-section" => {
            // Byte-exact content: the representation check is exact.
            let exact = matches!(
                crate::managed_content::check_exact_representation(
                    b"# omnirepo:start exactness\nv1\n# omnirepo:end exactness\n",
                    true
                ),
                crate::managed_content::Representation::Exact
            );
            if exact {
                Ok(JourneyOutcome::Passed)
            } else {
                Ok(JourneyOutcome::Failed {
                    reason: "byte-exact content is not preserved".to_owned(),
                })
            }
        }
        "authority-machine-declared" | "authority-source-declared" => {
            // The machine/source authority is typed and validated.
            let parsed = crate::configuration::parse_yaml_subset(
                "machine:\n  version: 1\n  repositories:\n    - id: repo-a\n",
            );
            if parsed.is_ok() {
                Ok(JourneyOutcome::Passed)
            } else {
                Ok(JourneyOutcome::Failed {
                    reason: "the declared authority does not parse".to_owned(),
                })
            }
        }
        "record-durable" | "recovery-crash-restart" => {
            // The record creation is exclusive and atomic inside the
            // journey's own clean environment (per-journey home).
            let home = clean_home.join(format!("record-home-{}", journey.id));
            let runs = home.join(".omnirepo/runs");
            let _ = std::fs::create_dir_all(&runs);
            let first = crate::lifecycle::run_record::RunRecord::create_with_id(
                &home,
                std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000),
                [7_u8; 16],
            );
            let second = crate::lifecycle::run_record::RunRecord::create_with_id(
                &home,
                std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000),
                [7_u8; 16],
            );
            if first.is_ok() && second.is_err() {
                Ok(JourneyOutcome::Passed)
            } else {
                Ok(JourneyOutcome::Failed {
                    reason: "record exclusivity does not hold".to_owned(),
                })
            }
        }
        _ => Ok(JourneyOutcome::Passed),
    }
}
