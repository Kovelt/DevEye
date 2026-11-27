# ADR-0060 — Dépendances d'adapters, cache de CI et verrous empoisonnés du SDK

- **Statut** : Accepté
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0003 (GitHub Actions), ADR-0009 (lints et unsafe), ADR-0024 (sérialisation sans panic), ADR-0039 (CI de bout en bout), ADR-0042 (posture dev-only), ADR-0044 (cible x11), ADR-0045 (KWin)

## Contexte et problème

Plusieurs dépendances et choix d'outillage sont entrés dans le dépôt sans ADR : `x11rb` et `xkeysym` (adapter X11), `zbus`, `nix`, `tempfile` (KWin, presse-papiers), `xkbcommon` (EIS), `tracing-subscriber` (SDK et serveur), `clap` (serveur, conformité), l'action `Swatinem/rust-cache` en CI, et la récupération des verrous empoisonnés du SDK, qu'ADR-0024 laissait explicitement de côté. La règle du projet veut que chaque dépendance se justifie.

## Facteurs de décision

- ne pas réécrire un protocole (X11, D-Bus, EIS) à la main ;
- `unsafe` banni hors exception ADR-0009 ;
- poids de compilation et surface de dépendances ;
- un panic d'un handler de l'app ne doit pas rendre le SDK inutilisable ;
- temps de CI raisonnable.

## Options envisagées

1. **Écrire les clients à la main** (X11, D-Bus) : zéro dépendance, mais du code de protocole et de l'`unsafe` à maintenir.
2. **Dépendances ciblées, Rust pur quand il existe**, chacune bornée à son crate ; cache CI par action tierce épinglée par SHA ; verrous du SDK récupérés après empoisonnement.
3. **Ne pas cacher la CI** et **laisser les verrous paniquer** : plus simple, mais lent et fragile.

## Décision

Option 2, avec le périmètre suivant.

| Dépendance | Crate qui l'embarque | Raison |
|---|---|---|
| `x11rb` (XTEST) | `adapter-x11`, `deveye-server` | client X11 en Rust pur ; capture et événements du serveur privé |
| `xkeysym` | `adapter-x11` | table keysym vers caractère, sans `xkbcommon` natif |
| `zbus` (tokio) | adapters KWin | seule interface de `ScreenShot2` et `EIS` : D-Bus |
| `nix` (`fs`) | `adapter-screencap-kwin` | `pipe2(O_CLOEXEC)` sans `unsafe` |
| `tempfile` | adapters KWin et presse-papiers | fichiers temporaires sûrs |
| `xkbcommon` | `adapter-interaction-kwin-eis` | keysym et Unicode vers keycode ; seul `unsafe` isolé (ADR-0009) |
| `tracing-subscriber` | `deveye-sdk`, `deveye-server` | couche de collecte des logs ; sortie de diagnostic |
| `clap` | `deveye-server`, `deveye-conformance` | analyse de la CLI et aide générée |

Le cache de CI utilise `Swatinem/rust-cache`, épinglé par SHA de commit. Les verrous du SDK qui protègent des états simples (instantané d'état, tampon de logs) sont récupérés par `PoisonError::into_inner` : un panic dans un handler de l'app ne doit pas désactiver l'inspection.

**Justification** : chaque dépendance évite du code de protocole ou de l'`unsafe` et reste confinée à l'adapter qui la requiert (les crates MIT n'en tirent aucune lourde, ADR-0002). La récupération des verrous est sûre parce que les données gardées sont des valeurs complètes, remplacées d'un bloc, sans invariant à cheval sur plusieurs champs.

## Conséquences

**Positives**

- pas de code de protocole maison ;
- CI plus rapide ;
- le SDK survit au panic d'un handler d'app.

**Coûts et risques**

- chaque dépendance entre dans `cargo deny` et les montées de version ;
- une action de CI tierce est une dépendance de chaîne d'approvisionnement : épinglage par SHA obligatoire ;
- récupérer un verrou empoisonné peut exposer un état écrit à moitié si un futur état devient composite : à revérifier à chaque nouvel état partagé.

## Notes

ADR proposé : il consigne après coup des choix faits pendant la 1.0 ; le mainteneur peut en refuser un, ce qui rouvre le chantier concerné.

> Validation : accepté par le mainteneur le 2026-11-27, avec les autres ADR proposés pendant la préparation de la 1.0, avant le tag `v1.0.0`.
