//! Les vérifications elles-mêmes. Chacune observe l'implémentation par HTTP et
//! rend un [`Outcome`] ; aucune ne suppose autre chose que `docs/wire-protocol.md`.

use serde_json::{json, Value};

use crate::client::{body_size, Resp, Target};
use crate::report::Outcome;
use crate::Options;

type Check = Result<(), String>;

/// Corps JSON de `POST /devtools/command` au-delà de la limite de 1 Mio.
const BODY_LIMIT: usize = 1024 * 1024;

/// Valeur de remplacement des données masquées.
const REDACTED: &str = "<secret>";

/// Hôtes que la garde doit accepter (le port est celui de la cible).
fn accepted_hosts(port: u16) -> Vec<String> {
    vec![
        "localhost".into(),
        format!("localhost:{port}"),
        "127.0.0.1".into(),
        format!("127.0.0.1:{port}"),
        format!("LocalHost:{port}"),
        "[::1]".into(),
        format!("[::1]:{port}"),
    ]
}

/// Hôtes que la garde doit rejeter (13 cas ; l'absence d'en-tête est testée à part).
fn rejected_hosts(port: u16) -> Vec<String> {
    vec![
        "evil.com".into(),
        format!("evil.com:{port}"),
        "127.0.0.1.evil.com".into(),
        "localhost.evil.com".into(),
        "sub.localhost".into(),
        "0.0.0.0".into(),
        "::1".into(),
        "[::1]toto".into(),
        "[::1]evil.com".into(),
        "[::1]:abc".into(),
        "[::1]:".into(),
        "127.0.0.1:abc".into(),
        String::new(),
    ]
}

pub(crate) fn verdict(result: Check) -> Outcome {
    match result {
        Ok(()) => Outcome::Pass,
        Err(why) => Outcome::Fail(why),
    }
}

fn status_is(resp: &Resp, expected: u16) -> Check {
    if resp.status == expected {
        Ok(())
    } else {
        Err(format!(
            "statut {} au lieu de {expected} (corps : {})",
            resp.status,
            body_size(&resp.body)
        ))
    }
}

/// Erreur client de requête malformée : 4xx hors 403, 404 et 405, qui ont un
/// autre sens (garde, route, méthode).
fn is_malformed_request(status: u16) -> bool {
    (400..500).contains(&status) && !(403..=405).contains(&status)
}

/// En-tête de version présent, de forme `MAJEURE.MINEURE`, majeure `1`.
fn version_of(resp: &Resp) -> Result<String, String> {
    let value = resp
        .header("x-deveye-protocol-version")
        .ok_or("en-tête X-DevEye-Protocol-Version absent")?;
    let well_formed = value.split_once('.').is_some_and(|(major, minor)| {
        !major.is_empty()
            && !minor.is_empty()
            && major.bytes().all(|b| b.is_ascii_digit())
            && minor.bytes().all(|b| b.is_ascii_digit())
    });
    if !well_formed {
        return Err(format!(
            "en-tête de version {value:?} : forme MAJEURE.MINEURE attendue"
        ));
    }
    if !value.starts_with("1.") {
        return Err(format!(
            "majeure {value:?} : cette suite valide le protocole 1.x"
        ));
    }
    Ok(value.to_string())
}

fn with_version(resp: &Resp) -> Check {
    version_of(resp).map(|_| ())
}

fn is_json_content_type(resp: &Resp) -> Check {
    match resp.header("content-type") {
        Some(ct) if ct.to_ascii_lowercase().starts_with("application/json") => Ok(()),
        other => Err(format!(
            "Content-Type {other:?} au lieu de application/json"
        )),
    }
}

fn error_message_body(resp: &Resp) -> Check {
    let body = resp.json()?;
    match body.get("error") {
        Some(Value::String(_)) => Ok(()),
        _ => Err(format!(
            "corps {} : objet avec un champ \"error\" (chaîne) attendu",
            body_size(&resp.body)
        )),
    }
}

fn get_ok(result: &Result<Resp, String>) -> Result<&Resp, String> {
    result.as_ref().map_err(Clone::clone)
}

/// Réponses des routes de lecture, récupérées une fois pour plusieurs vérifications.
pub(crate) struct Reads {
    pub(crate) state: Result<Resp, String>,
    pub(crate) info: Result<Resp, String>,
    pub(crate) commands: Result<Resp, String>,
    pub(crate) logs_first: Result<Resp, String>,
    pub(crate) logs_second: Result<Resp, String>,
}

impl Reads {
    pub(crate) async fn fetch(t: &Target) -> Self {
        Self {
            state: t.get("/devtools/state").await,
            info: t.get("/devtools/info").await,
            commands: t.get("/devtools/commands").await,
            logs_first: t.get("/devtools/logs/poll").await,
            logs_second: t.get("/devtools/logs/poll").await,
        }
    }
}

