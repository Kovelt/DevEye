# ADR-0013 — Neutralité vis-à-vis des apps clientes et archivage des documents historiques

- **Statut** : Accepté
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0003 (hébergement GitHub), ADR-0011 (surface stable), ADR-0014 (filtre de cible CDP), ADR-0015 (retrait du script de synchronisation)

## Contexte et problème

La règle de découplage du projet interdit qu'un crate DevEye dépende d'une app cliente ou en porte la logique. Dans les faits, le code, les tests, les commentaires, les valeurs par défaut, les scripts et la documentation citaient encore des apps consommatrices de l'époque, des chemins absolus de la machine du mainteneur, un dossier de synchronisation privé et des références à l'ancienne forge. Un dépôt destiné à être ouvert au public ne peut ni exposer ces traces privées, ni laisser croire que DevEye ne fonctionne qu'avec certaines apps. Par ailleurs, des documents datés (audits, feuille de route, spike, backlog) citent légitimement ces apps et leurs chiffres d'alors, mais se présentaient comme de la documentation courante.

## Facteurs de décision

- respecter la règle de découplage et la rendre vérifiable par un simple `grep` ;
- ne rien publier de privé (chemins, forge interne, adresses) à l'ouverture du dépôt ;
- ne pas perdre l'historique de conception, utile pour comprendre les choix ;
- ne pas laisser un lecteur confondre une archive datée avec une référence à jour ;
- peu d'effort de maintenance pour un seul mainteneur.

## Options envisagées

1. **Tout purger, y compris les archives** : plus aucune trace, mais l'historique de conception est perdu (il reste dans git, mais n'est plus lisible simplement).
2. **Réécrire les archives avec des noms neutres** : fausse l'histoire (chiffres et décisions datés) et coûte beaucoup pour un document non normatif.
3. **Purger le code, les tests, les scripts et les docs de référence ; déplacer les archives telles quelles sous `docs/historique/` avec un README qui en fixe le statut.**

## Décision

Option 3.

- Le code, les commentaires, les tests, les valeurs par défaut, les `Cargo.toml`, le `justfile`, les scripts et les documents de référence (`README.md`, `docs/usage.md`, `docs/architecture.md`, `docs/wire-protocol.md`, `docs/stability.md`) ne citent plus aucune app cliente ni référence privée ; les exemples utilisent des noms neutres (`mon_app`, « une app Dioxus desktop »).
- `docs/audit-2026-06-25.md`, `docs/audit-2026-07-04.md`, `docs/tools-backlog-2026-06-26.md`, `docs/spike-atspi2-2026-07-04.md` et `docs/roadmap/` sont déplacés **sans modification** sous `docs/historique/` (`git mv`) ; `docs/historique/README.md` en explique le statut d'archives non normatives.
- Les ADR peuvent évoquer l'histoire de façon neutre (« les apps consommatrices de l'époque ») sans nommer d'app.
- Les références à `github.com/Morgan-Voltz/DevEye` et la ligne de copyright des licences restent légitimes.

**Justification** : seule cette option rend la règle de découplage vérifiable sans détruire l'historique ni le falsifier ; le coût se limite à un déplacement de fichiers et à une purge mécanique.

## Conséquences

**Positives**

- un `grep` sur les noms d'apps hors `docs/historique/` ne doit plus rien renvoyer d'injustifié ;
- plus de trace privée dans les fichiers de référence ;
- l'historique reste consultable et clairement étiqueté.

**Coûts et risques**

- les archives contiennent toujours des noms d'apps et des chemins privés : elles sont publiques avec le dépôt. **Point de checklist avant de passer le dépôt en public** : le mainteneur relit `docs/historique/` et décide de les conserver telles quelles, de les expurger ou de les exclure du dépôt public ; tant que cette relecture n'est pas faite, l'ouverture ne s'appuie pas sur l'hypothèse qu'elles sont anodines ;
- les liens internes des archives vers d'anciens chemins ne sont pas corrigés (contenu inchangé).

## Notes

Application : lot « neutralité » de la release 1.0 (commits `refactor`, `docs` et `docs(adr)` du même lot).

Archives déplacées : [`docs/historique/`](../historique/README.md) (statut), [audit du 25/06](../historique/audit-2026-06-25.md), [audit du 04/07](../historique/audit-2026-07-04.md), [backlog d'outils](../historique/tools-backlog-2026-06-26.md), [spike AT-SPI2](../historique/spike-atspi2-2026-07-04.md), [feuille de route](../historique/roadmap/).
