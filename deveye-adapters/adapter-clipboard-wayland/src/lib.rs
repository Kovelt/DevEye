//! Adapter DevEye : presse-papiers de session **Wayland** via les outils
//! `wl-clipboard` (`wl-copy` / `wl-paste`).
//!
//! Pourquoi un process externe plutôt qu'une crate Wayland : le presse-papiers
//! Wayland exige qu'un client *reste vivant* pour servir la donnée copiée.
//! `wl-copy` gère exactement ça (il fork un démon qui sert la sélection jusqu'à
//! son remplacement). Réimplémenter ce protocole dans DevEye serait disproportionné
//! pour un outil dev-only — on délègue à l'outil de référence, déjà présent sur
//! la machine.

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]

use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use deveye_ports::{Clipboard, PortError, PortResult};

/// Délai max toléré pour `wl-paste`/`wl-copy` : un daemon wl-copy figé (ou un
/// compositeur muet) tiendrait sinon le gate MCP indéfiniment (audit R1).
const WL_CLIPBOARD_TIMEOUT: Duration = Duration::from_secs(5);

/// Presse-papiers Wayland via `wl-copy` / `wl-paste`.
#[derive(Clone, Debug)]
pub struct WaylandClipboard {
    // Noms/chemins des binaires invoqués. Configurable uniquement pour les
    // tests (pointer vers un script factice) — en production toujours
    // "wl-copy"/"wl-paste", résolus via le PATH hérité du process courant.
    copy_bin: String,
    paste_bin: String,
}

impl WaylandClipboard {
    pub fn new() -> Self {
        Self {
            copy_bin: "wl-copy".to_string(),
            paste_bin: "wl-paste".to_string(),
        }
    }

    #[cfg(test)]
    fn with_binaries(copy_bin: impl Into<String>, paste_bin: impl Into<String>) -> Self {
        Self {
            copy_bin: copy_bin.into(),
            paste_bin: paste_bin.into(),
        }
    }
}

impl Default for WaylandClipboard {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Clipboard for WaylandClipboard {
    async fn get(&self) -> PortResult<String> {
        // `--no-newline` : ne pas ajouter de \n final (wl-paste en ajoute un).
        // On garde `child` (spawn plutôt que `.output()`) pour pouvoir le
        // tuer si le timeout expire — `.output()` consomme le process et ne
        // laisse aucune prise dessus en cas de blocage.
        let mut child = tokio::process::Command::new(&self.paste_bin)
            .arg("--no-newline")
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| {
                PortError::Unsupported(format!(
                    "wl-paste indisponible (wl-clipboard installé ? session Wayland ?) : {e}"
                ))
            })?;
        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| PortError::Unsupported("wl-paste : stdout indisponible".to_string()))?;

        let mut buf = Vec::new();
        let wait = tokio::time::timeout(WL_CLIPBOARD_TIMEOUT, async {
            stdout.read_to_end(&mut buf).await?;
            child.wait().await
        })
        .await;

