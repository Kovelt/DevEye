# ADR-0057 — `doctor` : `--display` vérifié après le parsing, sondes limitées à la cible et parallèles

- **Statut** : Proposé
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0031 (validation des flags selon la cible), ADR-0032 (`doctor` en lecture seule), ADR-0044 (cible x11 isolée)

## Contexte et problème

Deux défauts rendaient `doctor` moins utile que prévu :

1. `--display` était exigé par clap (`required_if_eq("target", "x11")`). `deveye-server doctor --target x11` sans display était donc refusé avant d'exécuter `doctor`, alors que son rôle est justement de diagnostiquer cette absence (« aucun --display fourni ») ;
2. les sondes SDK et CDP étaient exécutées l'une après l'autre, et le CDP aussi pour les cibles qui ne s'en servent pas : jusqu'à 9 à 12 s d'attente pour rien.

S'y ajoutaient le découpage fragile des URL (`http://[::1]` sans port, `user@hôte`).

## Facteurs de décision

- `doctor` doit répondre vite et rester en lecture seule ;
- le serveur MCP, lui, doit continuer à refuser `--target x11` sans `--display` ;
- aucune nouvelle dépendance (`reqwest::Url` est déjà présent).

## Options envisagées

1. **Garder l'exigence clap** : `doctor` ne peut pas diagnostiquer l'absence de display.
2. **Déplacer l'exigence dans `Args::validate`** : refusée pour le serveur, tolérée pour `doctor`.
3. **Retirer l'exigence partout** : le serveur démarrerait sans cible utilisable.

## Décision

Option 2. En complément : `doctor` ne sonde le SDK et le CDP que si la table des prérequis de la cible les déclare utiles (le CDP seulement pour `auto` et `cdp`), les deux sondes tournent en parallèle (`tokio::join!`), et l'hôte et le port des URL sont lus par `reqwest::Url`.

**Justification** : le diagnostic gagne en vitesse et en utilité sans toucher au contrat du serveur.

## Conséquences

**Positives**

- `doctor --target x11` signale l'absence de display au lieu d'être refusé ;
- les cibles `sdk`, `kwin-eis` et `x11` ne paient plus le délai du CDP.

**Coûts et risques**

- le message d'erreur du serveur n'est plus produit par clap mais par `validate` (même type d'erreur, `MissingRequiredArgument`) ;
- `auto` sans CDP ni SDK joignable continue de sortir `0` (deux avertissements) : le repli OS-level reste possible, ce n'est pas un échec.
