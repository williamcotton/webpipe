use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    // Only adjust linker search paths on macOS. Other platforms (Linux CI) use system paths.
    if cfg!(target_os = "macos") {
        println!("cargo:rerun-if-env-changed=ONIG_DIR");
        println!("cargo:rerun-if-env-changed=JQ_LIB_DIR");

        // Help the linker find oniguruma (required by libjq) when installed via Homebrew.
        if let Some(onig_prefix) = env::var_os("ONIG_DIR").map(PathBuf::from).or_else(|| brew_prefix("oniguruma")) {
            let onig_lib = onig_prefix.join("lib");
            println!("cargo:rustc-link-search=native={}", onig_lib.display());
            // Emitting the link directive is harmless even if already requested by a dependency.
            println!("cargo:rustc-link-lib=onig");
        }

        // Also ensure the jq library path is on the search list if available via env or Homebrew.
        if let Some(jq_lib_dir) = env::var_os("JQ_LIB_DIR").map(PathBuf::from).or_else(|| brew_prefix("jq").map(|p| p.join("lib"))) {
            println!("cargo:rustc-link-search=native={}", jq_lib_dir.display());
        }
    }
}

fn brew_prefix(package: &str) -> Option<PathBuf> {
    // Prefer HOMEBREW_PREFIX if present for generic root, but attempt per-package prefix first.
    // Try `brew --prefix <package>` for the most accurate path (e.g., /opt/homebrew/opt/<pkg>).
    let output = Command::new("brew")
        .args(["--prefix", package])
        .output()
        .ok()?;
    if output.status.success() {
        let s = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        let path = PathBuf::from(s);
        if path.exists() {
            return Some(path);
        }
    }

    // Fallback to generic `brew --prefix` if the package-specific prefix failed.
    let output = Command::new("brew").args(["--prefix"]).output().ok()?;
    if output.status.success() {
        let root = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        let candidate = PathBuf::from(root).join("opt").join(package);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}


