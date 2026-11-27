//! Sondes de `doctor` : collectent les faits de l'environnement, sans décider
//! (la décision est dans `doctor.rs`). Strictement en lecture : variables
//! d'environnement, `/proc`, `/tmp/.X11-unix`, `PATH`, connexions TCP/HTTP GET
//! vers les URL données, et une introspection D-Bus par `busctl`. Rien n'est
//! lancé, créé, ni écrit (ADR-0032).

use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::cli::{Args, TargetMode};
use crate::doctor::{
    host_port, wants_cdp, wants_sdk, CdpView, EisProbe, KwinProbe, Probes, SdkView, ServiceProbe,
    X11Probe,
};

/// Délai de chaque sonde réseau ou processus : `doctor` doit répondre vite,
/// même devant un port qui avale les connexions.
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

const KWIN_PROCESSES: [&str; 2] = ["kwin_wayland", "kwin_x11"];
const KWIN_VARIABLE: &str = "KWIN_SCREENSHOT_NO_PERMISSION_CHECKS";
const EIS_PATH: &str = "/org/kde/KWin/EIS/RemoteDesktop";

/// Collecte tous les faits utiles à `target`.
pub(crate) async fn gather(args: &Args) -> Probes {
    let target = args.target;
    let kwin = probe_kwin();
    let eis = if target == TargetMode::KwinEis && matches!(kwin, KwinProbe::Running { .. }) {
        probe_eis().await
    } else {
        // Sans objet ou sans KWin : la décision n'utilisera pas ce fait.
        EisProbe::Unknown("non sondé".to_string())
    };
    let session_type = env_var("XDG_SESSION_TYPE");
    let desktop = env_var("XDG_CURRENT_DESKTOP");
    // Sondes réseau en parallèle, et seulement celles que la cible utilise :
    // chacune peut attendre jusqu'à son délai complet.
    let (sdk, cdp) = tokio::join!(
        async {
            if wants_sdk(target) {
                probe_sdk(&args.sdk_url).await
            } else {
                not_probed(&args.sdk_url)
            }
        },
        async {
            if wants_cdp(target) {
                probe_cdp(args.cdp_url()).await
            } else {
                not_probed(args.cdp_url())
            }
        }
    );
    Probes {
        session_type,
        desktop,
        kwin,
        eis,
        xvfb_installed: in_path("Xvfb"),
        x11: args.display.as_deref().map(probe_x11),
        sdk,
        cdp,
        supported_protocol: deveye_sdk_client::SUPPORTED_PROTOCOL_VERSION.to_string(),
    }
}

/// Service que la cible n'utilise pas : la décision le marque « sans objet ».
fn not_probed<T>(url: &str) -> ServiceProbe<T> {
    ServiceProbe {
        url: url.to_string(),
        tcp_open: false,
        answer: Err("non sondé (sans objet pour cette cible)".to_string()),
    }
}

fn env_var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// Exécutable présent dans un dossier du `PATH` (recherche pure, rien n'est lancé).
fn in_path(binary: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|dir| is_executable(&dir.join(binary)))
    })
}

fn is_executable(path: &Path) -> bool {
    path.metadata()
        .map(|m| m.is_file() && m.mode() & 0o111 != 0)
        .unwrap_or(false)
}

// ── KWin ──────────────────────────────────────────────────────────────────────

fn my_uid() -> Option<u32> {
    std::fs::metadata("/proc/self").ok().map(|m| m.uid())
}

/// Cherche un processus KWin du même utilisateur dans `/proc` et lit la
/// variable de capture dans son environnement.
fn probe_kwin() -> KwinProbe {
    let binary_installed = KWIN_PROCESSES.iter().any(|b| in_path(b));
    let (Some(uid), Ok(entries)) = (my_uid(), std::fs::read_dir("/proc")) else {
        return KwinProbe::NotRunning { binary_installed };
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if !name.to_string_lossy().bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let dir = entry.path();
        let is_kwin = std::fs::read_to_string(dir.join("comm"))
            .map(|c| KWIN_PROCESSES.contains(&c.trim()))
            .unwrap_or(false);
        if !is_kwin || dir.metadata().map(|m| m.uid()).ok() != Some(uid) {
            continue;
        }
        // `environ` de KWin est souvent protégé (privilèges du compositeur) :
        // repli sur notre propre environnement, hérité de la même session ;
        // une absence chez nous ne prouve rien, d'où `None`.
        let own = std::env::var(KWIN_VARIABLE).is_ok_and(|v| !v.is_empty() && v != "0");
        let no_permission_checks = match std::fs::read(dir.join("environ")) {
            Ok(bytes) => Some(environ_enables(&bytes, KWIN_VARIABLE) || own),
            Err(_) => own.then_some(true),
        };
        return KwinProbe::Running {
            no_permission_checks,
        };
    }
    KwinProbe::NotRunning { binary_installed }
}