        match wait {
            Ok(Ok(status)) if status.success() => Ok(String::from_utf8_lossy(&buf).to_string()),
            // Presse-papiers vide : wl-paste sort en code non nul (« Nothing is
            // copied »). Ce n'est pas une erreur — on renvoie une chaîne vide.
            Ok(Ok(_)) => Ok(String::new()),
            Ok(Err(e)) => Err(PortError::Unsupported(format!("wl-paste : attente : {e}"))),
            Err(_) => {
                // start_kill() est non bloquant ; tokio récupère (reap) le
                // process tué via sa file d'orphelins interne, sans wait() de plus.
                let _ = child.start_kill();
                tracing::warn!(
                    timeout_s = WL_CLIPBOARD_TIMEOUT.as_secs(),
                    "wl-paste : délai dépassé, process tué (compositeur muet ?)"
                );
                Err(PortError::Timeout(format!(
                    "wl-paste : délai dépassé ({} s), process tué",
                    WL_CLIPBOARD_TIMEOUT.as_secs()
                )))
            }
        }
    }

    async fn set(&self, text: &str) -> PortResult<()> {
        // wl-copy lit stdin puis fork un démon qui sert la sélection — mais il
        // ne touche à stdin qu'APRÈS sa négociation Wayland (connect +
        // roundtrip). Un compositeur muet bloque donc `write_all` dès que le
        // texte dépasse la capacité du tube (64 Ko) : toute la séquence
        // écriture + fermeture + attente doit tenir dans le MÊME timeout.
        let mut child = tokio::process::Command::new(&self.copy_bin)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| {
                PortError::Unsupported(format!(
                    "wl-copy indisponible (wl-clipboard installé ? session Wayland ?) : {e}"
                ))
            })?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| PortError::Interaction("wl-copy : stdin indisponible".to_string()))?;
        let io =
            tokio::time::timeout(WL_CLIPBOARD_TIMEOUT, async {
                stdin.write_all(text.as_bytes()).await.map_err(|e| {
                    PortError::Interaction(format!("wl-copy : écriture stdin : {e}"))
                })?;
                stdin.shutdown().await.map_err(|e| {
                    PortError::Interaction(format!("wl-copy : fermeture stdin : {e}"))
                })?;
                // EOF explicite : wl-copy ne se termine qu'une fois stdin fermé.
                drop(stdin);
                child
                    .wait()
                    .await
                    .map_err(|e| PortError::Interaction(format!("wl-copy : attente : {e}")))
            })
            .await;
        match io {
            Ok(Ok(status)) if status.success() => Ok(()),
            Ok(Ok(status)) => Err(PortError::Interaction(format!(
                "wl-copy a échoué (code {:?})",
                status.code()
            ))),
            Ok(Err(e)) => Err(e),
            Err(_) => {
                // start_kill() est non bloquant ; tokio récupère (reap) le
                // process tué via sa file d'orphelins interne, sans wait() de plus.
                let _ = child.start_kill();
                tracing::warn!(
                    timeout_s = WL_CLIPBOARD_TIMEOUT.as_secs(),
                    "wl-copy : délai dépassé, process tué (compositeur muet ?)"
                );
                Err(PortError::Timeout(format!(
                    "wl-copy : délai dépassé ({} s), process tué",
                    WL_CLIPBOARD_TIMEOUT.as_secs()
                )))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};

    use super::*;

    /// Sérialise les tests qui écrivent PUIS exécutent un script factice.
    /// Sans ça, un test qui `exec()` son script pendant qu'un autre test
    /// `fork()` (spawn) hérite transitoirement du fd d'écriture (fenêtre
    /// fork→exec) déclenche `ETXTBSY` (~40 % d'échecs observés en parallèle,
    /// revue Q6). `tokio::sync::Mutex` (pas `std`) : son garde se tient à
    /// travers les `.await` des tests sans déclencher `await_holding_lock`, et
    /// ne s'empoisonne pas sur panic.
    static EXEC_SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    /// Écrit un script shell exécutable `body` sous `dir/name`, renvoie son
    /// chemin absolu. Un chemin contenant un `/` court-circuite la recherche
    /// PATH (std/tokio n'y a recours que pour un nom "nu").
    fn write_script(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("ecriture script factice");
        let mut perms = std::fs::metadata(&path)
            .expect("metadata script factice")
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).expect("chmod +x script factice");
        path
    }

    #[tokio::test]
    async fn get_renvoie_chaine_vide_si_code_non_nul() {
        let _serial = EXEC_SERIAL.lock().await;
        let dir = tempfile::tempdir().expect("tempdir");
        // Simule `wl-paste` sortant en erreur "Nothing is copied" : pas une
        // erreur cote adapter, presse-papiers vide.
        let paste = write_script(dir.path(), "wl-paste-fake", "exit 1");
        let clipboard =
            WaylandClipboard::with_binaries("wl-copy-inutilise", paste.to_str().unwrap());

        let result = clipboard
            .get()
            .await
            .expect("code non nul => chaine vide, pas une erreur");
        assert_eq!(result, "");
    }

    #[tokio::test]
    async fn get_renvoie_lerreur_attendue_si_binaire_absent() {
        let _serial = EXEC_SERIAL.lock().await;
        // Nom volontairement farfelu (jamais installe sur aucune machine) :
        // equivalent a un PATH isole qui ne contient pas le binaire, sans
        // manipuler la variable d'environnement globale du process de test.
        let clipboard = WaylandClipboard::with_binaries(
            "wl-copy-inexistant-deveye-test",
            "wl-paste-inexistant-deveye-test",
        );

        let err = clipboard.get().await.expect_err("binaire absent => erreur");
        assert!(
            matches!(err, PortError::Unsupported(_)),
            "attendu PortError::Unsupported, obtenu {err:?}"
        );
    }

    #[tokio::test]
    async fn set_transmet_le_texte_sur_stdin() {
        let _serial = EXEC_SERIAL.lock().await;
        let dir = tempfile::tempdir().expect("tempdir");
        let out_file = dir.path().join("stdin_capture.txt");
        // Recopie tout stdin dans un fichier temoin, puis reussit.
        let copy = write_script(
            dir.path(),
            "wl-copy-fake",
            &format!("cat > '{}'\nexit 0", out_file.display()),
        );
        let clipboard =
            WaylandClipboard::with_binaries(copy.to_str().unwrap(), "wl-paste-inutilise");

        clipboard
            .set("bonjour le presse-papiers")
            .await
            .expect("set doit reussir");

        let captured = std::fs::read_to_string(&out_file).expect("lecture stdin capture");
        assert_eq!(captured, "bonjour le presse-papiers");
    }

    #[tokio::test]
    async fn set_renvoie_lerreur_attendue_si_code_non_nul() {
        let _serial = EXEC_SERIAL.lock().await;
        let dir = tempfile::tempdir().expect("tempdir");
        // Consomme stdin (evite un broken pipe cote ecriture) puis echoue.
        let copy = write_script(dir.path(), "wl-copy-fake-fail", "cat > /dev/null\nexit 3");
        let clipboard =
            WaylandClipboard::with_binaries(copy.to_str().unwrap(), "wl-paste-inutilise");

        let err = clipboard
            .set("peu importe")
            .await
            .expect_err("code non nul => erreur");
        assert!(
            matches!(err, PortError::Interaction(_)),
            "attendu PortError::Interaction, obtenu {err:?}"
        );
    }
}
