# Implémentation tierce du protocole

Une app qui n'est pas écrite en Rust (par exemple .NET, Go, Python, Node) n'embarque pas `deveye-sdk` : elle implémente le **contrat HTTP** du SDK, et `deveye-server` la pilote comme n'importe quelle app avec SDK. Le contrat est **agnostique du langage** : tout ce qui sert du HTTP/1.1 sur la boucle locale convient.

## 1. Le contrat en bref

| Point | Valeur |
|---|---|
| Adresse | `127.0.0.1` exclusivement, jamais `0.0.0.0` ni `::` |
| Port | `31337`, surchargé par `DEVEYE_PORT` (entier de 1 à 65535) |
| Routes | `GET /devtools/info`, `GET /devtools/state`, `GET /devtools/logs/poll`, `POST /devtools/eval`, `POST /devtools/command`, `GET /devtools/commands` |
| Garde `Host` | `403` si `Host` n'est pas `localhost`, `127.0.0.1` ou `[::1]` (port numérique optionnel), **avant** toute route |
| Version | en-tête `X-DevEye-Protocol-Version: 1.0` sur **chaque** réponse, y compris les `403` et `404` |
| CORS | aucune origine autorisée |
| Corps | `413` au-delà de 1 Mio |
| Secrets | masquage par clés dans l'état et les champs de logs (recommandé, périmètre exact en §2.4) |

La spécification normative (formes JSON, codes de statut, ordre de tri, exigences « DOIT » et « DEVRAIT ») est [wire-protocol.md](../wire-protocol.md) ; la checklist d'écriture en est la section 6 ; les schémas JSON sont dans `docs/schemas/1.0/`.

## 2. Valider avec la suite de conformité

La suite `deveye-conformance` valide une implémentation **à partir de sa seule URL**, sans accès à son code :

```bash
# depuis un clone de DevEye, l'implémentation étant à l'écoute
cargo run -p deveye-conformance -- --url http://127.0.0.1:31337
# ou : just conformance http://127.0.0.1:31337
```

Options : `--slow-expression` (vérifie le `504` quand la pompe ne répond pas, 10 s), `--abandon-expression` (le `500` quand elle abandonne), `--invoke-route` (le `200` de `POST /devtools/command`, avec une commande sans effet de bord), `--strict` (échoue aussi quand une exigence n'a pas pu être observée). Codes de sortie : 0 conforme, 1 écarts, 2 la suite n'a pas pu s'exécuter. Ajouter la suite à la CI de l'implémentation : un écart de contrat est détecté avant qu'un agent ne le subisse ([ADR-0035](../adr/0035-suite-de-conformite-boite-noire-du-protocole.md)).

## 3. Brancher DevEye

Le serveur se branche sur l'implémentation comme sur un SDK Rust ; la cible dépend de l'app :

```json
{
  "mcpServers": {
    "deveye": {
      "type": "stdio",
      "command": "deveye-server",
      "args": ["--target", "kwin-eis", "--screencap-window", "Mon App",
               "--sdk-url", "http://127.0.0.1:31337", "--serve-mcp"]
    }
  }
}
```

(Ici une app native sous KDE Wayland ; pour une app dans un serveur X privé, `--target x11 --display :250` : [app-x11-xvfb-prive.md](app-x11-xvfb-prive.md).) `deveye-server doctor --sdk-url …` vérifie que `GET /devtools/info` répond et que la version du protocole est compatible.

## 4. Garde-fous

- **Dev-only.** Ne compiler et n'activer l'implémentation que dans les builds de développement ; la garde `Host` et le bind loopback sont obligatoires ([ADR-0042](../adr/0042-posture-dev-only-du-sdk.md)). Un SDK qui ne les respecte pas n'est pas conforme.
- **Pompe d'évaluation.** Si l'app a une webview, `POST /devtools/eval` doit exécuter l'expression dans le moteur de la page et répondre `{ value }` ou `{ error }` ; sans pompe, répondre `503`. Une pompe qui ne répond pas dans les 10 s donne `504`.
- **Route `ui.widgets`.** Pour une app sans webview, l'exposer comme commande : c'est le « DOM » de l'outil `dom` en cible `x11`.
- **Secrets.** Ne pas publier de secret dans l'état ; le masquage par clés n'est qu'un filet et ne couvre ni `eval`, ni les retours de commandes.
- **Statut de l'implémentation .NET existante.** Une implémentation .NET tierce est maintenue **hors de ce dépôt** et validée par cette suite ([ADR-0036](../adr/0036-statut-du-shim-dotnet-implementation-tierce.md)).
