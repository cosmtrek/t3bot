//! Version string: release builds (T3BOT_RELEASE set, as in the release workflow) are the
//! plain Cargo version; anything else is `<version>-dev+<commit>[.dirty]`.

use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn main() {
    println!("cargo:rerun-if-env-changed=T3BOT_RELEASE");
    println!("cargo:rerun-if-changed=src");
    if let Some(dir) = git(&["rev-parse", "--git-dir"]) {
        for f in ["HEAD", "index", "refs"] {
            println!("cargo:rerun-if-changed={dir}/{f}");
        }
    }
    let pkg = std::env::var("CARGO_PKG_VERSION").unwrap();
    let version = if std::env::var_os("T3BOT_RELEASE").is_some() {
        pkg
    } else {
        match git(&["rev-parse", "--short", "HEAD"]) {
            Some(sha) => {
                let dirty = git(&["status", "--porcelain", "--untracked-files=no"]).is_some_and(|s| !s.is_empty());
                format!("{pkg}-dev+{sha}{}", if dirty { ".dirty" } else { "" })
            }
            None => format!("{pkg}-dev"),
        }
    };
    println!("cargo:rustc-env=T3BOT_VERSION={version}");
}
