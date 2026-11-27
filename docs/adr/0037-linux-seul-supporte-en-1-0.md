# ADR-0037 — Linux seul supporté en 1.0

- **Statut** : Accepté
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0003 (CI GitHub Actions), ADR-0011 (surface stable), ADR-0038 (distribution), ADR-0039 (tests de bout en bout)

## Contexte et problème

Les documents présentaient Windows (WebView2) comme cible prioritaire, alors que la CI ne tourne que sous Linux et que `deveye-server` dépend sans condition d'adapters propres à Linux (X11, KWin par D-Bus, Wayland, descripteurs Unix). Annoncer une plateforme que personne ne teste est une promesse que la 1.0 ne peut pas tenir. Il faut dire clairement ce qui est supporté, et faire en sorte qu'un essai sur une autre plateforme échoue de façon lisible.

## Facteurs de décision

- n'annoncer que ce que la CI vérifie ;
- coût de maintenance d'un job Windows ou macOS pour un outil de développement ;
- message d'échec clair plutôt qu'une pile d'erreurs de compilation obscures ;
- ne pas fermer la porte : d'autres plateformes pourront venir en 1.x.

## Options envisagées

1. **Ajouter des jobs Windows et macOS** et gater (`cfg`) chaque adapter Linux : coût élevé, adapters à réécrire pour compiler, aucun testeur sur ces plateformes.
2. **Gater les adapters Linux par `cfg(target_os = "linux")` dans `deveye-server`** : un binaire dégradé sur les autres plateformes, à tester sans pouvoir le faire.
3. **Déclarer Linux seul et faire échouer la compilation de `deveye-server` ailleurs** par un `compile_error!` au message explicite.

## Décision

Option 3, conformément à la décision du propriétaire : Linux est officiel (X11 privé, Wayland KDE, CDP et SDK sous Linux) ; Windows et macOS ne sont pas supportés en 1.0 (non testés) ; GNOME Wayland est hors périmètre (portails XDG envisagés en 1.x). Un `compile_error!` en tête de `deveye-server/src/main.rs` interrompt la compilation hors `target_os = "linux"` en renvoyant à cet ADR.

**Justification** : c'est l'option la plus simple et la plus honnête. Le gating (option 2) produirait un binaire que personne ne peut valider ; la CI multi-plateforme (option 1) ne se justifie pas sans demande. Le message d'erreur explique tout de suite la situation. La limite est volontairement posée sur le binaire seul : les bibliothèques MIT (`deveye-core`, `deveye-ports`, `deveye-sdk`, `deveye-sdk-client`) restent portables, car une app cliente les embarque sur sa propre plateforme.

## Conséquences

**Positives**

- promesse alignée sur ce que la CI teste ;
- échec de compilation immédiat et explicite hors Linux ;
- aucune dette de `cfg` dans les adapters.

**Coûts et risques**

- les utilisateurs Windows et macOS ne peuvent pas construire l'outil en 1.0 ; une app cible qui tourne sur ces systèmes ne peut être pilotée que par un `deveye-server` exécuté sous Linux (par exemple un CDP joignable), cas non testé ;
- le `compile_error!` n'a pas pu être exercé localement hors Linux (aucune cible installée) ; sa correction repose sur la simplicité de la directive ;
- ouvrir une nouvelle plateforme exigera un nouvel ADR et un job de CI dédié.

## Notes

Les mentions de Windows et de macOS dans `docs/architecture.md` et `docs/usage.md` décrivent l'app cible ou l'historique ; la passe de documentation de la 1.0 les relit.

Nuance (revue de la 1.0) : le `compile_error!` n'est atteint que si les dépendances compilent d'abord hors Linux ; un adapter Linux (`nix`, `xcb`, D-Bus) peut échouer avant avec un message moins clair. Il reste le signal explicite pour le cas courant ; une garde en `build.rs` n'y changerait rien, cargo compilant les dépendances en parallèle du script de construction.
