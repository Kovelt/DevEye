# ADR-0044 — Cible x11 isolée par construction : display privé obligatoire

- **Statut** : Accepté
- **Date** : 2026-11-04
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0041 (architecture), ADR-0037 (Linux seul), ADR-0039 (tests Xvfb bloquants en CI), ADR-0045 (Wayland KDE)

## Contexte et problème

Pour piloter une app X11 native (egui, GTK, Qt) sans webview ni SDK, DevEye injecte de vrais événements d'entrée par l'extension XTEST. Pointée sur la session graphique de l'utilisateur, cette injection déplacerait sa souris et taperait dans ses vraies fenêtres : un agent qui se trompe de display détruit du travail en cours.

## Facteurs de décision

- aucune possibilité de piloter la session vivante, même par erreur de configuration ;
- un message de refus qui dit quoi faire ;
- testable sans serveur graphique réel (la garde) et avec un vrai serveur privé (`Xvfb`) ;
- pas de dépendance à `$DISPLAY`, qui désigne justement la session vivante.

## Options envisagées

1. **Utiliser `$DISPLAY` par défaut** et avertir : le défaut est précisément la session vivante.
2. **Display explicite avec une option pour lever les garde-fous** : un contournement existe, donc finira par servir.
3. **Display explicite et garde sans contournement.**

## Décision

Option 3. La cible `x11` exige `--display`, sans valeur par défaut ni repli sur `$DISPLAY`. L'adapter `adapter-x11` refuse, à la construction ou à la première connexion :

- un nom de display non local (seuls `:N` et `:N.S` sont acceptés) ;
- un numéro inférieur à 100 (les sessions réelles vivent sur `:0` ou `:1` ; `Xvfb :99` des CI usuelles est donc refusé, prendre `:100` ou plus) ;
- le display de la session courante (`$DISPLAY`) ;
- un socket `/tmp/.X11-unix/X<n>` appartenant à un autre uid ;
- tout serveur annonçant l'extension `XWAYLAND` (le serveur X embarqué d'un compositeur Wayland est une session vivante).

Il n'existe **aucune option** pour lever ces refus. Pour piloter une session réelle, la voie est un autre adapter (KWin EIS, ADR-0045).

**Justification** : un garde-fou contournable par option n'en est pas un ; seule l'absence de contournement fait que la cible ne peut pas, par construction, toucher la session de l'utilisateur. Le coût (lancer un `Xvfb` privé) est faible face au risque.

## Conséquences

**Positives**

- impossible d'injecter dans la session vivante avec cette cible ;
- les cinq refus se testent sans serveur graphique, le reste contre un `Xvfb` privé en CI ;
- la cible fonctionne sans gestionnaire de fenêtres ni bibliothèque système (client X en Rust pur).

**Coûts et risques**

- on ne peut pas piloter une app déjà ouverte sur la session ; il faut la lancer sur le serveur privé ;
- la capture est celle de la racine rognée à la fenêtre : une fenêtre recouvrante apparaît dans l'image, aucune mise à l'échelle HiDPI, pas de compositeur (docs/usage.md) ;
- le plancher à 100 est un choix de sécurité, pas du protocole X.

## Notes

ADR rétroactif. Introduit le 2026-11-04 (`49c702c`, garde d'isolation ; cible `--target x11` dans `cd60252`). Voir [docs/guides/app-x11-xvfb-prive.md](../guides/app-x11-xvfb-prive.md).