/// `true` si `variable` est posée à une valeur non vide et différente de `0`
/// dans un bloc `environ` (paires `NOM=valeur` séparées par des NUL).
pub(crate) fn environ_enables(environ: &[u8], variable: &str) -> bool {
    environ
        .split(|b| *b == 0)
        .filter_map(|pair| std::str::from_utf8(pair).ok())
        .filter_map(|pair| pair.split_once('='))
        .any(|(name, value)| name == variable && !value.is_empty() && value != "0")
}

/// Introspection D-Bus de l'interface EIS, par `busctl` : un appel
/// `Introspect`, sans `connectToEIS` (qui, lui, déclencherait une connexion).
async fn probe_eis() -> EisProbe {
    if !in_path("busctl") {
        return EisProbe::Unknown("busctl absent du PATH".to_string());
    }
    let run = tokio::process::Command::new("busctl")
        .args([
            "--user",
            "--no-pager",
            "introspect",
            "org.kde.KWin",
            EIS_PATH,
        ])
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true)
        .output();
    match tokio::time::timeout(PROBE_TIMEOUT, run).await {
        Err(_) => EisProbe::Unknown("délai dépassé".to_string()),
        Ok(Err(e)) => EisProbe::Unknown(format!("busctl non exécutable : {e}")),
        Ok(Ok(out)) if out.status.success() => {
            if String::from_utf8_lossy(&out.stdout).contains("connectToEIS") {
                EisProbe::Available
            } else {
                EisProbe::Missing
            }
        }
        Ok(Ok(_)) => EisProbe::Missing,
    }
}

// ── Serveur X privé ───────────────────────────────────────────────────────────

fn probe_x11(raw: &str) -> X11Probe {
    let socket = |number: u32| PathBuf::from(format!("/tmp/.X11-unix/X{number}"));
    let owner = |number: u32| socket(number).metadata().ok().map(|m| m.uid());
    let guard = match my_uid() {
        None => Err("uid du processus illisible".to_string()),
        Some(uid) => {
            adapter_x11::check_display(raw, std::env::var("DISPLAY").ok().as_deref(), owner, uid)
                .map(|id| id.to_string())
                .map_err(|e| e.to_string())
        }
    };
    let socket_exists = adapter_x11::parse_display(raw)
        .map(|id| socket(id.number).exists())
        .unwrap_or(false);
    X11Probe {
        display: raw.to_string(),
        guard,
        socket_exists,
    }
}

// ── Services HTTP locaux ──────────────────────────────────────────────────────

/// Le port accepte-t-il une connexion TCP ?
async fn tcp_open(url: &str) -> bool {
    let Some((host, port)) = host_port(url) else {
        return false;
    };
    let host = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_string();
    matches!(
        tokio::time::timeout(PROBE_TIMEOUT, tokio::net::TcpStream::connect((host, port))).await,
        Ok(Ok(_))
    )
}

/// Lit `/devtools/info` **sans** contrôler la version du protocole : c'est à
/// `doctor` de la juger (`decide_sdk`). `deveye_sdk_client::fetch_info` refuse
/// une majeure différente avant tout décodage, ce qui ferait passer un SDK
/// incompatible pour « un autre service sur ce port ».
async fn fetch_sdk_info(
    http: &reqwest::Client,
    base_url: &str,
) -> Result<deveye_sdk_client::SdkInfo, String> {
    let url = format!("{}/devtools/info", base_url.trim_end_matches('/'));
    let response = http
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("GET {url} : {e}"))?;
    let status = response.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        return Err(format!(
            "GET {url} : route absente, SDK antérieur au protocole 1.0 ; mettre deveye-sdk à jour"
        ));
    }
    if !status.is_success() {
        return Err(format!("GET {url} : statut {status}"));
    }
    response
        .json()
        .await
        .map_err(|e| format!("décodage JSON de {url} : {e}"))
}

