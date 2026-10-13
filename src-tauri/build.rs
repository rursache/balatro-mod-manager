use std::path::{Path, PathBuf};
use std::process::Command;

#[path = "src/app_version.rs"]
mod app_version;

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?.trim().to_string();
    (!text.is_empty()).then_some(text)
}

fn watch(path: &Path) {
    // Cargo reruns forever when a watched path is missing, so only watch existing ones
    if path.exists() {
        println!("cargo:rerun-if-changed={}", path.display());
    }
}

/// Watches what moves when HEAD changes or tags are added, returns false outside this repo
fn watch_git_state(manifest_dir: &Path) -> bool {
    let workspace_root = manifest_dir.parent().unwrap_or(manifest_dir);
    let Some(top) = git(manifest_dir, &["rev-parse", "--show-toplevel"]) else {
        return false;
    };
    // Ignore an unrelated parent repository, e.g. sources unpacked inside another checkout
    let same_repo = match (
        Path::new(&top).canonicalize(),
        workspace_root.canonicalize(),
    ) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    };
    if !same_repo {
        return false;
    }

    let resolve_dir = |flag: &str| {
        git(manifest_dir, &["rev-parse", flag]).map(|p| {
            let p = PathBuf::from(p);
            if p.is_absolute() {
                p
            } else {
                manifest_dir.join(p)
            }
        })
    };
    let (Some(git_dir), Some(common_dir)) = (
        resolve_dir("--absolute-git-dir"),
        resolve_dir("--git-common-dir"),
    ) else {
        return true;
    };

    let head = git_dir.join("HEAD");
    watch(&head);
    if let Ok(contents) = std::fs::read_to_string(&head)
        && let Some(reference) = contents.trim().strip_prefix("ref: ")
    {
        watch(&common_dir.join(reference));
    }
    watch(&common_dir.join("packed-refs"));
    watch(&common_dir.join("refs").join("tags"));
    true
}

fn resolve_app_version() -> String {
    println!("cargo:rerun-if-env-changed=BMM_VERSION");
    let env_override = std::env::var("BMM_VERSION").ok();
    let manifest_dir =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));

    let (tag, hash) = if watch_git_state(&manifest_dir) {
        (
            git(
                &manifest_dir,
                &["describe", "--tags", "--exact-match", "HEAD"],
            ),
            git(&manifest_dir, &["rev-parse", "--short", "HEAD"]),
        )
    } else {
        (None, None)
    };

    app_version::resolve(env_override.as_deref(), tag.as_deref(), hash.as_deref())
}

fn main() {
    let version = resolve_app_version();
    println!("cargo:rustc-env=BMM_VERSION={version}");
    tauri_build::build();
}
