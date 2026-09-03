use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=LIGHTSPEED_GIT_SHA");
    println!("cargo:rerun-if-env-changed=LIGHTSPEED_RELEASE_VERSION");
    println!("cargo:rerun-if-env-changed=LIGHTSPEED_ENVD_TARGETS");
    println!("cargo:rerun-if-changed=../../.git/HEAD");

    let git_sha = std::env::var("LIGHTSPEED_GIT_SHA")
        .ok()
        .filter(|value| !value.is_empty())
        .or_else(|| command_output("git", &["rev-parse", "HEAD"]))
        .unwrap_or_else(|| "unknown".to_owned());
    let release_version = std::env::var("LIGHTSPEED_RELEASE_VERSION")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| std::env::var("CARGO_PKG_VERSION").expect("package version"));
    let rust_version =
        command_output("rustc", &["--version"]).unwrap_or_else(|| "rustc unknown".to_owned());
    let target = std::env::var("TARGET").unwrap_or_else(|_| "unknown".to_owned());
    // The release pipeline names the targets it publishes the environment
    // daemon for; a local build only knows its own.
    let envd_targets = std::env::var("LIGHTSPEED_ENVD_TARGETS")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| target.clone());

    emit("LIGHTSPEED_BUILD_GIT_SHA", &git_sha);
    emit("LIGHTSPEED_BUILD_VERSION", &release_version);
    emit("LIGHTSPEED_BUILD_RUST_VERSION", &rust_version);
    emit("LIGHTSPEED_BUILD_TARGET", &target);
    emit("LIGHTSPEED_BUILD_ENVD_TARGETS", &envd_targets);
}

fn command_output(command: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(command).args(args).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn emit(name: &str, value: &str) {
    println!("cargo:rustc-env={name}={value}");
}