pub(crate) fn capabilities(reads: &Reads) -> Vec<String> {
    get_ok(&reads.info)
        .and_then(Resp::json)
        .ok()
        .and_then(|info| {
            info.get("capabilities").and_then(Value::as_array).map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
        })
        .unwrap_or_default()
}

// --- Routes de lecture -------------------------------------------------

pub(crate) fn state_shape(reads: &Reads) -> Check {
    let resp = get_ok(&reads.state)?;
    status_is(resp, 200)?;
    is_json_content_type(resp)?;
    let body = resp.json()?;
    match body.get("entries") {
        Some(Value::Object(_)) => Ok(()),
        _ => Err("corps 200 : champ \"entries\" (objet) attendu".into()),
    }
}

pub(crate) fn info_shape(reads: &Reads) -> Check {
    let resp = get_ok(&reads.info)?;
    status_is(resp, 200)?;
    is_json_content_type(resp)?;
    let body = resp.json()?;
    let version = body
        .get("protocol_version")
        .and_then(Value::as_str)
        .ok_or("champ \"protocol_version\" (chaîne) absent")?;
    if version_of(resp)? != version {
        return Err("protocol_version diffère de l'en-tête X-DevEye-Protocol-Version".into());
    }
    match body.get("sdk_version").and_then(Value::as_str) {
        Some(v) if !v.is_empty() => {}
        _ => return Err("champ \"sdk_version\" (chaîne non vide) absent".into()),
    }
    let caps = capabilities(reads);
    for required in ["state", "commands", "info"] {
        if !caps.iter().any(|c| c == required) {
            return Err(format!("capacité obligatoire {required:?} non annoncée"));
        }
    }
    Ok(())
}

pub(crate) fn commands_shape(reads: &Reads) -> Check {
    let resp = get_ok(&reads.commands)?;
    status_is(resp, 200)?;
    is_json_content_type(resp)?;
    let body = resp.json()?;
    let list = body.as_array().ok_or("corps 200 : tableau JSON attendu")?;
    for (i, item) in list.iter().enumerate() {
        for field in ["name", "description"] {
            if !item.get(field).is_some_and(Value::is_string) {
                return Err(format!("élément {i} : champ {field:?} (chaîne) absent"));
            }
        }
    }
    Ok(())
}

pub(crate) fn commands_sorted(reads: &Reads) -> Outcome {
    let names: Vec<String> = match get_ok(&reads.commands)
        .and_then(Resp::json)
        .map(|b| b.as_array().cloned().unwrap_or_default())
    {
        Ok(list) => list
            .iter()
            .filter_map(|c| c.get("name").and_then(Value::as_str).map(str::to_string))
            .collect(),
        Err(why) => return Outcome::Fail(why),
    };
    if names.len() < 2 {
        return Outcome::Skip(format!(
            "{} commande(s) enregistrée(s) : l'ordre n'est pas observable",
            names.len()
        ));
    }
    // Ordre des octets UTF-8, comme `str::cmp` côté Rust.
    if names.windows(2).all(|w| w[0].as_bytes() <= w[1].as_bytes()) {
        Outcome::Pass
    } else {
        Outcome::Fail(format!("noms non triés : {names:?}"))
    }
}

pub(crate) fn read_routes_carry_version(reads: &Reads) -> Check {
    let mut seen: Option<String> = None;
    for (name, result) in [
        ("state", &reads.state),
        ("info", &reads.info),
        ("commands", &reads.commands),
        ("logs/poll", &reads.logs_first),
    ] {
        let resp = get_ok(result)?;
        let version = version_of(resp).map_err(|e| format!("GET /devtools/{name} : {e}"))?;
        if let Some(previous) = &seen {
            if *previous != version {
                return Err(format!(
                    "versions divergentes entre routes : {previous} et {version}"
                ));
            }
        }
        seen = Some(version);
    }
    Ok(())
}

// --- Logs ----------------------------------------------------------------

fn events_of(result: &Result<Resp, String>) -> Result<Vec<Value>, String> {
    let resp = get_ok(result)?;
    status_is(resp, 200)?;
    let body = resp.json()?;
    body.get("events")
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| "corps 200 : champ \"events\" (tableau) attendu".to_string())
}

pub(crate) fn logs_shape(reads: &Reads) -> Check {
    is_json_content_type(get_ok(&reads.logs_first)?)?;
    for (i, event) in events_of(&reads.logs_first)?.iter().enumerate() {
        let level = event.get("level").and_then(Value::as_str);
        if !matches!(level, Some("trace" | "debug" | "info" | "warn" | "error")) {
            return Err(format!(
                "événement {i} : level {level:?} hors trace/debug/info/warn/error"
            ));
        }
        for field in ["target", "message", "timestamp"] {
            if !event.get(field).is_some_and(Value::is_string) {
                return Err(format!("événement {i} : champ {field:?} (chaîne) absent"));
            }
        }
        if let Some(fields) = event.get("fields") {
            if !fields.is_object() && !fields.is_null() {
                return Err(format!(
                    "événement {i} : fields doit être un objet ou absent"
                ));
            }
        }
    }
    Ok(())
}

