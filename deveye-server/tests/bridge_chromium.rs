//! Le script du pont d'éléments (`adapter-dioxus-signals`, ADR-0074) dans un vrai
//! Chromium headless, évalué par `Runtime.evaluate` : mêmes comportements que sur
//! WebKitGTK (où le script passe par la pompe `document::eval` du SDK), sans app à
//! compiler. `#[ignore]` : il lance un navigateur.
//!
//! ```text
//! cargo test -p deveye-server --test bridge_chromium -- --ignored --test-threads=1
//! ```
//!
//! Navigateur : `$DEVEYE_TEST_CHROMIUM`, sinon le premier trouvé parmi
//! `chromium-browser`, `chromium`, `brave-browser`, `google-chrome`. Un seul
//! processus à la fois, profil jetable, arrêté au `Drop` (jamais de `pkill -f`).

use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use adapter_cdp_webview::{CdpClient, TargetFilter};
use adapter_dioxus_signals::{BridgeElementInspector, JsInteraction};
use deveye_core::elements::{ElementNode, SnapshotQuery};
use deveye_core::{InteractionCommand, MouseButton, Position};
use deveye_ports::{ElementError, ElementInspector, InteractionSink, JsEvaluator};

const FIXTURE: &str = r##"<!doctype html>
<html lang="fr"><head><meta charset="utf-8"><title>Fixture pont</title>
<style>
  body { margin: 0; font: 14px sans-serif; }
  button, input { margin: 4px; }
  #scroller { height: 80px; width: 300px; overflow: auto; border: 1px solid #888; }
  #scroller .tall { height: 400px; }
  #spacer { height: 1500px; }
  #target { position: relative; display: inline-block; }
  #veil { position: absolute; left: 0; top: 0; width: 100%; height: 100%; background: rgba(0,0,0,.1); }
  .pointer { cursor: pointer; display: inline-block; padding: 4px; }
</style></head>
<body>
  <h1>Tableau de bord</h1>
  <nav aria-label="Principal"><a href="#a">Accueil</a></nav>
  <button id="save" onclick="window.saves = (window.saves||0) + 1">Enregistrer</button>
  <button id="save2">Enregistrer</button>
  <button id="del" disabled>Supprimer</button>
  <input type="text" aria-label="Nom" value="Ada">
  <input type="password" aria-label="Mot de passe" value="hunter2">
  <input type="text" autocomplete="current-password" aria-label="Mot de passe visible" value="hunter3">
  <input type="text" name="api_key" aria-label="Cle API" value="sk-live-hunter4">
  <input type="text" autocomplete="one-time-code" aria-label="Code recu" value="hunter5">
  <input type="text" id="cvv" aria-label="Cryptogramme" value="hunter6">
  <input type="password" id="pw2" aria-label="Bascule" value="hunter7">
  <div data-deveye-secret><input type="text" aria-label="Marque secret" value="hunter8"></div>
  <input type="text" name="author" aria-label="Auteur" value="Ada Lovelace">
  <label><input type="checkbox" id="chk" checked> Se souvenir</label>
  <div class="pointer" onclick="window.zone = 1">Zone cliquable</div>
  <div id="scroller"><div class="tall"></div><button id="deep" onclick="window.deep = 1">Tout en bas</button></div>
  <div id="target"><button id="covered" onclick="window.covered = 1">Couvert</button><div id="veil"></div></div>
  <div id="tab0" tabindex="0">Focusable seulement</div>
  <div id="tab1" tabindex="1">Zone tabindex</div>
  <div id="host"></div>
  <div id="hidden" style="display:none"><button>Invisible</button></div>
  <iframe srcdoc="<button>Dans iframe</button>" width="200" height="40"></iframe>
  <ul id="rows"></ul>
  <div id="spacer"></div>
  <button id="foot" onclick="window.foot = 1">Pied de page</button>
  <script>
    // Libellé dont un emoji tombe sur la frontière de coupe (120 unités UTF-16).
    document.body.insertAdjacentHTML('beforeend',
      '<button id="long">' + 'a'.repeat(118) + '\u{1F600}\u{1F600} fin</button>');
    // Liste réaliste (balisage verbeux d'une app) : sert à mesurer le gain de taille.
    for (let i = 1; i <= 40; i++) {
      document.getElementById('rows').insertAdjacentHTML('beforeend',
        '<li class="row list-item flex items-center gap-2 px-4 py-2 border-b" data-row-id="' + i + '">' +
        '<div class="cell flex items-center gap-2 text-sm text-gray-700"><span class="icon w-4 h-4">' +
        '<svg viewBox="0 0 16 16" width="16" height="16"><path d="M2 2h12v12H2z"/></svg></span>' +
        '<span class="label font-medium">Ligne ' + i + '</span></div>' +
        '<button class="btn btn-primary btn-sm rounded px-3" data-id="' + i + '">Ouvrir</button></li>');
    }
    const root = document.getElementById('host').attachShadow({mode: 'open'});
    root.innerHTML = '<button id="shadowbtn" onclick="window.shadow = 1" ' +
      'onmousedown="window.sdown = (window.sdown || 0) + 1">Ombre</button>' +
      '<input id="sin" type="text" aria-label="Saisie ombre">' +
      '<div id="sce" contenteditable="true" role="textbox" aria-label="Edition ombre"></div>' +
      '<div id="sscroll" style="height:60px;width:200px;overflow:auto;border:1px solid #888">' +
      '<div style="height:400px">defile</div></div>';
  </script>
