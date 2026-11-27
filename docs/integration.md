# Intégrer le SDK dans une app

Ce guide s'adresse à l'auteur d'une app Rust qui veut qu'un agent voie son **état**, ses **logs** et déclenche ses **commandes**, et qui, avec un webview sans CDP, lui laisse évaluer du JavaScript. Le SDK (`deveye-sdk`, licence MIT) n'impose aucune contrainte de licence à l'app : l'intégrer ne la rend pas AGPL ([ADR-0002](adr/0002-licences-agpl-outil-mit-bibliotheques.md)).

Une app **non Rust** (par exemple .NET) n'embarque pas ce crate : elle implémente le même contrat HTTP, voir [guides/implementation-tierce.md](guides/implementation-tierce.md).

## 1. Déclarer la dépendance

Le SDK ne doit exister que dans les builds de développement. Cargo ne sait pas évaluer `cfg(debug_assertions)` dans une table `[target.'cfg(…)'.dependencies]` : le patron fiable est une **dépendance optionnelle** derrière une feature, plus une garde dans le code.

Une fois les bibliothèques publiées sur crates.io (la publication reste une décision du mainteneur, [ADR-0004](adr/0004-preparer-publication-cratesio-sans-publier.md)) :

```toml
[dependencies]
deveye-sdk = { version = "1", optional = true }
serde_json = "1"

[features]
deveye = ["dep:deveye-sdk"]
```

**Tant qu'elles ne le sont pas**, dépendre du dépôt, épinglé sur le tag de la version :

```toml
[dependencies]
deveye-sdk = { git = "https://github.com/Morgan-Voltz/DevEye", tag = "v1.0.0", optional = true }
```

Lancer l'app avec la feature : `cargo run --features deveye` (ou `dx serve --features deveye` pour une app Dioxus).

Le SDK dépend de `tokio` (runtime multi-thread à un thread, embarqué), `axum` 0.8 et `tower-http` 0.7 : leur plage de versions est visible dans le graphe de l'app ([ADR-0008](adr/0008-montees-de-version-avis-rustsec.md)). La version minimale de Rust est 1.88.

## 2. Initialiser, en debug seulement

```rust
fn main() {
    #[cfg(all(debug_assertions, feature = "deveye"))]
    {
        deveye_sdk::init(deveye_sdk::Config::default());
        deveye_sdk::install_tracing_layer(); // optionnel : capture des logs `tracing`
    }

    // … lancement de l'app (par exemple `dioxus::launch(App)`) …
}
```

- **Double garde.** `#[cfg(debug_assertions)]` est la voie normale ; en plus, `deveye_sdk::init` est un **no-op en build release** : même si l'intégrateur oublie la garde, aucun port ne s'ouvre ([ADR-0042](adr/0042-posture-dev-only-du-sdk.md)). Le dépôt le prouve sur binaire réel (`just release-noop`).
- **Appelable avant le runtime de l'app.** Le SDK a son propre runtime tokio ; `init` s'appelle sans contexte asynchrone, y compris avant `dioxus::launch`.
- **Port.** `Config::default()` lit `DEVEYE_PORT` (entier de 1 à 65535) sinon écoute `31337`, **toujours sur `127.0.0.1`**. Plusieurs apps en même temps : un `DEVEYE_PORT` par app et le `--sdk-url` correspondant. Un port déjà pris est signalé par un `warn` `tracing` et laisse l'app tourner.
- **Logs.** Trois voies, au choix ([ADR-0066](adr/0066-push-de-logs-et-layer-du-sdk.md)) :
  - `deveye_sdk::try_install_tracing_layer()` pose le subscriber `tracing` **global** (un seul par process) et renvoie une erreur si un autre existe déjà ; `install_tracing_layer()` est la variante qui ignore cet échec ;
  - si l'app possède déjà son subscriber : `tracing_subscriber::registry().with(deveye_sdk::layer())` (couplage à `tracing-subscriber` 0.3, feature `tracing-layer`, active par défaut ; `default-features = false` l'écarte) ;
  - sans `tracing` du tout : `deveye_sdk::log(LogLevel::Info, "cible", "message")` ou `log_with(…, champs_json)`, appelables de n'importe quel fil, avec ou sans runtime.

  Niveau minimal capturé : `info`, surchargeable par `DEVEYE_LOG_LEVEL` (`trace` à `error`) ; tampon de 1 024 événements, les plus anciens étant abandonnés. Les champs structurés sont masqués par clés sensibles, pas le texte du message. Les trois voies sont des no-op en build release.

