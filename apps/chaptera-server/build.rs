use std::{env, process::Command};

fn main() {
    println!("cargo:rerun-if-env-changed=CHAPTERA_BUILD_GIT_SHA");

    let git_sha = env::var("CHAPTERA_BUILD_GIT_SHA")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(git_head)
        .unwrap_or_else(|| "unknown".to_owned());

    println!("cargo:rustc-env=CHAPTERA_BUILD_GIT_SHA={git_sha}");
}

fn git_head() -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--short=12", "HEAD"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let value = String::from_utf8(output.stdout).ok()?;
    let value = value.trim();

    (!value.is_empty()).then(|| value.to_owned())
}
