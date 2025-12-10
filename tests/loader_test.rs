use std::path::PathBuf;

#[test]
fn loader_test_succeeds() {
    // Build path to tests/loader_test.wp
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests");
    path.push("loader_test.wp");

    // Run `cargo run -- <file> --test` using the built binary
    let mut bin = PathBuf::from(env!("CARGO_BIN_EXE_webpipe"));
    // Fallback to target/debug/webpipe if CARGO_BIN_EXE is not set
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
    assert!(status.success(), "loader_test.wp failed");
}