## 3. Publier l'état

```rust
#[cfg(all(debug_assertions, feature = "deveye"))]
deveye_sdk::set_state("page", serde_json::json!("Agenda"));
```

`set_state(nom, valeur)` écrit de façon synchrone dans le registre (une lecture juste après voit la nouvelle valeur) et écrase la valeur précédente. L'agent lit l'état avec l'outil `state` (filtrable par clés), l'assert avec `assert_state`, ou attend une valeur avec `wait_condition` (`state-key`). `wait_ready` attend un état **non vide** : publier au moins une clé dès que l'app est prête est le signal le plus simple.

Appeler `set_state` depuis un effet réactif de l'app (par exemple un `use_effect` Dioxus) pour suivre ses signaux.

## 4. Exposer des commandes

```rust
#[cfg(all(debug_assertions, feature = "deveye"))]
deveye_sdk::expose_route("set-page", "Change la page affichée", |payload| {
    let page = payload.get("page").and_then(|v| v.as_str()).unwrap_or("Accueil");
    // … muter le signal de navigation de l'app …
    serde_json::json!({ "ok": true, "page": page })
});
```

L'agent découvre les commandes avec `list_commands` (triées par nom) et les déclenche avec `invoke_command { name, args }`. Plus robuste qu'une cascade de clics pour amener l'app dans un état donné. Le handler est synchrone, s'exécute hors du runtime du SDK et gère ses propres erreurs ; un second `expose_route` sur le même nom remplace le précédent. Un nom a un usage conventionnel : `ui.widgets`, que la cible `x11` lit comme « DOM » ([guides/app-x11-xvfb-prive.md](guides/app-x11-xvfb-prive.md)).

## 5. Webview sans CDP : la pompe d'évaluation

Sous WebKitGTK, `eval_js`, `dom` et les clics du pont natif passent par une **pompe** : une tâche de l'app qui reprend les requêtes d'évaluation et les exécute dans sa webview.

```rust
#[cfg(all(debug_assertions, feature = "deveye"))]
{
    let mut requests = deveye_sdk::take_eval_requests();
    // `spawn` : le lanceur de tâches de votre framework (runtime de l'app).
    spawn(async move {
        while let Some(deveye_sdk::EvalRequest { expression, respond, .. }) = requests.recv().await {
            // `evaluate_in_webview` est propre à l'app : elle exécute `expression`
            // dans sa webview et rend la valeur JSON, ou un message d'erreur.
            let outcome: Result<serde_json::Value, String> = evaluate_in_webview(&expression).await;
            let _ = respond.send(outcome);
        }
    });
}
```

Points à respecter :

- `expression` est une expression JavaScript unique (en pratique une fonction immédiate) ; la pompe l'enveloppe d'un `try/catch` et renvoie `Err(message)` en cas d'exception, que l'agent reçoit en `js_exception`.
- `EvalRequest` est `#[non_exhaustive]` : toujours la destructurer avec `..`.
- Une pompe qui ne répond pas dans les **10 s** donne un `504` au client ; une file pleine (32 requêtes en attente) donne un `503`. Ne pas bloquer le thread qui draine la file.
- Un nouvel appel de `take_eval_requests` remplace la pompe précédente.
- Sans pompe, `eval_js` répond « aucune pompe d'évaluation enregistrée » ; avec un webview Chromium joignable en CDP, la pompe n'est pas nécessaire.

## 6. Secrets et limites

Le SDK remplace par `<secret>` la valeur des clés JSON sensibles dans l'état et dans les champs structurés des logs : `password`, `passwd`, `secret`, `apikey`, `privatekey`, `credential`, `authorization` (n'importe où dans la clé), les clés se terminant par `token`, `cookie` ou `sessionid`, et les clés exactes `pwd`, `auth`, `jwt`, `bearer`, `otp`. La comparaison ignore la casse et les séparateurs (`API-KEY` vaut `api_key`). Pour ajouter les clés propres à l'app :