fn parse_timestamp(text: &str) -> Result<chrono::DateTime<chrono::FixedOffset>, String> {
    if !text.ends_with('Z') {
        return Err(format!("horodatage {text:?} : suffixe Z (UTC) attendu"));
    }
    chrono::DateTime::parse_from_rfc3339(text)
        .map_err(|e| format!("horodatage {text:?} non conforme RFC 3339 : {e}"))
}

pub(crate) fn logs_timestamps(reads: &Reads) -> Outcome {
    let events = match events_of(&reads.logs_first) {
        Ok(events) => events,
        Err(why) => return Outcome::Fail(why),
    };
    if events.is_empty() {
        return Outcome::Skip("aucun événement de log à observer".into());
    }
    for event in &events {
        let text = event.get("timestamp").and_then(Value::as_str).unwrap_or("");
        if let Err(why) = parse_timestamp(text) {
            return Outcome::Fail(why);
        }
    }
    Outcome::Pass
}

pub(crate) fn logs_order(reads: &Reads) -> Outcome {
    let events = match events_of(&reads.logs_first) {
        Ok(events) => events,
        Err(why) => return Outcome::Fail(why),
    };
    let stamps: Vec<_> = events
        .iter()
        .filter_map(|e| e.get("timestamp").and_then(Value::as_str))
        .filter_map(|t| parse_timestamp(t).ok())
        .collect();
    if stamps.len() < 2 {
        return Outcome::Skip("moins de 2 événements : l'ordre n'est pas observable".into());
    }
    if stamps.windows(2).all(|w| w[0] <= w[1]) {
        Outcome::Pass
    } else {
        Outcome::Fail("événements non rendus par horodatage croissant (ordre d'émission)".into())
    }
}

pub(crate) fn logs_drain(reads: &Reads) -> Outcome {
    let (first, second) = match (events_of(&reads.logs_first), events_of(&reads.logs_second)) {
        (Ok(first), Ok(second)) => (first, second),
        (Err(why), _) | (_, Err(why)) => return Outcome::Fail(why),
    };
    // Un premier poll vide ne prouve rien : un second vide non plus.
    if first.is_empty() {
        return Outcome::Skip(
            "aucun événement de log rendu au premier poll : le drainage n'est pas observable"
                .into(),
        );
    }
    if let Some(repeated) = second.iter().find(|e| first.contains(e)) {
        return Outcome::Fail(format!(
            "un événement déjà rendu est rendu à nouveau (le poll doit drainer) : événement de {}",
            body_size(&repeated.to_string())
        ));
    }
    Outcome::Pass
}

// --- Garde Host ---------------------------------------------------------

pub(crate) async fn host_accepted(t: &Target) -> Check {
    for host in accepted_hosts(t.port) {
        let resp = t
            .send("GET", "/devtools/info", &[("host", &host)], None)
            .await?;
        status_is(&resp, 200).map_err(|e| format!("Host {host:?} devait être accepté : {e}"))?;
    }
    Ok(())
}

pub(crate) async fn host_rejected(t: &Target) -> Check {
    for host in rejected_hosts(t.port) {
        let resp = t
            .send("GET", "/devtools/state", &[("host", &host)], None)
            .await?;
        status_is(&resp, 403).map_err(|e| format!("Host {host:?} devait être rejeté : {e}"))?;
        with_version(&resp).map_err(|e| format!("403 pour Host {host:?} : {e}"))?;
    }
    Ok(())
}

pub(crate) async fn host_absent(t: &Target) -> Check {
    let (status, raw) = t.raw_without_host("/devtools/state").await?;
    match status {
        403 if raw.contains("x-deveye-protocol-version:") => Ok(()),
        403 => Err("le 403 sans Host ne porte pas l'en-tête de version".into()),
        // RFC 9112 §3.2 impose 400 à un serveur HTTP/1.1 devant une requête
        // sans Host : certaines piles (Kestrel) le font avant tout middleware,
        // et l'application n'est alors jamais atteinte (§2.2).
        400 => Ok(()),
        other => Err(format!(
            "requête sans Host : statut {other} au lieu de 403 (ou 400 refusé par la pile HTTP)"
        )),
    }
}

