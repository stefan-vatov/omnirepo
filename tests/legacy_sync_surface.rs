use assert_cmd::cargo::cargo_bin_cmd;

#[test]
fn legacy_sync_selector_flags_are_rejected() {
    for flag in [
        "--url",
        "--source-file",
        "--template-file",
        "--destination",
        "--file",
    ] {
        cargo_bin_cmd!("omnirepo")
            .args(["sync", flag, "legacy-value"])
            .assert()
            .code(2);
    }
}
