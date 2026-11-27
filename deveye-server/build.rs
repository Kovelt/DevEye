//! Provenance du commit affichée par `--version` (ADR-0033), sans dépendance :
//! variable d'environnement `DEVEYE_GIT_COMMIT` si elle est posée (builds de
//! release, CI), sinon `git rev-parse` s'il est présent et que l'on est dans
//! un dépôt, sinon chaîne vide (archive source, crate publié).

use std::path::{Path, PathBuf};
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

/// Dépôt git qui contient ce workspace : sortie de `git rev-parse` rendue en
/// chemins absolus. `None` hors dépôt, sans `git`, ou si le dépôt trouvé est
/// celui d'un autre projet qui vendorise ce crate (le workspace DevEye porte
/// `deveye-server/Cargo.toml` à la racine du dépôt ; sinon, le commit affiché
/// serait celui du projet hôte).
struct Repo {
    git_dir: PathBuf,
    common_dir: PathBuf,
}

fn git_output(args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (!text.is_empty()).then_some(text)
}

fn find_repo() -> Option<Repo> {
    let top = PathBuf::from(git_output(&["rev-parse", "--show-toplevel"])?);
    if !top.join("deveye-server").join("Cargo.toml").is_file() {
        return None;
    }
    // Dans un worktree, `.git` est un fichier : `--absolute-git-dir` donne le
    // dossier propre au worktree (HEAD) et `--git-common-dir` celui des refs.
    let git_dir = PathBuf::from(git_output(&["rev-parse", "--absolute-git-dir"])?);
    let common = PathBuf::from(git_output(&["rev-parse", "--git-common-dir"])?);
    let common_dir = if common.is_absolute() {
        common
    } else {
        Path::new(env!("CARGO_MANIFEST_DIR")).join(common)
    };
    Some(Repo {
        git_dir,
        common_dir,
    })
}

/// Commit court du dépôt courant, `None` hors dépôt ou sans `git`.
fn git_short_commit() -> Option<String> {
    find_repo()?;
    git_output(&["rev-parse", "--short=9", "HEAD"])
}

/// Relance le script quand HEAD, la branche courante ou `packed-refs` bougent :
/// sans cela, le commit affiché resterait celui de la première compilation.
/// Une ref absente (branche seulement packée) est tout de même déclarée : cargo
/// relance alors à sa création.
fn watch_git_head() {
    let Some(repo) = find_repo() else {
        return;
    };
    let head = repo.git_dir.join("HEAD");
    println!("cargo:rerun-if-changed={}", head.display());
    println!(
        "cargo:rerun-if-changed={}",
        repo.common_dir.join("packed-refs").display()
    );
    if let Ok(content) = std::fs::read_to_string(&head) {
        if let Some(reference) = content.trim().strip_prefix("ref: ") {
            println!(
                "cargo:rerun-if-changed={}",
                repo.common_dir.join(reference).display()
            );
        }
    }
}