pub(crate) async fn host_guard_precedes_routing(t: &Target) -> Check {
    let evil = [("host", "evil.com"), ("content-type", "application/json")];
    let resp = t
        .send("POST", "/devtools/command", &evil, Some(b"{".to_vec()))
        .await?;
    status_is(&resp, 403).map_err(|e| format!("POST invalide avec Host étranger : {e}"))?;
    error_message_body(&resp).map_err(|e| format!("corps du 403 : {e}"))?;
    let resp = t
        .send(
            "GET",
            "/devtools/inexistante",
            &[("host", "evil.com")],
            None,
        )
        .await?;
    status_is(&resp, 403).map_err(|e| format!("route inconnue avec Host étranger : {e}"))
}

// --- Erreurs communes -----------------------------------------------------

pub(crate) async fn unknown_path(t: &Target) -> Check {
    for path in ["/devtools/inexistante", "/", "/devtools/state/extra"] {
        let resp = t.get(path).await?;
        status_is(&resp, 404).map_err(|e| format!("GET {path} : {e}"))?;
        with_version(&resp).map_err(|e| format!("404 de {path} : {e}"))?;
    }
    Ok(())
}

pub(crate) async fn wrong_method(t: &Target) -> Check {
    for (method, path) in [
        ("POST", "/devtools/state"),
        ("GET", "/devtools/eval"),
        ("GET", "/devtools/command"),
        ("POST", "/devtools/commands"),
    ] {
        let resp = t.send(method, path, &[], None).await?;
        status_is(&resp, 405).map_err(|e| format!("{method} {path} : {e}"))?;
        with_version(&resp).map_err(|e| format!("405 de {method} {path} : {e}"))?;
    }
    Ok(())
}

pub(crate) async fn invalid_json(t: &Target) -> Check {
    for path in ["/devtools/command", "/devtools/eval"] {
        let resp = t.post_raw(path, b"{ pas du json").await?;
        if !is_malformed_request(resp.status) {
            return Err(format!(
                "POST {path} avec JSON invalide : statut {} au lieu d'un 4xx (400 attendu)",
                resp.status
            ));
        }
        with_version(&resp).map_err(|e| format!("erreur de {path} : {e}"))?;
    }
    Ok(())
}

pub(crate) async fn missing_required_field(t: &Target) -> Check {
    for path in ["/devtools/command", "/devtools/eval"] {
        let resp = t.post_json(path, &json!({})).await?;
        if !is_malformed_request(resp.status) {
            return Err(format!(
                "POST {path} sans champ obligatoire : statut {} au lieu d'un 4xx (422 attendu)",
                resp.status
            ));
        }
        with_version(&resp).map_err(|e| format!("erreur de {path} : {e}"))?;
    }
    Ok(())
}

pub(crate) async fn body_too_large(t: &Target) -> Outcome {
    let resp = match t
        .post_raw("/devtools/command", &vec![b'x'; BODY_LIMIT + 1])
        .await
    {
        Ok(resp) => resp,
        // §2.5 : le serveur peut couper la connexion avant d'avoir lu tout le
        // corps ; le 413 n'est alors pas observable, ce n'est pas un écart.
        Err(why) => {
            return Outcome::Skip(format!(
                "connexion coupée avant la réponse (permis par §2.5), 413 non observable : {why}"
            ))
        }
    };
    verdict(status_is(&resp, 413).and_then(|()| with_version(&resp)))
}

// --- CORS ---------------------------------------------------------------

fn no_cross_origin_grant(resp: &Resp) -> Check {
    for header in [
        "access-control-allow-origin",
        "access-control-allow-credentials",
    ] {
        if let Some(value) = resp.header(header) {
            return Err(format!("{header}: {value} émis (CORS deny-all attendu)"));
        }
    }
    Ok(())
}

pub(crate) async fn cors_preflight(t: &Target) -> Check {
    let headers = [
        ("origin", "https://evil.example"),
        ("access-control-request-method", "POST"),
        ("access-control-request-headers", "content-type"),
    ];
    let resp = t
        .send("OPTIONS", "/devtools/command", &headers, None)
        .await?;
    if resp.status >= 500 {
        return Err(format!("préflight : statut {} inattendu", resp.status));
    }
    no_cross_origin_grant(&resp)?;
    with_version(&resp).map_err(|e| format!("réponse au préflight : {e}"))
}

pub(crate) async fn cors_simple_request(t: &Target) -> Check {
    let resp = t
        .send(
            "GET",
            "/devtools/state",
            &[("origin", "https://evil.example")],
            None,
        )
        .await?;
    status_is(&resp, 200)?;
    no_cross_origin_grant(&resp)
}

// --- Commandes -----------------------------------------------------------

