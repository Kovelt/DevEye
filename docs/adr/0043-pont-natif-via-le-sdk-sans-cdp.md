# ADR-0043 — Pont natif via le SDK quand CDP est indisponible

- **Statut** : Accepté
- **Date** : 2026-06-11
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0042 (posture dev-only du SDK), ADR-0046 (repli du mode auto), ADR-0041 (architecture)

## Contexte et problème

Sur Windows, la webview d'une app Dioxus (WebView2) expose le protocole CDP. Sous Linux, la webview est WebKitGTK, qui n'a pas de CDP standard : DevEye n'avait ni yeux ni mains sur la vraie app desktop. Il fallait un moyen de lui faire évaluer du JavaScript et recevoir des événements, sans CDP.

## Facteurs de décision

- piloter la **vraie** app desktop, pas une variante web servie à part ;
- rester dans le contrat dev-only du SDK (ADR-0042) ;
- réutiliser les ports existants plutôt que d'en créer un par technique ;
- ne pas dépendre du moteur de la webview.

## Options envisagées

1. **Servir une variante web de l'app** et la piloter en CDP : ne teste pas l'app réelle.
2. **Protocole d'inspection propre à WebKitGTK** : non standard, peu stable, une pile de plus.
3. **Pont natif par le SDK** : l'app embarque le SDK, qui expose une route d'évaluation ; une « pompe » côté app exécute le JavaScript dans sa webview et renvoie le résultat.

## Décision

Option 3. Le SDK expose `POST /devtools/eval` ; l'app reprend les requêtes avec `deveye_sdk::take_eval_requests()` et les exécute dans sa webview (par exemple avec le rappel d'évaluation de la webview). Côté DevEye, `adapter-dioxus-signals` s'appuie dessus : évaluateur JavaScript, inspecteur (le DOM vient d'`outerHTML`) et interaction par événements JavaScript synthétiques (clics, saisie, touches, défilement, glisser) qui gardent le focus. Les yeux viennent d'une capture OS (`--target sdk`).

**Justification** : c'est le seul moyen de piloter l'app réelle sous WebKitGTK sans dépendre d'un protocole propre au moteur, et il se range derrière les ports `JsEvaluator`, `AppInspector` et `InteractionSink` : le reste de DevEye ne voit pas la différence avec CDP. Il impose peu à l'app : une pompe d'évaluation de quelques lignes.

## Conséquences

**Positives**

- la vraie app Linux est pilotable (état, logs, commandes, DOM, saisie) ;
- les outils MCP sont les mêmes avec ou sans CDP ;
- l'état, les logs et les commandes de l'app passent par le même SDK, y compris pour les apps sans webview (cible `x11`).

**Coûts et risques**

- l'app doit intégrer le SDK et sa pompe ; sans elle, `eval_js` répond « sdk_unavailable » ;
- les événements sont synthétiques : ils ne valent pas de vrais événements du système (une saisie dans un champ natif ou un menu système n'est pas couverte), et `resize`, `upload_file` et `handle_dialog` restent propres à CDP ;
- un JavaScript évalué par le SDK n'est pas masqué (ADR-0016).

## Notes

ADR rétroactif. Introduit le 2026-06-11 (`da81991`, « pont natif »), avec l'option `--target auto|cdp|sdk`. Voir [docs/integration.md](../integration.md) et [docs/guides/dioxus-desktop-linux.md](../guides/dioxus-desktop-linux.md).
