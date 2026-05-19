use std::path::PathBuf;
use std::io::Write;
use std::process::{Command, Stdio};

fn webpipe_bin() -> PathBuf {
    let mut bin = PathBuf::from(env!("CARGO_BIN_EXE_webpipe"));
    if !bin.exists() {
        bin = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        bin.push("target");
        bin.push("debug");
        bin.push(if cfg!(windows) { "webpipe.exe" } else { "webpipe" });
    }
    bin
}

fn webpipe_command() -> Command {
    let mut command = Command::new(webpipe_bin());
    command.env("WEBPIPE_DISABLE_SYSTEM_PROXY", "1");
    command
}

#[test]
fn cli_test_runner_mode_succeeds() {
    // Build path to tests/e2e_app.wp
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests");
    path.push("e2e_app.wp");

    // Run `cargo run -- <file> --test` as a subprocess is slow; instead invoke binary directly via `std::process::Command`.
    // We assume `cargo test` built the binary under target/debug.
    let status = webpipe_command()
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

    let status = webpipe_command()
        .arg(&path)
        .arg("--test")
        .status()
        .expect("failed to run webpipe in test mode");
    assert!(status.success());
}

#[test]
fn cli_main_pipeline_outputs_json_and_exits_zero() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app_path = dir.path().join("app.wp");
    std::fs::write(
        &app_path,
        r#"
pipeline main =
  |> jq: `{ ok: true, count: 2 }`
"#,
    )
    .expect("write app");

    let output = webpipe_command()
        .arg(&app_path)
        .output()
        .expect("run webpipe");

    assert!(output.status.success(), "stderr: {}", String::from_utf8_lossy(&output.stderr));
    let stdout: serde_json::Value = serde_json::from_slice(&output.stdout).expect("json stdout");
    assert_eq!(stdout["ok"], serde_json::json!(true));
    assert_eq!(stdout["count"], serde_json::json!(2));
}

#[test]
fn cli_main_pipeline_result_error_prints_output_and_exits_one() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app_path = dir.path().join("app.wp");
    std::fs::write(
        &app_path,
        r#"
pipeline main =
  |> jq: `{ errors: [{ type: "validationError", message: "bad input" }] }`
  |> result
    validationError(422):
      |> jq: `{ error: .errors[0].message }`
    ok(200):
      |> jq: `{ ok: true }`
"#,
    )
    .expect("write app");

    let output = webpipe_command()
        .arg(&app_path)
        .output()
        .expect("run webpipe");

    assert!(!output.status.success());
    let stdout: serde_json::Value = serde_json::from_slice(&output.stdout).expect("json stdout");
    assert_eq!(stdout["error"], serde_json::json!("bad input"));
}

#[test]
fn cli_main_pipeline_stdin_json_consumes_piped_input() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app_path = dir.path().join("app.wp");
    std::fs::write(
        &app_path,
        r#"
pipeline main =
  |> stdin: json
  |> jq: `{ sum: (.a + .b) }`
"#,
    )
    .expect("write app");

    let mut child = webpipe_command()
        .arg(&app_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn webpipe");

    let mut stdin = child.stdin.take().expect("stdin");
    stdin
        .write_all(br#"{ "a": 2, "b": 3 }"#)
        .expect("write stdin");
    drop(stdin);

    let output = child.wait_with_output().expect("wait");
    assert!(output.status.success(), "stderr: {}", String::from_utf8_lossy(&output.stderr));
    let stdout: serde_json::Value = serde_json::from_slice(&output.stdout).expect("json stdout");
    assert_eq!(stdout["sum"], serde_json::json!(5));
}

#[test]
fn cli_main_pipeline_stdin_text_preserves_input_and_stays_quiet() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app_path = dir.path().join("app.wp");
    let scripts_path = dir.path().join("scripts");
    std::fs::create_dir(&scripts_path).expect("create scripts dir");
    std::fs::write(scripts_path.join("helper.lua"), "return {}").expect("write script");
    std::fs::write(
        &app_path,
        r#"
pipeline main =
  |> stdin: text
  |> jq: `{ message: . }`
"#,
    )
    .expect("write app");

    let mut child = webpipe_command()
        .arg(&app_path)
        .current_dir(dir.path())
        .env_remove("RUST_LOG")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn webpipe");

    let mut stdin = child.stdin.take().expect("stdin");
    stdin.write_all(b"test\n").expect("write stdin");
    drop(stdin);

    let output = child.wait_with_output().expect("wait");
    assert!(output.status.success(), "stderr: {}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
    let stdout: serde_json::Value = serde_json::from_slice(&output.stdout).expect("json stdout");
    assert_eq!(stdout["message"], serde_json::json!("test\n"));
}

#[test]
fn cli_run_requires_root_main_even_if_import_defines_main() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app_path = dir.path().join("app.wp");
    let lib_path = dir.path().join("lib.wp");
    std::fs::write(
        &lib_path,
        r#"
pipeline main =
  |> jq: `{ imported: true }`
"#,
    )
    .expect("write lib");
    std::fs::write(
        &app_path,
        r#"
import "./lib.wp" as lib

GET /
  |> jq: `{ ok: true }`
"#,
    )
    .expect("write app");

    let output = webpipe_command()
        .arg(&app_path)
        .arg("--run")
        .output()
        .expect("run webpipe");

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("root-level pipeline named main"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
