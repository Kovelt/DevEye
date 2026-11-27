# ADR-0031 — Validation des flags selon la cible

- **Statut** : Accepté
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0011 (les flags de `deveye-server` font partie de la surface stable), ADR-0014 (filtre de cible CDP), ADR-0032 (sous-commande `doctor`)

## Contexte et problème

`clap` acceptait et ignorait en silence les options qui ne s'appliquent pas à la cible choisie : `--cdp-url` ou `--cdp-target-filter-*` avec `--target sdk`, `kwin-eis` ou `x11`, `--screencap-backend` avec `x11`. Seuls `--display` et `--x11-class` étaient refusés hors `x11`. Une option ignorée fait croire à une configuration qui n'existe pas (filtre CDP posé mais sans effet, backend de capture forcé mais jamais utilisé). Par ailleurs, sans `--serve-mcp`, le binaire composait les adapters, imprimait « Composition validée, sortie » et rendait le code 0 : un lancement sans le flag réussissait en apparence et ne servait rien. Les flags devenant une promesse de la 1.0 (ADR-0011), il faut fixer la règle avant le gel.

## Facteurs de décision

- aucune option acceptée sans effet ;
- ne refuser que ce qui est vraiment sans effet (vérifié dans le code de composition, pas supposé) ;
- compatibilité : la 1.0 n'est pas publiée, les commandes documentées (README, `usage.md`) ne combinent aucune option ignorée ;
- un échec qui nomme l'option, la cible et la règle.

## Options envisagées

1. **Avertir** (journal sur stderr) et continuer : rétrocompatible, mais l'avertissement se noie dans les logs d'un serveur MCP lancé par un client, et la configuration reste fausse.
2. **Refuser** avec une erreur d'usage (code 2) au démarrage.
3. **Ne rien changer** et documenter.

## Décision

Option 2 : une option sans effet sur la cible est une erreur d'usage au démarrage, avec le message `<option> <règle> ; cible actuelle : <cible>`.

Table retenue (vérifiée sur `compose.rs`) :

| Option | Cibles où elle s'applique | Refusée avec |
|---|---|---|
| `--cdp-url`, `--cdp-target-filter-url`, `--cdp-target-filter-title`, `--cdp-accept-blank` | `auto`, `cdp` | `sdk`, `kwin-eis`, `x11` |
| `--display`, `--x11-class` | `x11` | toutes les autres |
| `--screencap-backend` | `auto`, `cdp`, `sdk`, `kwin-eis` | `x11` |
| `--sdk-url`, `--screencap-window` | toutes | jamais |

Points précis :

- `--screencap-backend` n'est **pas** refusé avec `cdp`, contrairement à ce que laissait croire la documentation : en cible `cdp`, `window_geometry` et `focus_window` passent par KWin selon ce backend. Il n'est inutile qu'avec `x11`, qui capture par `GetImage`.
- `--sdk-url` sert toutes les cibles (state, logs et commandes viennent toujours du SDK) ; `--screencap-window` sert le focus et la géométrie même en `cdp`.
- Pour distinguer « option absente » de « option posée à sa valeur par défaut », `--cdp-url` et `--screencap-backend` n'ont plus de valeur par défaut clap : les accesseurs `Args::cdp_url()` (`http://localhost:9222`) et `Args::screencap_backend()` (`auto`) la restituent. Les valeurs par défaut restent celles de la surface stable.
- **`--serve-mcp` reste obligatoire sans sous-commande** (il n'est pas rendu implicite) : c'est la forme documentée partout (`.mcp.json`, README) et le seul moyen de ne pas démarrer un serveur stdio qui attend sur l'entrée quand quelqu'un tape `deveye-server` dans un terminal. Son absence devient une erreur d'usage (code 2) au lieu d'une sortie silencieuse en succès ; `--serve-mcp` combiné à une sous-commande est refusé.

**Justification** : seule l'erreur rend la mauvaise configuration visible au moment où elle est écrite ; l'avertissement l'aurait laissée vivre. Le coût de compatibilité est nul avant la 1.0 et la règle est plus simple à promettre qu'une liste d'exceptions tolérées. La table est tirée du câblage réel pour ne pas refuser une option utile.

## Conséquences

**Positives**

- une faute de frappe de cible (`--target sdk` au lieu de `auto`) est détectée au lieu d'ignorer le filtre CDP ;
- `deveye-server` sans mode n'a plus de succès trompeur ;
- les tests couvrent chaque combinaison (`cli::tests`, `tests/cli.rs`).

**Coûts et risques**

- un `.mcp.json` écrit pour `auto` puis passé à `--target sdk` en gardant `--cdp-url` échoue désormais au démarrage : le message dit quoi retirer ;
- ajouter plus tard un usage à une option aujourd'hui refusée (par exemple `--screencap-backend` en `x11`) est une extension mineure ; refuser une option aujourd'hui acceptée serait une rupture (ADR-0011).

## Notes

Application : `deveye-server/src/cli.rs` (`Args::validate`, `Args::require_mode`), `deveye-server/src/compose.rs` (accesseurs), `docs/usage.md` (troubleshooting) et `docs/stability.md`.

> Validation : accepté par le mainteneur le 2026-11-27, avec les autres ADR proposés pendant la préparation de la 1.0, avant le tag `v1.0.0`.