pub(crate) async fn unknown_command(t: &Target) -> Check {
    let absent = t
        .post_json(
            "/devtools/command",
            &json!({ "route": "deveye-conformance-inconnue" }),
        )
        .await?;
    let null = t
        .post_json(
            "/devtools/command",
            &json!({ "route": "deveye-conformance-inconnue", "payload": null }),
        )
        .await?;
    for (label, resp) in [("payload absent", &absent), ("payload null", &null)] {
        status_is(resp, 404).map_err(|e| format!("route inconnue, {label} : {e}"))?;
        with_version(resp)?;
        error_message_body(resp).map_err(|e| format!("route inconnue, {label} : {e}"))?;
    }
    Ok(())
}

pub(crate) async fn invoke_known_command(t: &Target, route: Option<&str>) -> Outcome {
    let Some(route) = route else {
        return Outcome::Skip("aucune commande connue (option --invoke-route)".into());
    };
    let result = async {
        let resp = t
            .post_json(
                "/devtools/command",
                &json!({ "route": route, "payload": null }),
            )
            .await?;
        status_is(&resp, 200)?;
        with_version(&resp)?;
        resp.json().map(|_| ())
    }
    .await;
    verdict(result)
}

// --- Évaluation JS ----------------------------------------------------------

pub(crate) async fn eval_matches_capability(t: &Target, caps: &[String]) -> Check {
    let resp = t
        .post_json("/devtools/eval", &json!({ "expression": "1 + 1" }))
        .await?;
    with_version(&resp)?;
    if caps.iter().any(|c| c == "eval") {
        status_is(&resp, 200).map_err(|e| format!("capacité eval annoncée : {e}"))?;
        let body = resp.json()?;
        let has_value = body.get("value").is_some();
        let has_error = body.get("error").is_some_and(Value::is_string);
        if has_value == has_error {
            return Err("200 : exactement un champ parmi \"value\" et \"error\" attendu".into());
        }
        Ok(())
    } else {
        status_is(&resp, 503).map_err(|e| format!("capacité eval non annoncée : {e}"))?;
        error_message_body(&resp)
    }
}

pub(crate) async fn eval_special(
    t: &Target,
    caps: &[String],
    expression: Option<&str>,
    expected: u16,
    option: &str,
) -> Outcome {
    let Some(expression) = expression else {
        return Outcome::Skip(format!("non observable sans {option}"));
    };
    if !caps.iter().any(|c| c == "eval") {
        return Outcome::NotApplicable("capacité eval non annoncée".into());
    }
    let result = async {
        let resp = t
            .post_json("/devtools/eval", &json!({ "expression": expression }))
            .await?;
        status_is(&resp, expected)?;
        with_version(&resp)?;
        error_message_body(&resp)
    }
    .await;
    verdict(result)
}

// --- Masquage des secrets ------------------------------------------------------

/// Reflet exact de la règle du §2.4 : clé sensible d'après son nom normalisé.
fn is_secret_key(key: &str) -> bool {
    let key: String = key
        .chars()
        .filter(|c| !matches!(c, '_' | '-' | '.' | ' '))
        .map(|c| c.to_ascii_lowercase())
        .collect();
    const CONTAINS: [&str; 7] = [
        "password",
        "passwd",
        "secret",
        "apikey",
        "privatekey",
        "credential",
        "authorization",
    ];
    const SUFFIXES: [&str; 3] = ["token", "cookie", "sessionid"];
    const EXACT: [&str; 5] = ["pwd", "auth", "jwt", "bearer", "otp"];
    CONTAINS.iter().any(|t| key.contains(t))
        || SUFFIXES.iter().any(|t| key.ends_with(t))
        || EXACT.contains(&key.as_str())
}

/// Parcourt `value` ; compte les clés sensibles rencontrées et note celles dont
/// la valeur n'est pas masquée (chemin seulement, jamais la valeur).
fn scan_secrets(value: &Value, path: &str, seen: &mut usize, leaks: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, inner) in map {
                let here = format!("{path}/{key}");
                if is_secret_key(key) {
                    *seen += 1;
                    if inner.as_str() != Some(REDACTED) {
                        leaks.push(here);
                    }
                } else {
                    scan_secrets(inner, &here, seen, leaks);
                }
            }
        }
        Value::Array(items) => {
            for (i, inner) in items.iter().enumerate() {
                scan_secrets(inner, &format!("{path}/{i}"), seen, leaks);
            }
        }
        _ => {}
    }
}

