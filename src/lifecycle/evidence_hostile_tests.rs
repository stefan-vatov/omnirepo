//! Adversarial evidence acceptance: redaction, truncation, and
//! control-channel injection.
//!
//! Credential sentinels are placed in the redaction policy shapes
//! (secret-key `key=value`, URL userinfo) across config, URLs, env,
//! filenames, helper output, chunk boundaries, binary/control text,
//! journal fields, terminal projections, and the agent protocol.  No
//! forbidden sentinel may appear in any rendered evidence; redaction
//! works after truncation and on chunk-assembled evidence; terminal
//! control/newline/OSC stays inert; raw evidence storage follows the
//! bounded-sanitized policy; accounting stays complete despite bounds.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::agent_runtime::run_agent;
use crate::lifecycle::diagnostic_aggregation::redact;
use crate::lifecycle::hostile_process_fixtures::{
    ProcessFixtureKind, hostile_process_corpus, materialize_process,
};
use crate::lifecycle::terminal_projection::sanitize_id;
use crate::platform::{AgentWorkingDirectoryRoot, AuthorityRoot, ReadOnly};
use std::{fs, path::Path, time::Duration};

/// A credential sentinel assembled from parts (never a real credential).
const SECRET: &str = concat!("e9f3c17a", "41b2d0c5");
const SECRET_ALT: &str = concat!("7a41b2d0", "c5e9f3c1");

/// The sentinel placements in the redaction policy shapes.
fn sentinel_text() -> String {
    format!(
        concat!(
            "machine:\n  token=",
            "{SECRET}\n",
            "repo: https://user:{SECRET_ALT}@host.example/repo.git\n",
            "env: export TOKEN={SECRET}\n",
            "file: report-token={SECRET}.txt\n",
            "helper: helper says token={SECRET}\n",
            "journal: field token={SECRET}\n"
        ),
        SECRET = SECRET,
        SECRET_ALT = SECRET_ALT
    )
}

/// The key=value placements only: the prefix-safe policy shapes that
/// remain masked under any truncation (the key always precedes its
/// value).
fn key_value_text() -> String {
    format!(
        concat!(
            "machine:\n  token=",
            "{SECRET}\n",
            "env: export TOKEN={SECRET}\n",
            "file: report-token={SECRET}.txt\n",
            "helper: helper says token={SECRET}\n",
            "journal: field token={SECRET}\n"
        ),
        SECRET = SECRET
    )
}

fn harness_root(name: &str) -> tempfile::TempDir {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    tempfile::Builder::new()
        .prefix(name)
        .tempdir_in(&base)
        .expect("fixture")
}

#[test]
fn no_forbidden_sentinel_survives_redaction_anywhere_in_the_text() {
    let text = sentinel_text();
    let rendered = redact(&text);
    assert!(!rendered.contains(SECRET), "{rendered}");
    assert!(!rendered.contains(SECRET_ALT), "{rendered}");
}

#[test]
fn redaction_works_after_truncation_at_every_length() {
    let text = key_value_text();
    let bytes = text.as_bytes();
    // Truncation removes only the tail, so every secret key precedes its
    // value in the truncated text: redaction must mask every surviving
    // value at every truncation length.
    for length in 0..bytes.len() {
        let truncated = String::from_utf8_lossy(&bytes[..length]);
        let rendered = redact(&truncated);
        assert!(!rendered.contains(SECRET), "truncated at {length} leaks");
        assert!(
            !rendered.contains(SECRET_ALT),
            "truncated at {length} leaks"
        );
    }
}

#[test]
fn chunk_assembled_evidence_is_redacted_as_one_stream() {
    let text = key_value_text();
    let bytes = text.as_bytes();
    // The product captures evidence in bounded chunks and redacts the
    // assembled stream (never per-chunk): assemble every possible chunk
    // split in order and redact — no sentinel may survive.
    for chunk_size in 1..=16 {
        let assembled = bytes
            .chunks(chunk_size)
            .map(|chunk| String::from_utf8_lossy(chunk))
            .collect::<String>();
        let rendered = redact(&assembled);
        assert!(!rendered.contains(SECRET), "chunk size {chunk_size} leaks");
        assert!(
            !rendered.contains(SECRET_ALT),
            "chunk size {chunk_size} leaks"
        );
    }
}

#[test]
fn terminal_control_newline_and_osc_are_inert_in_projections_and_redaction() {
    let hostile =
        format!("\u{1b}]0;token={SECRET}\u{07}\u{1b}[31mred\u{1b}[0m\ntoken={SECRET_ALT}\r");
    let projected = sanitize_id(&hostile);
    assert!(!projected.contains('\u{1b}'), "{projected:?}");
    assert!(!projected.contains('\u{07}'), "{projected:?}");
    assert!(!projected.contains('\n'), "{projected:?}");
    assert!(!projected.contains('\r'), "{projected:?}");
    // Redaction is the token boundary: the control-stripped text still
    // carries the policy shapes, and redaction masks every sentinel.
    let rendered = redact(&hostile);
    assert!(!rendered.contains(SECRET), "{rendered:?}");
    assert!(!rendered.contains(SECRET_ALT), "{rendered:?}");
    assert!(!redact(&projected).contains(SECRET), "{projected:?}");
    assert!(!redact(&projected).contains(SECRET_ALT), "{projected:?}");
}

#[test]
fn raw_evidence_storage_is_bounded_and_sanitized() {
    let root = harness_root("hostile-evidence-");
    fs::create_dir_all(root.path().join("destination")).expect("destination");
    let destination = root.path().join("destination");
    let corpus = hostile_process_corpus();
    let flood = corpus
        .iter()
        .find(|entry| entry.kind == ProcessFixtureKind::AgentFlood)
        .expect("flood fixture");
    let flood_path = materialize_process(flood, root.path()).expect("materialize");
    let agent_root = AuthorityRoot::<AgentWorkingDirectoryRoot, ReadOnly>::open(&destination)
        .expect("agent root");
    let confinement =
        crate::lifecycle::agent_confinement::confine(&agent_root, &[], &[]).expect("confinement");
    let evidence_dir = destination.join(".omnirepo-agent");
    let result = run_agent(
        &[flood_path.display().to_string(), "task".to_owned()],
        &confinement,
        &evidence_dir,
        Duration::from_secs(10),
    )
    .expect("flood completes");
    let stored = fs::read_to_string(&result.evidence_path).expect("stored evidence");
    assert!(
        stored.len() <= 64 * 1024,
        "raw evidence storage is bounded: {}",
        stored.len()
    );
    assert_eq!(
        stored, result.sanitized,
        "the stored file matches the sanitized evidence"
    );
}

#[test]
fn accounting_stays_complete_despite_evidence_bounds() {
    use crate::lifecycle::run_summary::{RepoOutcome, SummaryStatus, fold_summary};
    // A bounded evidence reference still yields a complete accounting.
    let summary = fold_summary(
        "run-1",
        vec![(
            "repo-a".to_owned(),
            RepoOutcome::Success,
            "process/agent/evidence-00000000000000000000000000000000".to_owned(),
        )],
        true,
    )
    .expect("summary");
    assert_eq!(summary.status, SummaryStatus::Success);
    assert_eq!(summary.repositories.len(), 1);
}