</body></html>"##;

struct Chromium {
    child: Child,
    dir: PathBuf,
}

impl Drop for Chromium {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn find_browser() -> String {
    if let Ok(path) = std::env::var("DEVEYE_TEST_CHROMIUM") {
        return path;
    }
    for name in [
        "chromium-browser",
        "chromium",
        "brave-browser",
        "google-chrome",
    ] {
        let found = std::env::var_os("PATH")
            .map(|paths| std::env::split_paths(&paths).any(|dir| dir.join(name).is_file()))
            .unwrap_or(false);
        if found {
            return name.to_string();
        }
    }
    panic!("aucun Chromium trouvé : définir DEVEYE_TEST_CHROMIUM");
}

fn start_chromium() -> (Chromium, u16, String) {
    let dir = std::env::temp_dir().join(format!("deveye-bridge-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("répertoire temporaire");
    let page = dir.join("fixture.html");
    std::fs::write(&page, FIXTURE).expect("fixture");
    let port = {
        let listener = TcpListener::bind("127.0.0.1:0").expect("port libre");
        listener.local_addr().expect("adresse").port()
    };
    let child = Command::new(find_browser())
        .args([
            "--headless=new",
            "--no-sandbox",
            "--disable-gpu",
            "--no-first-run",
            "--window-size=800,600",
            &format!("--remote-debugging-port={port}"),
            &format!("--user-data-dir={}", dir.join("profile").display()),
        ])
        .arg("about:blank")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("lancement du navigateur");
    let url = format!("file://{}", page.display());
    (Chromium { child, dir }, port, url)
}

async fn connect(port: u16) -> Arc<CdpClient> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        // Le navigateur démarre sur `about:blank` ; la fixture y est chargée ensuite.
        let filter = TargetFilter {
            accept_blank: true,
            ..TargetFilter::default()
        };
        if let Ok(client) = CdpClient::connect(&format!("http://127.0.0.1:{port}"), filter).await {
            return Arc::new(client);
        }
        assert!(Instant::now() < deadline, "CDP injoignable");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

async fn wait_for(client: &Arc<CdpClient>, expression: &str) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Ok(value) = client.evaluate(expression).await {
            if value == serde_json::Value::Bool(true) {
                return;
            }
        }
        assert!(
            Instant::now() < deadline,
            "condition jamais vraie : {expression}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn flatten<'a>(nodes: &'a [ElementNode], out: &mut Vec<&'a ElementNode>) {
    for node in nodes {
        out.push(node);
        flatten(&node.children, out);
    }
}

fn find<'a>(flat: &[&'a ElementNode], role: &str, name: &str) -> &'a ElementNode {
    flat.iter()
        .copied()
        .find(|n| n.role == role && n.name.as_deref() == Some(name))
        .unwrap_or_else(|| {
            let seen: Vec<_> = flat.iter().map(|n| (&n.role, &n.name)).collect();
            panic!("introuvable : {role} « {name} » parmi {seen:?}")
        })
}

fn all<'a>(flat: &[&'a ElementNode], role: &str, name: &str) -> Vec<&'a ElementNode> {
    flat.iter()
        .copied()
        .filter(|n| n.role == role && n.name.as_deref() == Some(name))
        .collect()
}

async fn eval_bool(client: &Arc<CdpClient>, expression: &str) -> bool {
    client.evaluate(expression).await.expect("évaluation") == serde_json::Value::Bool(true)
}