```rust
deveye_sdk::init(deveye_sdk::Config::default().with_secret_keys(["iban", "numero_carte"]));
```

**Ce filet n'est pas une garantie.** Il n'agit pas sur : le résultat d'`eval_js`, les valeurs de retour de `invoke_command`, le texte libre des messages de log, ni un secret rangé sous un nom de clé neutre non déclaré. Ne pas publier de secret dans l'état ; ne pas supposer qu'une valeur est masquée parce qu'elle « ressemble » à un secret. La route `ui.widgets` est traitée à part : le helper `deveye_sdk::ui` ne publie jamais la valeur d'un widget secret et `dom` la masque en seconde ligne ([ADR-0064](adr/0064-masquage-des-secrets-transversal-et-vecteurs-partages.md)). Détail : [wire-protocol.md](wire-protocol.md) §2.4, [ADR-0016](adr/0016-masquage-des-secrets-par-cles-dans-le-sdk.md), [SECURITY.md](../SECURITY.md).

Corps de requête limités à 1 Mio. Aucun jeton d'authentification : tout processus de la machine peut parler au SDK ; le SDK est conçu pour un poste de développement de confiance.

## 7. Vérifier

1. Lancer l'app avec la feature, puis `deveye-server doctor --target sdk` : la ligne « SDK de l'app cible » doit afficher la version du SDK et du protocole.
2. Lancer la suite de conformité contre l'app : `deveye-conformance --url http://127.0.0.1:31337` (depuis un clone de DevEye) valide le contrat HTTP ([wire-protocol.md](wire-protocol.md) §7).
3. Brancher `deveye-server` dans `.mcp.json` ([usage.md](usage.md#brancher-dans-claude-code--mcpjson)) et appeler `target_info`, `state`, `list_commands`.

## 8. Transition : copies vendorées et script de synchronisation

> **Déprécié.** La voie normale est la dépendance versionnée (sections 1 et 2). Le script décrit ici n'existe que pour les apps qui, **avant la publication sur crates.io**, embarquent des copies de `deveye-core`, `deveye-ports` et `deveye-sdk` dans leur arbre ([ADR-0040](adr/0040-fin-du-vendoring-script-de-synchronisation-de-transition.md)). Il sera supprimé quand les bibliothèques seront publiées.

`scripts/sync-deveye.sh` copie ces trois crates vers des dossiers que **vous** désignez ; il n'écrit que lorsqu'on le lance.

1. Désigner les cibles, au choix : l'argument répétable `--target [ÉTIQUETTE=]DOSSIER` ; la variable `DEVEYE_SYNC_TARGETS` (entrées séparées par `:`) ; ou un fichier de cibles, par défaut `scripts/sync-targets` (local, ignoré par git ; modèle : `scripts/sync-targets.example`, avec des chemins fictifs). Priorité : arguments, puis variable, puis fichier.
2. `just sync-deveye --dry-run` : montre ce qui serait copié, sans écrire.
3. `just sync-deveye` : copie. Un `VENDORED.md` consigne dans chaque cible le commit source et un hash par crate.
4. `just sync-deveye --check` : compare sans écrire ; code **1** si une copie diffère du dépôt, utilisable en pré-commit ou en CI.

Codes de sortie : 0 succès ; 1 dérive détectée (`--check`) ou copie éditée localement (abandon) ; 2 usage ou configuration invalide : aucune cible, **cible absente** (jamais ignorée en silence), `rsync` manquant. Une copie éditée à la main depuis la dernière synchronisation fait abandonner le script : rapatrier les changements dans DevEye, puis relancer avec `--force`.

Les crates copiés héritent de `[workspace.package]`, `[workspace.dependencies]` et `[workspace.lints]` : le workspace qui contient la copie doit déclarer les mêmes entrées (voir le `Cargo.toml` racine de DevEye). Pour migrer : retirer la copie, déclarer la dépendance de la section 1, supprimer `VENDORED.md`.

Depuis la version 0.2, le contrat a évolué ; voir le [guide de migration](migration-1.0.md).
