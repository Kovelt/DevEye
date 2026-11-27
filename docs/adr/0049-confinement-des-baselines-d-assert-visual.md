# ADR-0049 — Confinement des baselines d'`assert_visual`

- **Statut** : Accepté
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0030 (`assert_visual` : masques, image de différence, mise à jour), ADR-0027 (annotations : `assert_visual` destructif), ADR-0042 (posture dev-only)

## Contexte et problème

`assert_visual` recevait un `baseline_path` libre et, selon le cas, créait le dossier parent, **écrivait** le PNG (baseline absente ou `update_baseline`) ou **lisait** le fichier pour le comparer (l'image de différence renvoyée en dévoile le contenu décodable). Un client MCP, ou un agent victime d'une injection de prompt, pouvait donc écraser ou créer n'importe quel fichier du compte (`~/.bashrc` avec `update_baseline: true`) ou lire n'importe quel PNG. ADR-0030 s'en remettait à git comme filet ; l'annotation « destructif » ne protège que si le client demande confirmation.

## Facteurs de décision

- fermer la faille sans casser l'usage normal (des baselines versionnées dans le dépôt de l'app) ;
- ne pas ajouter de flag CLI : les flags sont une surface stable (ADR-0011) ;
- comportement vérifiable par des tests, y compris les liens symboliques.

## Options envisagées

1. **Documenter la limite** seulement (ADR-0030, SECURITY) : aucun changement, la faille reste.
2. **Racine unique configurable** : `baseline_path` relatif à `DEVEYE_BASELINE_DIR` (défaut : répertoire courant du serveur) ; un chemin absolu n'est accepté que dans cette racine ; extension `.png` ; refus de `..`, des liens symboliques sortants et d'un lien ou dossier à l'emplacement de la baseline ; remplacement refusé si le fichier existant n'est pas un PNG lisible.
3. **Flag `--baseline-dir`** : même effet, mais nouvelle surface de CLI à geler.

## Décision

Option 2, appliquée dans `adapter-mcp-server` (`confine_baseline`, racine lue à la construction du serveur, `with_baseline_root` pour les tests).

**Justification** : elle ferme l'écriture et la lecture arbitraires avec une variable d'environnement (non gelée comme un flag) et un défaut raisonnable (le dossier où l'on lance le serveur, en général le dépôt de l'app).

## Conséquences

**Positives**

- plus d'écrasement ni de lecture hors de la racine ; un fichier non PNG n'est jamais écrasé ;
- comportement testé (`..`, absolu hors racine, extension, liens symboliques, fichier non PNG).

**Coûts et risques**

- changement de comportement visible : un `baseline_path` absolu hors du répertoire courant est désormais refusé (`invalid_params`) tant que `DEVEYE_BASELINE_DIR` n'est pas posé ;
- le serveur lancé depuis `$HOME` confine à `$HOME` : mieux vaut le lancer depuis le dépôt de l'app ;
- la racine est résolue à la construction : changer la variable demande de relancer le serveur.

## Notes

Application : `deveye-adapters/adapter-mcp-server/src/tools_capture.rs`. Documenté dans `docs/usage.md` et `SECURITY.md`.

> Validation : accepté par le mainteneur le 2026-11-27, avec les autres ADR proposés pendant la préparation de la 1.0, avant le tag `v1.0.0`.