/// `GET /devtools/info` du SDK.
async fn probe_sdk(url: &str) -> ServiceProbe<SdkView> {
    let tcp_open = tcp_open(url).await;
    let answer = if tcp_open {
        let http = deveye_sdk_client::client();
        match tokio::time::timeout(PROBE_TIMEOUT, fetch_sdk_info(&http, url)).await {
            Ok(Ok(info)) => Ok(SdkView {
                sdk_version: info.sdk_version,
                protocol_version: info.protocol_version,
                capabilities: info.capabilities,
            }),
            Ok(Err(e)) => Err(e.to_string()),
            Err(_) => Err("délai dépassé".to_string()),
        }
    } else {
        Err("connexion refusée ou délai dépassé".to_string())
    };
    ServiceProbe {
        url: url.to_string(),
        tcp_open,
        answer,
    }
}

/// `GET /json/version` du navigateur CDP.
async fn probe_cdp(url: &str) -> ServiceProbe<CdpView> {
    let tcp_open = tcp_open(url).await;
    let answer = if tcp_open {
        fetch_cdp_version(url).await
    } else {
        Err("connexion refusée ou délai dépassé".to_string())
    };
    ServiceProbe {
        url: url.to_string(),
        tcp_open,
        answer,
    }
}

async fn fetch_cdp_version(url: &str) -> Result<CdpView, String> {
    let http = reqwest::Client::builder()
        .timeout(PROBE_TIMEOUT)
        .build()
        .map_err(|e| e.to_string())?;
    let response = http
        .get(format!("{}/json/version", url.trim_end_matches('/')))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status()));
    }
    let body: serde_json::Value = response.json().await.map_err(|e| e.to_string())?;
    let field = |key: &str| body.get(key).and_then(|v| v.as_str()).map(str::to_string);
    match (field("Browser"), field("Protocol-Version")) {
        (Some(browser), protocol) => Ok(CdpView {
            browser,
            protocol: protocol.unwrap_or_else(|| "inconnu".to_string()),
        }),
        _ => Err("réponse sans champ Browser".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn environ_parsing_handles_nul_separated_pairs() {
        let env = b"HOME=/home/x\0KWIN_SCREENSHOT_NO_PERMISSION_CHECKS=1\0PATH=/usr/bin\0";
        assert!(environ_enables(env, KWIN_VARIABLE));
        assert!(!environ_enables(b"HOME=/home/x\0", KWIN_VARIABLE));
        assert!(!environ_enables(
            b"KWIN_SCREENSHOT_NO_PERMISSION_CHECKS=0\0",
            KWIN_VARIABLE
        ));
        assert!(!environ_enables(
            b"KWIN_SCREENSHOT_NO_PERMISSION_CHECKS=\0",
            KWIN_VARIABLE
        ));
        // Un nom qui contient simplement la variable ne compte pas.
        assert!(!environ_enables(
            b"X_KWIN_SCREENSHOT_NO_PERMISSION_CHECKS=1\0",
            KWIN_VARIABLE
        ));
    }

    #[tokio::test]
    async fn a_closed_port_is_reported_closed() {
        // Port 1 : jamais ouvert sur une machine de dev ; l'URL invalide aussi.
        assert!(!tcp_open("http://127.0.0.1:1").await);
        assert!(!tcp_open("http://").await);
    }

    #[tokio::test]
    async fn an_sdk_announcing_another_major_is_read_not_rejected() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            // La sonde TCP ouvre d'abord une connexion muette : on boucle.
            while let Ok((mut socket, _)) = listener.accept().await {
                let mut buf = [0u8; 1024];
                if socket.read(&mut buf).await.unwrap_or(0) == 0 {
                    continue;
                }
                let body =
                    r#"{"sdk_version":"2.0.0","protocol_version":"2.0","capabilities":["state"]}"#;
                let reply = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\nx-deveye-protocol-version: 2.0\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(reply.as_bytes()).await;
            }
        });
        let probe = probe_sdk(&format!("http://127.0.0.1:{port}")).await;
        let view = probe.answer.expect("la sonde doit lire l'info du SDK");
        assert_eq!(view.protocol_version, "2.0");
        assert_eq!(
            crate::doctor::protocol_compat(&view.protocol_version, "1.0"),
            crate::doctor::Compat::MajorDiffers
        );
    }
}
