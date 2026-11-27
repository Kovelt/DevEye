# ADR-0059 — Politique de sécurité et conventions de contribution

- **Statut** : Accepté
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0003 (GitHub Actions), ADR-0011 (versions suivies), ADR-0016 (masquage des secrets), ADR-0042 (posture dev-only), ADR-0047 (code de conduite)

## Contexte et problème

`SECURITY.md` et `CONTRIBUTING.md` engagent le mainteneur (canal de signalement, délai de réponse, versions corrigées, format des commits et des branches) sans qu'aucun ADR n'en consigne le choix. ADR-0047 ne couvre que le code de conduite.

## Facteurs de décision

- un projet maintenu par une seule personne : engagements tenables ;
- aucune adresse e-mail publiée ;
- historique lisible et vérifiable mécaniquement ;
- cohérence avec GitHub comme seule forge.

## Options envisagées

1. **Signalement privé de vulnérabilité de GitHub**, repli par une issue sans détail technique ; accusé de réception sous sept jours ; correctifs sur la dernière 1.x seulement.
2. **Adresse e-mail dédiée** : contraire au choix de ne publier aucune adresse.
3. **Aucune politique** : laisse les failles se signaler en public.

Pour les contributions : commits Conventional Commits en français, **sujet en ASCII sans accents** (outils et terminaux sans surprise, recherche fiable dans l'historique), branches `<type>/<slug>` ; ou règles plus libres.

## Décision

Option 1 pour la sécurité, et les conventions de `CONTRIBUTING.md` pour les commits (ASCII, impératif, scope = crate) et les branches (GitHub Flow, `<type>/<slug-kebab-case>`).

**Justification** : le signalement privé de GitHub ne demande ni adresse publique ni infrastructure ; le délai de sept jours et la seule dernière 1.x sont ce qu'une personne seule peut tenir. Les conventions de commit rendent l'historique filtrable et alimentent le CHANGELOG.

## Conséquences

**Positives**

- un canal clair pour les failles, sans exposer de détail technique ;
- un historique homogène.

**Coûts et risques**

- le signalement privé doit être activé dans les réglages GitHub, sinon le repli par issue s'applique ;
- les anciennes versions 1.x ne reçoivent pas de correctif ;
- l'ASCII dans les sujets de commit est une contrainte que seul un relecteur fait respecter.

> Validation : accepté par le mainteneur le 2026-11-27, avec les autres ADR proposés pendant la préparation de la 1.0, avant le tag `v1.0.0`.
