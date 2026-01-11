use std::path::PathBuf;

#[test]
fn cli_test_runner_mode_succeeds() {
    // Build path to tests/e2e_app.wp
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests");
    path.push("e2e_app.wp");

    // Run `cargo run -- <file> --test` as a subprocess is slow; instead invoke binary directly via `std::process::Command`.
    // We assume `cargo test` built the binary under target/debug.
    let mut bin = PathBuf::from(env!("CARGO_BIN_EXE_webpipe"));
    // Some environments may not expose CARGO_BIN_EXE; fallback to target/debug/webpipe
    if !bin.exists() {
        bin = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        bin.push("target");
        bin.push("debug");
        bin.push(if cfg!(windows) { "webpipe.exe" } else { "webpipe" });
    }

    let status = std::process::Command::new(bin)
        .arg(&path)
        .arg("--test")
        .status()
        .expect("failed to run webpipe in test mode");
    assert!(status.success());
}

#[test]
fn cli_test_gg_context_succeeds() {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests");
    path.push("e2e_gg_context.wp");

    let mut bin = PathBuf::from(env!("CARGO_BIN_EXE_webpipe"));
    if !bin.exists() {
        bin = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        bin.push("target");
        bin.push("debug");
        bin.push(if cfg!(windows) { "webpipe.exe" } else { "webpipe" });
    }

    let status = std::process::Command::new(bin)
        .arg(&path)
        .arg("--test")
        .status()
        .expect("failed to run webpipe in test mode");
    assert!(status.success());
}


