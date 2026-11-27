//! Provenance du commit affichée par `--version` (ADR-0033), sans dépendance :
//! variable d'environnement `DEVEYE_GIT_COMMIT` si elle est posée (builds de
//! release, CI), sinon `git rev-parse` s'il est présent et que l'on est dans
//! un dépôt, sinon chaîne vide (archive source, crate publié).

use std::path::Path;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=DEVEYE_GIT_COMMIT");
    watch_git_head();

    let commit = std::env::var("DEVEYE_GIT_COMMIT")
        .ok()
        .map(|c| c.trim().to_string())
        .filter(|c| !c.is_empty())
        .or_else(git_short_commit)
        .unwrap_or_default();
    println!("cargo:rustc-env=DEVEYE_BUILD_COMMIT={commit}");
}

/// Commit court du dépôt courant, `None` hors dépôt ou sans `git`.
fn git_short_commit() -> Option<String> {
    let out = Command::new("git")
        .args(["rev-parse", "--short=9", "HEAD"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let commit = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (!commit.is_empty()).then_some(commit)
}

/// Relance le script quand HEAD ou la branche courante bougent : sans cela, le
/// commit affiché resterait celui de la première compilation.
fn watch_git_head() {
    let git = Path::new("../.git");
    let head = git.join("HEAD");
    if !head.exists() {
        return;
    }
    println!("cargo:rerun-if-changed={}", head.display());
    if let Ok(content) = std::fs::read_to_string(&head) {
        if let Some(reference) = content.trim().strip_prefix("ref: ") {
            let path = git.join(reference);
            if path.exists() {
                println!("cargo:rerun-if-changed={}", path.display());
            }
        }
    }
}
