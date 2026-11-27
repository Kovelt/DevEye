# ADR-0047 — Adopter le Contributor Covenant 2.1 comme code de conduite

- **Statut** : Accepté
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0001 (processus des ADR), ADR-0002 (open source sous AGPL et MIT), ADR-0003 (hébergement GitHub)

## Contexte et problème

DevEye devient un projet open source : des personnes extérieures vont ouvrir des issues, proposer du code et discuter. Il faut dire clairement, dès le premier jour, ce qui est attendu d'elles, ce qui ne sera pas toléré, qui fait respecter la règle et comment signaler un problème. Le propriétaire a demandé un code de conduite **clair pour les participants**.

## Facteurs de décision

- un texte reconnu, que les contributeurs ont déjà lu ailleurs ;
- lisible en français, la langue du projet ;
- des conséquences graduées, prévisibles, pas arbitraires ;
- un canal de signalement qui n'expose aucune adresse électronique et qui fonctionne sans infrastructure en plus (projet maintenu par une personne, hébergé sur GitHub) ;
- une licence qui autorise l'adaptation et la traduction avec attribution.

## Options envisagées

1. **Rédiger un texte maison** : sur mesure, mais sans reconnaissance, avec le risque d'oublier des cas (champ d'application, récusation, gradation).
2. **Reprendre le Contributor Covenant 2.1 en anglais tel quel** : reconnu, mais en anglais, et son contact attend une adresse électronique.
3. **Adapter en français le Contributor Covenant 2.1**, avec attribution, et remplacer son contact par le signalement de contenu de GitHub.

## Décision

Option 3. `CODE_OF_CONDUCT.md` est une adaptation française fidèle du Contributor Covenant 2.1 (engagement, standards de comportement, responsabilités d'application, champ d'application, signalement, quatre niveaux d'application : correction, avertissement, exclusion temporaire, exclusion définitive), avec l'attribution et le lien vers la version originale (licence CC BY 4.0).

**Signalement.** Aucune adresse électronique n'est publiée. Les signalements passent par la fonction de **signalement de contenu de GitHub** (menu « ... » d'un commentaire, d'une issue, d'une demande de fusion ou d'une discussion, puis « Report content » et « Report to repository admins »). Les mainteneurs s'engagent à accuser réception sous sept jours, à garder la confidentialité de la personne qui signale, à se récuser quand ils sont concernés, et renvoient les cas graves à GitHub lui-même. Le CoC renvoie vers [CONTRIBUTING.md](../../CONTRIBUTING.md) et [SECURITY.md](../../SECURITY.md) (une faille de sécurité n'est pas un signalement de conduite).

**Justification** : le Covenant est le texte le plus répandu des projets open source, ses niveaux d'application évitent l'arbitraire, et son adaptation française le rend lisible par tous les participants. Remplacer le contact par la fonction GitHub évite d'exposer une boîte de réception et laisse une trace horodatée.

## Conséquences

**Positives**

- les participants savent ce qui est attendu, ce qui est exclu et comment signaler ;
- les conséquences sont graduées et connues d'avance ;
- aucun secret ni adresse à entretenir.

**Coûts et risques**

- la fonction « Report to repository admins » doit être **activée** par le propriétaire dans les réglages du dépôt après la publication (Settings, Moderation options, Reported content) ; tant qu'elle ne l'est pas, le signalement de contenu ne parvient pas aux mainteneurs ;
- l'engagement d'accusé de réception sous sept jours engage le mainteneur ;
- l'adaptation n'est pas la traduction officielle du Covenant : le texte original fait foi en cas de doute (précisé dans l'attribution).

## Notes

Le projet reste modeste : un seul mainteneur. Si l'équipe s'élargit, le texte du CoC pourra désigner plusieurs personnes ; ce serait un nouvel ADR.
