# ADR-0066 — Push de logs et `layer()` du SDK

- **Statut** : Accepté
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0070 (remplace le point « feature `tracing-layer` »), ADR-0012 (types `tokio` exposés), ADR-0016 (masquage par clés), ADR-0042 (posture dev-only), ADR-0060 (verrous empoisonnés du SDK), ADR-0065 (helper `ui`, capacité annoncée à l'usage)

## Contexte et problème

Les logs n'arrivaient dans `logs_tail` que par `install_tracing_layer()`, qui pose le subscriber `tracing` **global** et ignore silencieusement l'échec si un autre existe déjà. Un hôte qui possède son subscriber (plugin audio, jeu) ou n'utilise pas `tracing` n'avait aucune voie. Le buffer reposait en outre sur un canal non borné et une tâche de drain créés par cette seule fonction : événements perdus avant l'installation, mémoire non bornée, couplage à un runtime.

## Facteurs de décision

- additif seulement : aucune signature stable ne change (`install_tracing_layer` garde la sienne) ;
- un push doit marcher de n'importe quel fil, y compris hors runtime tokio ;
- `tracing-subscriber` ne doit pas être imposé à qui n'en veut pas ;
- no-op en release, comme tout le SDK (ADR-0042).

## Options envisagées

1. **`log()` seul** — zéro couplage, mais n'aide pas l'hôte qui a déjà un subscriber et des événements `tracing` à capter.
2. **`log()` + `layer()` public, sans feature** — impose `tracing-subscriber` à tous.
3. **`log()` + `layer()` derrière la feature `tracing-layer` (par défaut), `try_install_tracing_layer()` qui signale l'échec** — retenue à l'origine ; le volet « feature » est remplacé par l'ADR-0070 (aucune feature en 1.x, `layer()` et `try_install_tracing_layer()` restent toujours disponibles).
4. **Verrou du buffer** : `tokio::sync::Mutex` + `blocking_lock` (panique depuis un contexte async), ou canal non borné (état précédent), ou `std::sync::Mutex` — `std` retenu.

## Décision

Option 3 (hors feature, cf. ADR-0070) et `std::sync::Mutex` pour le buffer.

- `log(level, target, message)` et `log_with(…, fields)` empilent directement dans le buffer (horodatage UTC, filtre `DEVEYE_LOG_LEVEL` défaut `info`, abandon des plus anciens à 1 024), sans abonné ni runtime. `LogLevel` est réexporté.
- `layer()` renvoie `DeveyeLayer`, un `tracing_subscriber::Layer` composable avec le subscriber de l'hôte ; `DeveyeLayer` ne se construit que par `layer()`.
- `try_install_tracing_layer()` renvoie `TracingInstallError::SubscriberAlreadySet` (`#[non_exhaustive]`) si un subscriber global existe ; `install_tracing_layer()` l'appelle et ignore le résultat, comme avant. Un second appel après un succès est sans effet.
- Le canal non borné et la tâche de drain sont supprimés : le layer écrit synchrone, ce qui préserve l'ordre d'émission et ne perd plus rien avant l'installation. Le verrou n'est jamais tenu à travers un `.await`.
- La capacité `logs` est annoncée dès qu'une source est armée (premier `log`, `layer()` construit, subscriber installé), cohérent avec ADR-0065. Capacité additive : `protocol_version` reste `1.0`.
- `DEVEYE_LOG_LEVEL` garde son vocabulaire (noms sans casse ou chiffres 1 à 5).

## Conséquences

**Positives** : tout hôte peut alimenter `logs_tail` ; l'échec d'installation devient observable ; plus de panique possible depuis un contexte async ; code plus simple.

**Coûts et risques** : `DeveyeLayer` expose `tracing_subscriber::Layer` 0.3 dans l'API stable ; un changement de majeure de `tracing-subscriber` serait un changement de surface du SDK (cf. ADR-0070 et `docs/stability.md`). (Les points de cette phrase relatifs à la feature `tracing-layer`, à `default-features = false`, au CHANGELOG, à `docs/stability.md` et au test `--no-default-features` sont caducs : il n'y a plus de feature, donc aucune rupture pour un consommateur 1.0 ; voir ADR-0070.) `layer()` annonce `logs` dès sa construction : un layer jamais composé dans un abonné actif laisse `logs_tail` vide (limite assumée, documentée). Un seul subscriber global par process : `layer()` ne règle pas le cas d'un hôte qui n'en compose aucun, d'où `log()`. Le texte du message n'est pas masqué, seuls les champs le sont (ADR-0016).

## Notes

Tests : `deveye-sdk/tests/logs_push.rs` (fil nu, tâche tokio, 1 500 pushs pour 1 024 gardés, `layer()` local, masquage), `logs_subscriber_conflict.rs`, `release_noop.rs` et la sonde `release_probe`.

Note de forme (2026-11-27) : le point « feature `tracing-layer` » de cette décision est remplacé par l'ADR-0070. La feature n'a jamais été publiée ; `layer()`, `install_tracing_layer` et `try_install_tracing_layer` sont toujours disponibles, car la déplacer derrière une feature rompait `default-features = false` pour un consommateur 1.0. Le reste de l'ADR (push `log()`, tampon borné, capacité `logs`) est inchangé.
