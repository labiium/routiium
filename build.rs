use std::env;
use std::process::Command;

fn valid_revision(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn git_output(args: &[&str]) -> Option<String> {
    Command::new("git")
        .args(args)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn main() {
    println!("cargo:rerun-if-env-changed=ROUTIIUM_BUILD_REVISION");
    if let Some(head_path) = git_output(&["rev-parse", "--git-path", "HEAD"]) {
        println!("cargo:rerun-if-changed={head_path}");
    }
    if let Some(reference) = git_output(&["symbolic-ref", "-q", "HEAD"]) {
        if let Some(reference_path) = git_output(&["rev-parse", "--git-path", &reference]) {
            println!("cargo:rerun-if-changed={reference_path}");
        }
    }
    let explicit = env::var("ROUTIIUM_BUILD_REVISION")
        .ok()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| valid_revision(value));
    let revision = explicit.or_else(|| {
        git_output(&["rev-parse", "HEAD"])
            .map(|value| value.to_ascii_lowercase())
            .filter(|value| valid_revision(value))
    });
    println!(
        "cargo:rustc-env=ROUTIIUM_BUILD_REVISION={}",
        revision.as_deref().unwrap_or("unknown")
    );
}