pub(crate) fn redaction(reads: &Reads, caps: &[String]) -> Outcome {
    if !caps.iter().any(|c| c == "redaction") {
        return Outcome::NotApplicable(
            "capacité redaction non annoncée par l'implémentation".into(),
        );
    }
    let (mut seen, mut leaks) = (0, Vec::new());
    if let Ok(body) = get_ok(&reads.state).and_then(Resp::json) {
        scan_secrets(&body, "state", &mut seen, &mut leaks);
    }
    for result in [&reads.logs_first, &reads.logs_second] {
        if let Ok(events) = events_of(result) {
            for (i, event) in events.iter().enumerate() {
                if let Some(fields) = event.get("fields") {
                    scan_secrets(fields, &format!("logs/{i}/fields"), &mut seen, &mut leaks);
                }
            }
        }
    }
    if !leaks.is_empty() {
        return Outcome::Fail(format!(
            "valeurs sensibles non masquées : {}",
            leaks.join(", ")
        ));
    }
    if seen == 0 {
        return Outcome::Skip(
            "aucune clé sensible dans l'état ni les logs : rien à vérifier (poser par exemple \
             set_state(\"password\", ...) avant la suite)"
                .into(),
        );
    }
    Outcome::Pass
}

// --- Route ui.widgets (deveye.ui/1) -------------------------------------------

/// Capacité qui engage l'implémentation à servir la route `ui.widgets`.
const UI_CAPABILITY: &str = "ui.widgets/1";

/// Lit la route `ui.widgets` avec le payload `{}` (négociation du payload non
/// engagée tant que `/devtools/info` n'annonce pas plus, ADR-0065).
async fn fetch_ui_tree(t: &Target) -> Result<Value, String> {
    let resp = t
        .post_json(
            "/devtools/command",
            &json!({ "route": "ui.widgets", "payload": {} }),
        )
        .await?;
    status_is(&resp, 200)?;
    with_version(&resp)?;
    resp.json()
}

/// Arbre conforme à `deveye.ui/1` : schéma déclaré, ids uniques, parents connus,
/// pas de cycle. Les avertissements (widget hors fenêtre) ne sont pas des écarts.
fn check_ui_tree(value: &Value) -> Check {
    if value.get("schema").and_then(Value::as_str) != Some(deveye_core::UI_SCHEMA) {
        return Err(format!(
            "champ \"schema\" : {:?} attendu",
            deveye_core::UI_SCHEMA
        ));
    }
    let tree = deveye_core::UiTree::from_value(value).map_err(|e| e.to_string())?;
    let errors: Vec<String> = tree
        .validate()
        .iter()
        .filter(|issue| issue.is_error())
        .map(ToString::to_string)
        .collect();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join(" ; "))
    }
}

/// Chemins (jamais les valeurs) des widgets secrets dont la valeur est publiée.
fn ui_secret_leaks(value: &Value) -> Result<(usize, Vec<String>), String> {
    let tree = deveye_core::UiTree::from_value(value).map_err(|e| e.to_string())?;
    let secrets: Vec<_> = tree.widgets.iter().filter(|w| w.is_secret()).collect();
    let leaks = secrets
        .iter()
        .filter(|w| {
            w.value
                .as_ref()
                .is_some_and(|v| v.as_str() != Some(REDACTED))
        })
        .map(|w| format!("widgets/{}", w.id))
        .collect();
    Ok((secrets.len(), leaks))
}

pub(crate) async fn ui_widgets(t: &Target, caps: &[String]) -> (Outcome, Outcome) {
    let not_applicable = || {
        Outcome::NotApplicable(format!(
            "capacité {UI_CAPABILITY} non annoncée par l'implémentation"
        ))
    };
    if !caps.iter().any(|c| c == UI_CAPABILITY) {
        return (not_applicable(), not_applicable());
    }
    let value = match fetch_ui_tree(t).await {
        Ok(value) => value,
        Err(why) => return (Outcome::Fail(why.clone()), Outcome::Fail(why)),
    };
    let shape = verdict(check_ui_tree(&value));
    let secrets = match ui_secret_leaks(&value) {
        Err(why) => Outcome::Fail(why),
        Ok((_, leaks)) if !leaks.is_empty() => Outcome::Fail(format!(
            "valeur publiée pour un widget secret : {}",
            leaks.join(", ")
        )),
        Ok((0, _)) => Outcome::Skip(
            "aucun widget secret dans l'arbre : rien à vérifier (publier par exemple un champ \
             de rôle password)"
                .into(),
        ),
        Ok(_) => Outcome::Pass,
    };
    (shape, secrets)
}