async fn click_ref(
    inspector: &BridgeElementInspector,
    click: &JsInteraction,
    reference: &str,
) -> Result<(), ElementError> {
    let location = inspector.locate(reference).await?;
    assert!(!location.offscreen, "{reference} doit être amené en vue");
    let (x, y) = location.center();
    click
        .dispatch(InteractionCommand::Click {
            position: Position::new(x, y),
            button: MouseButton::Left,
        })
        .await
        .expect("clic");
    Ok(())
}

#[tokio::test]
#[ignore = "lance un Chromium headless"]
async fn the_bridge_script_behaves_like_the_native_contract_in_chromium() {
    let (_browser, port, url) = start_chromium();
    let client = connect(port).await;
    let _: serde_json::Value = client
        .call("Page.navigate", serde_json::json!({ "url": url }))
        .await
        .expect("navigation");
    wait_for(&client, "location.href.includes('fixture.html')").await;
    wait_for(&client, "document.readyState === 'complete'").await;
    let evaluator: Arc<dyn JsEvaluator> = client.clone();
    let inspector = BridgeElementInspector::new(evaluator.clone());
    let click = JsInteraction::new(evaluator);

    // 1. Instantané : rôles, noms, états ; secrets, iframe et masqués absents.
    let snap = inspector
        .snapshot(&SnapshotQuery::new())
        .await
        .expect("snapshot");
    if snap.nodes.is_empty() {
        let raw = client
            .evaluate("JSON.stringify([document.readyState, location.href, innerWidth, innerHeight, document.body && document.body.children.length])")
            .await;
        panic!("instantané vide : {raw:?}");
    }
    assert_eq!(snap.target_kind, "bridge");
    let mut flat = Vec::new();
    flatten(&snap.nodes, &mut flat);
    find(&flat, "heading", "Tableau de bord");
    find(&flat, "link", "Accueil");
    find(&flat, "navigation", "Principal");
    let save = find(&flat, "button", "Enregistrer").reference.clone();
    assert!(find(&flat, "button", "Supprimer").state.disabled);
    assert_eq!(find(&flat, "textbox", "Nom").value.as_deref(), Some("Ada"));
    assert_eq!(
        find(&flat, "checkbox", "Se souvenir").state.checked,
        Some(true)
    );
    assert_eq!(find(&flat, "clickable", "Zone cliquable").role, "clickable");
    find(&flat, "button", "Ombre");
    // tabindex : seul un tabindex strictement positif désigne une zone d'action.
    find(&flat, "clickable", "Zone tabindex");
    assert!(!flat
        .iter()
        .any(|n| n.name.as_deref() == Some("Focusable seulement")));
    // Une coupe en pleine paire de substitution ne casse pas le JSON de l'instantané.
    assert!(flat.iter().any(|n| n
        .name
        .as_deref()
        .is_some_and(|name| name.starts_with("aaaa") && name.ends_with('…'))));
    // Jamais la valeur d'un champ secret, ni par le type ni par l'autocomplétion.
    assert_eq!(find(&flat, "textbox", "Mot de passe").value, None);
    assert_eq!(find(&flat, "textbox", "Mot de passe visible").value, None);
    let json = serde_json::to_string(&snap).expect("json");
    assert!(!json.contains("hunter"), "valeur secrète dans l'instantané");
    assert!(!flat.iter().any(|n| n.name.as_deref() == Some("Invisible")));
    assert!(!flat
        .iter()
        .any(|n| n.name.as_deref() == Some("Dans iframe")));
    assert!(find(&flat, "button", "Pied de page").offscreen);

    // 2. Deux instantanés donnent la même référence au même bouton.
    let again = inspector
        .snapshot(&SnapshotQuery::new())
        .await
        .expect("snapshot");
    let mut flat2 = Vec::new();
    flatten(&again.nodes, &mut flat2);
    assert_eq!(find(&flat2, "button", "Enregistrer").reference, save);
    assert_eq!(again.generation, snap.generation);

    // 3. Clic au centre remesuré, même après un décalage de mise en page.
    client
        .evaluate("document.getElementById('save').style.marginLeft = '120px'")
        .await
        .expect("décalage");
    click_ref(&inspector, &click, &save).await.expect("clic");
    assert!(eval_bool(&client, "window.saves === 1").await);

    // 4. Hors vue : amené au centre puis cliqué (fenêtre, puis conteneur défilant).
    let foot = find(&flat, "button", "Pied de page").reference.clone();
    click_ref(&inspector, &click, &foot).await.expect("pied");
    assert!(eval_bool(&client, "window.foot === 1").await);
    let deep = find(&flat, "button", "Tout en bas").reference.clone();
    click_ref(&inspector, &click, &deep)
        .await
        .expect("conteneur");
    assert!(eval_bool(&client, "window.deep === 1").await);
    let shadow = find(&flat, "button", "Ombre").reference.clone();
    click_ref(&inspector, &click, &shadow)
        .await
        .expect("shadow DOM");
    assert!(eval_bool(&client, "window.shadow === 1").await);

    // 5. Désactivé : signalé à la localisation ; couvert : référence du couvrant.
    let del = find(&flat, "button", "Supprimer").reference.clone();
    assert!(inspector.locate(&del).await.expect("locate").disabled);
    let covered = find(&flat, "button", "Couvert").reference.clone();
    match inspector.locate(&covered).await {
        Err(ElementError::Obscured { by }) => assert!(by.is_some()),
        other => panic!("attendu Obscured : {other:?}"),
    }

    // 6. Un nœud déplacé garde sa référence ; un nœud retiré est périmé, avec
    // pour candidat l'autre bouton de même rôle et de même nom.
    client
        .evaluate("document.body.appendChild(document.getElementById('save'))")
        .await
        .expect("déplacement");
    inspector.locate(&save).await.expect("référence conservée");
    let twin = find(&flat, "button", "Enregistrer");
    assert_eq!(all(&flat, "button", "Enregistrer").len(), 2);
    let others: Vec<_> = all(&flat, "button", "Enregistrer")
        .into_iter()
        .filter(|n| n.reference != twin.reference)
        .collect();
    let survivor = others[0].reference.clone();
    client
        .evaluate("document.getElementById('save').remove()")
        .await
        .expect("retrait");
    match inspector.locate(&save).await {
        Err(ElementError::Stale { candidates, .. }) => assert_eq!(candidates, [survivor]),
        other => panic!("attendu Stale : {other:?}"),
    }
    // Pas de rebind : la même référence reste périmée.
    assert!(matches!(
        inspector.locate(&save).await,
        Err(ElementError::Stale { .. })
    ));

    // 7. Bornes : max_nodes coupe et le dit ; scope par référence ; sélecteur.
    let small = inspector
        .snapshot(&SnapshotQuery::new().with_max_nodes(3))
        .await
        .expect("max_nodes");
    assert!(small.truncated && small.count() == 3);
    let scoped = inspector
        .snapshot(&SnapshotQuery::new().with_scope("#scroller"))
        .await
        .expect("scope sélecteur");
    assert!(scoped.count() >= 1 && scoped.count() < snap.count());

    // 8. Taille consignée : texte compact (une ligne par élément) contre le DOM brut.
    let compact: usize = flat
        .iter()
        .map(|n| {
            format!(
                "- {} {:?} [ref={}]\n",
                n.role,
                n.name.as_deref().unwrap_or(""),
                n.reference
            )
            .len()
        })
        .sum();
    let dom = client
        .evaluate("document.documentElement.outerHTML")
        .await
        .expect("dom");
    let dom_len = dom.as_str().map_or(0, str::len);
    eprintln!(
        "taille snapshot : {} elements, ~{} octets en texte compact, {} octets en json, \
         contre {} octets de DOM brut sur la fixture",
        snap.count(),
        compact,
        json.len(),
        dom_len
    );

    // 8 bis. Secrets : indices d'autocomplétion, de nom, de marquage explicite, champ
    // basculé de password à text après avoir été vu ; un champ ordinaire reste lu.
    let secrets = inspector
        .snapshot(&SnapshotQuery::new())
        .await
        .expect("snapshot secrets");
    client
        .evaluate("document.getElementById('pw2').type = 'text'")
        .await
        .expect("bascule");
    let toggled = inspector
        .snapshot(&SnapshotQuery::new())
        .await
        .expect("snapshot bascule");
    for snapshot in [&secrets, &toggled] {
        let mut flat = Vec::new();
        flatten(&snapshot.nodes, &mut flat);
        let json = serde_json::to_string(snapshot).expect("json");
        for leak in ["hunter4", "hunter5", "hunter6", "hunter7", "hunter8"] {
            assert!(!json.contains(leak), "{leak} fuit dans l'instantané");
        }
        assert_eq!(
            find(&flat, "textbox", "Auteur").value.as_deref(),
            Some("Ada Lovelace")
        );
    }

    // 8 ter. Toutes les clés des vecteurs partagés (ADR-0064) sont masquées, aucune
    // clé ordinaire ne l'est.
    let vectors: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../docs/schemas/shared-vectors-1.json"
        ))
        .expect("vecteurs partagés"),
    )
    .expect("json des vecteurs");
    let keys = |group: &str| -> Vec<String> {
        vectors["redaction"][group]
            .as_array()
            .expect("liste")
            .iter()
            .map(|k| k.as_str().expect("chaîne").to_string())
            .collect()
    };
    let secret_keys: Vec<String> = ["required_secret_keys", "extended_secret_keys"]
        .iter()
        .flat_map(|g| keys(g))
        .collect();
    let ordinary_keys = keys("ordinary_keys");
    let fields = secret_keys
        .iter()
        .chain(ordinary_keys.iter())
        .enumerate()
        .map(|(i, k)| {
            format!(
                "<input type=\"text\" name=\"{k}\" aria-label=\"champ-{i}\" value=\"valeur-{i}\">"
            )
        })
        .collect::<String>();
    client
        .evaluate(&format!(
            "document.getElementById('spacer').insertAdjacentHTML('beforebegin', {})",
            serde_json::to_string(&format!("<div id=\"vecteurs\">{fields}</div>")).expect("html")
        ))
        .await
        .expect("champs des vecteurs");
    let vec_snap = inspector
        .snapshot(&SnapshotQuery::new().with_max_nodes(2000))
        .await
        .expect("snapshot vecteurs");
    let mut vec_flat = Vec::new();
    flatten(&vec_snap.nodes, &mut vec_flat);
    for (i, key) in secret_keys.iter().enumerate() {
        let node = find(&vec_flat, "textbox", &format!("champ-{i}"));
        assert_eq!(node.value, None, "la clé secrète {key} doit être masquée");
    }
    for (offset, key) in ordinary_keys.iter().enumerate() {
        let i = secret_keys.len() + offset;
        let node = find(&vec_flat, "textbox", &format!("champ-{i}"));
        assert_eq!(
            node.value.as_deref(),
            Some(format!("valeur-{i}").as_str()),
            "la clé ordinaire {key} ne doit pas être masquée"
        );
    }

    // 8 quater. Élément à moitié visible dont le centre est hors fenêtre : ramené en vue.
    client
        .evaluate(
            "window.scrollTo(0, 0); document.body.insertAdjacentHTML('beforeend', \
             '<div id=\"big\" role=\"button\" aria-label=\"Grande carte\" onclick=\"window.big = 1\" \
              style=\"position:absolute;left:0;width:300px;height:400px;top:' + (innerHeight - 100) + 'px\"></div>')",
        )
        .await
        .expect("grande carte");
    let big_snap = inspector
        .snapshot(&SnapshotQuery::new())
        .await
        .expect("snapshot carte");
    let mut big_flat = Vec::new();
    flatten(&big_snap.nodes, &mut big_flat);
    let big = find(&big_flat, "button", "Grande carte");
    assert!(!big.offscreen, "à moitié visible n'est pas hors vue");
    let big_ref = big.reference.clone();
    click_ref(&inspector, &click, &big_ref)
        .await
        .expect("clic sur la grande carte");
    assert!(eval_bool(&client, "window.big === 1").await);
    // La carte recouvrirait le shadow root testé plus bas.
    client
        .evaluate("document.getElementById('big').remove()")
        .await
        .expect("retrait de la carte");

    // 8 quinquies. Shadow DOM ouvert : saisie (champ et contenteditable), défilement,
    // glisser. Chaque geste doit atteindre l'élément DANS le shadow root.
    client
        .evaluate("window.scrollTo(0, document.getElementById('host').offsetTop - 100)")
        .await
        .expect("vers l'hôte");
    let shadow_snap = inspector
        .snapshot(&SnapshotQuery::new())
        .await
        .expect("snapshot ombre");
    let mut sflat = Vec::new();
    flatten(&shadow_snap.nodes, &mut sflat);
    let field = find(&sflat, "textbox", "Saisie ombre").reference.clone();
    click_ref(&inspector, &click, &field)
        .await
        .expect("focus champ");
    click
        .dispatch(InteractionCommand::type_text("abc"))
        .await
        .expect("saisie dans le shadow root");
    assert!(
        eval_bool(
            &client,
            "document.getElementById('host').shadowRoot.getElementById('sin').value === 'abc'"
        )
        .await
    );
    let editable = find(&sflat, "textbox", "Edition ombre").reference.clone();
    click_ref(&inspector, &click, &editable)
        .await
        .expect("focus contenteditable");
    click
        .dispatch(InteractionCommand::type_text("xyz"))
        .await
        .expect("saisie contenteditable");
    assert!(
        eval_bool(
            &client,
            "document.getElementById('host').shadowRoot.getElementById('sce').textContent === 'xyz'"
        )
        .await
    );
    let scroller = client
        .evaluate(
            "(() => { const r = document.getElementById('host').shadowRoot.getElementById('sscroll')\
             .getBoundingClientRect(); return [Math.round(r.left + r.width / 2), Math.round(r.top + r.height / 2)]; })()",
        )
        .await
        .expect("rect du conteneur");
    let (sx, sy) = (
        scroller[0].as_i64().expect("x") as i32,
        scroller[1].as_i64().expect("y") as i32,
    );
    click
        .dispatch(InteractionCommand::Scroll {
            position: Position::new(sx, sy),
            dx: 0,
            dy: 120,
        })
        .await
        .expect("défilement");
    assert!(
        eval_bool(
            &client,
            "document.getElementById('host').shadowRoot.getElementById('sscroll').scrollTop > 0"
        )
        .await,
        "le conteneur défilant du shadow root doit défiler"
    );
    let before = client
        .evaluate("window.sdown || 0")
        .await
        .expect("compteur")
        .as_i64()
        .expect("entier");
    let shadow_btn = inspector
        .locate(&find(&sflat, "button", "Ombre").reference)
        .await
        .expect("locate bouton");
    let (bx, by) = shadow_btn.center();
    click
        .dispatch(InteractionCommand::DragDrop {
            from: Position::new(bx, by),
            to: Position::new(bx, by),
        })
        .await
        .expect("glisser");
    let after = client
        .evaluate("window.sdown || 0")
        .await
        .expect("compteur")
        .as_i64()
        .expect("entier");
    assert_eq!(
        after,
        before + 1,
        "le glisser doit partir du bouton du shadow root"
    );

    // 8 sexies. DOM très profond : la sortie reste décodable (limite serde_json de
    // 128 niveaux), tronquée et signalée ; `max_depth: 0` vaut 1.
    client
        .evaluate(
            "document.getElementById('spacer').insertAdjacentHTML('beforebegin', \
             '<div id=\"abyss\"></div>'); let n = document.getElementById('abyss'); \
             for (let i = 0; i < 120; i++) { const d = document.createElement('div'); \
             d.setAttribute('role', 'group'); d.setAttribute('aria-label', 'niveau ' + i); \
             d.style.height = '1px'; n.appendChild(d); n = d; } true",
        )
        .await
        .expect("DOM profond");
    let deep_snap = inspector
        .snapshot(
            &SnapshotQuery::new()
                .with_scope("#abyss")
                .with_interactive_only(false),
        )
        .await
        .expect("un DOM profond ne doit pas faire échouer l'instantané");
    assert!(deep_snap.truncated, "profondeur coupée et signalée");
    let one = inspector
        .snapshot(&SnapshotQuery::new().with_max_depth(0))
        .await
        .expect("max_depth 0");
    assert!(one.count() >= 1 && one.nodes.iter().all(|n| n.children.is_empty()));

    // 9. Rechargement de page : autre epoch, anciennes références périmées, numéros
    // jamais réutilisés.
    let before_max = flat
        .iter()
        .filter_map(|n| n.reference.strip_prefix('e')?.parse::<u64>().ok())
        .max()
        .expect("références");
    client
        .evaluate("setTimeout(() => location.reload(), 0); true")
        .await
        .expect("rechargement");
    wait_for(
        &client,
        "document.readyState === 'complete' && !window.__deveye_elements__",
    )
    .await;
    match inspector.locate(&foot).await {
        Err(ElementError::Stale { .. }) => {}
        other => panic!("attendu Stale après rechargement : {other:?}"),
    }
    let reloaded = inspector
        .snapshot(&SnapshotQuery::new())
        .await
        .expect("snapshot");
    assert!(reloaded.generation > snap.generation);
    let mut flat3 = Vec::new();
    flatten(&reloaded.nodes, &mut flat3);
    let fresh = find(&flat3, "button", "Enregistrer");
    let fresh_n: u64 = fresh.reference[1..].parse().expect("numéro");
    assert!(
        fresh_n > before_max,
        "{} réutilise un numéro",
        fresh.reference
    );
}
