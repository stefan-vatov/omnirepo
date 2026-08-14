//! Contract: the ambient-effects inventory stays truthful against the live tree.
//!
//! Owned by `.50.1` (ambient-effects inventory). The inventory may close with
//! no candidates; these assertions keep it honest: the claims it makes about
//! ambient categories must hold for product code (no network, no global rayon,
//! no non-owned process spawns, no clock use outside the owned consumers).

use std::path::Path;

const INVENTORY: &str = "docs/ambient-effects-inventory.md";

fn product_files() -> Vec<std::path::PathBuf> {
    walkdir("src")
}

fn walkdir(dir: &str) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let root = Path::new(dir);
    if !root.is_dir() {
        return out;
    }
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir_path) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir_path) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().map(|e| e == "rs").unwrap_or(false) {
                out.push(path);
            }
        }
    }
    out
}

fn is_product(file: &std::path::Path) -> bool {
    let name = file.file_name().unwrap_or_default().to_string_lossy();
    !name.ends_with("_tests.rs")
        && name != "tests.rs"
        && !file.to_string_lossy().contains("/tests/")
        && !file.to_string_lossy().contains("coverage_tests")
}

fn contains(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| haystack.contains(n))
}

#[test]
fn inventory_exists_and_names_the_conclusion() {
    let text = std::fs::read_to_string(INVENTORY).expect("inventory document");
    assert!(
        text.contains("no candidates"),
        "inventory must record its verdict explicitly"
    );
    assert!(
        text.contains("necessary adapter seam") && text.contains("needless abstraction"),
        "inventory must apply the seam-vs-abstraction distinction from the acceptance criteria"
    );
}

#[test]
fn no_network_access_in_product_code() {
    let files = product_files();
    assert!(!files.is_empty(), "product files must exist");
    let mut offenders = Vec::new();
    for file in files {
        if !is_product(&file) {
            continue;
        }
        let text = std::fs::read_to_string(&file).expect("readable source");
        if contains(
            &text,
            &["std::net::", "TcpStream", "UdpSocket", "reqwest", "ureq"],
        ) {
            offenders.push(file.display().to_string());
        }
    }
    assert!(
        offenders.is_empty(),
        "product code must have no direct network access: {offenders:?}"
    );
}

#[test]
fn no_global_rayon_pool_in_product_code() {
    let mut offenders = Vec::new();
    for file in product_files() {
        if !is_product(&file) {
            continue;
        }
        let text = std::fs::read_to_string(&file).expect("readable source");
        if contains(&text, &["rayon::", "par_iter", "rayon"]) {
            offenders.push(file.display().to_string());
        }
    }
    assert!(
        offenders.is_empty(),
        "product code must not use the global rayon pool: {offenders:?}"
    );
}

#[test]
fn no_global_mutable_state_in_product_code() {
    let mut offenders = Vec::new();
    for file in product_files() {
        if !is_product(&file) {
            continue;
        }
        let text = std::fs::read_to_string(&file).expect("readable source");
        if contains(
            &text,
            &[
                "static mut ",
                "static GLOBAL",
                "OnceLock",
                "once_cell",
                "lazy_static",
            ],
        ) {
            offenders.push(file.display().to_string());
        }
    }
    assert!(
        offenders.is_empty(),
        "product code must have no global mutable state: {offenders:?}"
    );
}

#[test]
fn process_spawns_are_owned_or_inventory_listed() {
    // Every non-git process spawn in product code must be one of the owned
    // consumers named by the inventory: kill (acquisition, agent_runtime,
    // remote_push) and cargo (release_build, release_platform).
    let mut offenders = Vec::new();
    for file in product_files() {
        if !is_product(&file) {
            continue;
        }
        let text = std::fs::read_to_string(&file).expect("readable source");
        for line in text.lines() {
            if line.contains("Command::new(\"")
                && !line.contains("Command::new(\"git\")")
                && !line.contains("VerificationCommand::new(")
            {
                let owned = contains(
                    &file.to_string_lossy(),
                    &[
                        "acquisition.rs",
                        "agent_runtime.rs",
                        "remote_push.rs",
                        "release_build.rs",
                        "release_platform.rs",
                        "release_verify.rs",
                        "release_gates.rs",
                        "check_runner.rs",
                    ],
                );
                if !owned {
                    offenders.push(format!("{}: {}", file.display(), line.trim()));
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "every non-git process spawn must belong to an inventory-owned consumer: {offenders:?}"
    );
}

#[test]
fn clock_use_is_limited_to_inventory_consumers() {
    let mut offenders = Vec::new();
    for file in product_files() {
        if !is_product(&file) {
            continue;
        }
        let text = std::fs::read_to_string(&file).expect("readable source");
        if contains(&text, &["SystemTime::now", "Instant::now"]) {
            let owned = contains(
                &file.to_string_lossy(),
                &[
                    "run_record.rs",
                    "acquisition.rs",
                    "capture.rs",
                    "agent_runtime.rs",
                    "check_runner.rs",
                    "fleet_profile.rs",
                    "fleet_scenarios.rs",
                    "remote_push.rs",
                    "repair_reserve.rs",
                    "admission.rs",
                ],
            );
            if !owned {
                offenders.push(file.display().to_string());
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "clock use must stay inside inventory-owned consumers: {offenders:?}"
    );
}