/// Point d'entrée commun : exécute toutes les vérifications dans l'ordre.
pub(crate) async fn run_all(t: &Target, opts: &Options) -> crate::Report {
    let mut report = crate::Report::default();
    let reads = Reads::fetch(t).await;
    let caps = capabilities(&reads);

    report.record(
        "ROUTE-01",
        "§3.1",
        "GET /devtools/state : 200, objet entries",
        verdict(state_shape(&reads)),
    );
    report.record(
        "ROUTE-02",
        "§3.6",
        "GET /devtools/info : versions et capacités obligatoires",
        verdict(info_shape(&reads)),
    );
    report.record(
        "ROUTE-03",
        "§3.5",
        "GET /devtools/commands : tableau de {name, description}",
        verdict(commands_shape(&reads)),
    );
    report.record(
        "ROUTE-04",
        "§3.2",
        "GET /devtools/logs/poll : 200, événements bien formés",
        verdict(logs_shape(&reads)),
    );
    report.record(
        "VERS-01",
        "§2.3",
        "En-tête X-DevEye-Protocol-Version sur chaque route (200)",
        verdict(read_routes_carry_version(&reads)),
    );
    report.record(
        "HOST-01",
        "§2.2",
        "Garde Host : 7 formes locales acceptées",
        verdict(host_accepted(t).await),
    );
    report.record(
        "HOST-02",
        "§2.2",
        "Garde Host : 13 Host étrangers ou mal formés rejetés (403 + version)",
        verdict(host_rejected(t).await),
    );
    report.record(
        "HOST-03",
        "§2.2",
        "Garde Host : requête sans en-tête Host rejetée (403, ou 400 de la pile HTTP)",
        verdict(host_absent(t).await),
    );
    report.record(
        "HOST-04",
        "§2.2",
        "Garde Host : appliquée avant le routage et le décodage du corps, corps {error}",
        verdict(host_guard_precedes_routing(t).await),
    );
    report.record(
        "ERR-01",
        "§3.0",
        "Chemin inconnu : 404 avec en-tête de version",
        verdict(unknown_path(t).await),
    );
    report.record(
        "ERR-02",
        "§3.0",
        "Méthode inattendue : 405 avec en-tête de version",
        verdict(wrong_method(t).await),
    );
    report.record(
        "ERR-03",
        "§3.0",
        "Corps JSON invalide : 4xx avec en-tête de version",
        verdict(invalid_json(t).await),
    );
    report.record(
        "ERR-04",
        "§3.0",
        "Champ obligatoire absent : 4xx avec en-tête de version",
        verdict(missing_required_field(t).await),
    );
    report.record(
        "SIZE-01",
        "§2.5",
        "Corps de plus de 1 Mio : 413 avec en-tête de version",
        body_too_large(t).await,
    );
    report.record(
        "CORS-01",
        "§2.6",
        "Préflight OPTIONS : aucune autorisation cross-origin",
        verdict(cors_preflight(t).await),
    );
    report.record(
        "CORS-02",
        "§2.6",
        "Requête avec Origin : aucune autorisation cross-origin",
        verdict(cors_simple_request(t).await),
    );
    report.record(
        "CMD-01",
        "§3.4",
        "Commande inconnue : 404 {error}, payload absent ou null accepté",
        verdict(unknown_command(t).await),
    );
    report.record(
        "CMD-02",
        "§3.5",
        "Commandes triées par nom (octets UTF-8)",
        commands_sorted(&reads),
    );
    report.record(
        "CMD-03",
        "§3.4",
        "Commande connue : 200 et JSON sans enveloppe",
        invoke_known_command(t, opts.invoke_route.as_deref()).await,
    );
    report.record(
        "LOGS-01",
        "§3.0",
        "Horodatages des logs : RFC 3339 UTC suffixé Z",
        logs_timestamps(&reads),
    );
    report.record(
        "LOGS-02",
        "§3.2",
        "Logs rendus dans l'ordre d'émission",
        logs_order(&reads),
    );
    report.record(
        "LOGS-03",
        "§3.2",
        "Poll destructif : un événement n'est rendu qu'une fois",
        logs_drain(&reads),
    );
    report.record(
        "EVAL-01",
        "§3.3",
        "Évaluation : 503 sans pompe, 200 {value|error} avec pompe, selon la capacité annoncée",
        verdict(eval_matches_capability(t, &caps).await),
    );
    report.record(
        "EVAL-02",
        "§3.3",
        "Évaluation sans réponse : 504",
        eval_special(
            t,
            &caps,
            opts.slow_expression.as_deref(),
            504,
            "--slow-expression",
        )
        .await,
    );
    report.record(
        "EVAL-03",
        "§3.3",
        "Pompe qui abandonne la requête : 500",
        eval_special(
            t,
            &caps,
            opts.abandon_expression.as_deref(),
            500,
            "--abandon-expression",
        )
        .await,
    );
    report.record(
        "SEC-01",
        "§2.4",
        "Masquage des clés sensibles dans l'état et les logs",
        redaction(&reads, &caps),
    );
    let (ui_shape, ui_secrets) = ui_widgets(t, &caps).await;
    report.record(
        "UI-01",
        "§3.7",
        "Route ui.widgets : arbre deveye.ui/1 valide (ids uniques, parents connus)",
        ui_shape,
    );
    report.record(
        "UI-02",
        "§3.7",
        "Route ui.widgets : aucune valeur publiée pour un widget secret",
        ui_secrets,
    );
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// Serveur TCP minimal : lit un peu de la requête puis exécute `reply`.
    async fn stub(reply: Option<&'static str>) -> Target {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf).await;
                if let Some(text) = reply {
                    let _ = stream.write_all(text.as_bytes()).await;
                }
                // Sans réponse : la connexion est simplement coupée.
            }
        });
        Target::new(&format!("http://127.0.0.1:{port}")).unwrap()
    }

    /// Vecteurs partagés avec le SDK Rust et les SDK tiers (ADR-0064).
    fn shared_vectors() -> Value {
        const TEXT: &str = include_str!("../../docs/schemas/shared-vectors-1.json");
        serde_json::from_str(TEXT).expect("vecteurs partagés : JSON invalide")
    }

    fn strings(value: &Value) -> Vec<String> {
        value
            .as_array()
            .expect("liste attendue")
            .iter()
            .map(|v| v.as_str().expect("chaîne attendue").to_string())
            .collect()
    }

    #[test]
    fn host_lists_match_the_shared_vectors() {
        let guard = &shared_vectors()["host_guard"];
        assert_eq!(
            accepted_hosts(31337),
            strings(&guard["accepted"])
                .iter()
                .map(|h| h.replace("{port}", "31337"))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            rejected_hosts(31337),
            strings(&guard["rejected"])
                .iter()
                .map(|h| h.replace("{port}", "31337"))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn secret_key_rule_matches_the_shared_vectors() {
        let vectors = shared_vectors();
        let r = &vectors["redaction"];
        assert_eq!(r["placeholder"], REDACTED);
        for key in strings(&r["required_secret_keys"]) {
            assert!(is_secret_key(&key), "{key}");
        }
        for key in strings(&r["ordinary_keys"]) {
            assert!(!is_secret_key(&key), "{key}");
        }
    }

    #[tokio::test]
    async fn host_absent_accepts_a_400_from_the_http_stack() {
        let t = stub(Some(
            "HTTP/1.1 400 Bad Request\r\nConnection: close\r\n\r\n",
        ))
        .await;
        assert!(host_absent(&t).await.is_ok());
    }

    #[tokio::test]
    async fn host_absent_rejects_a_200() {
        let t = stub(Some("HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n")).await;
        assert!(host_absent(&t).await.is_err());
    }

    #[tokio::test]
    async fn body_too_large_is_skipped_when_the_connection_is_cut() {
        let t = stub(None).await;
        assert!(matches!(body_too_large(&t).await, Outcome::Skip(_)));
    }

    #[test]
    fn logs_drain_is_skipped_on_an_empty_buffer() {
        let empty = || Ok(Resp::for_test(200, r#"{"events":[]}"#));
        let reads = Reads {
            state: empty(),
            info: empty(),
            commands: empty(),
            logs_first: empty(),
            logs_second: empty(),
        };
        assert!(matches!(logs_drain(&reads), Outcome::Skip(_)));
    }

    #[test]
    fn reports_never_quote_response_bodies() {
        let resp = Resp::for_test(500, "mot-de-passe-secret");
        let message = status_is(&resp, 200).unwrap_err();
        assert!(!message.contains("secret"), "{message}");
    }

    #[test]
    fn ui_tree_check_accepts_a_valid_tree_and_rejects_duplicate_ids() {
        let valid = json!({
            "schema": "deveye.ui/1",
            "window": { "width": 100, "height": 100 },
            "widgets": [
                { "id": "a", "role": "button", "rect": [0, 0, 10, 10] },
                { "id": "b", "parent": "a", "role": "label", "rect": [1, 1, 5, 5] }
            ]
        });
        assert!(check_ui_tree(&valid).is_ok());

        let mut duplicate = valid.clone();
        duplicate["widgets"][1]["id"] = json!("a");
        assert!(check_ui_tree(&duplicate).is_err());

        let legacy = json!({ "widgets": [] });
        let why = check_ui_tree(&legacy).unwrap_err();
        assert!(why.contains("schema"), "{why}");
    }

    #[test]
    fn ui_secret_leak_reports_paths_never_values() {
        let leaking = json!({
            "schema": "deveye.ui/1",
            "window": { "width": 100, "height": 100 },
            "widgets": [
                { "id": "pw", "role": "password", "value": "hunter2", "rect": [0, 0, 10, 10] },
                { "id": "ok", "role": "password", "value": "<secret>", "rect": [0, 20, 10, 10] },
                { "id": "name", "role": "textbox", "value": "alice", "rect": [0, 40, 10, 10] }
            ]
        });
        let (secrets, leaks) = ui_secret_leaks(&leaking).unwrap();
        assert_eq!(secrets, 2);
        assert_eq!(leaks, vec!["widgets/pw".to_string()]);
        assert!(!leaks.concat().contains("hunter2"));
    }
}
