# ADR-0071 — Dépôt hébergé dans l'organisation GitHub Kovelt

- **Statut** : Accepté
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0003 (hébergement sur GitHub), ADR-0002 (licences)

## Contexte et problème

Le code de conduite s'appuie sur la fonction de signalement de contenu de GitHub (« Report to repository admins »), qui évite de publier une adresse e-mail. Or cette fonction n'existe que pour les dépôts appartenant à une organisation : sur un compte personnel, le menu n'est pas proposé. Le dépôt, hébergé sous le compte personnel du mainteneur, ne pouvait donc pas offrir le canal annoncé.

## Facteurs de décision

- un canal de signalement privé, horodaté, sans adresse publiée ;
- ne pas exposer de boîte de réception personnelle ;
- continuité des liens existants (git, web, crates publiés) ;
- coût de migration faible.

## Options envisagées

1. **Rester sur le compte personnel, avec une issue de demande de contact** : le repli actuel ; l'issue est publique et expose le fait qu'un signalement existe.
2. **Rester sur le compte personnel, avec une adresse e-mail dédiée** : canal privé, mais une adresse à publier, à surveiller et à protéger du courrier indésirable.
3. **Transférer le dépôt dans l'organisation Kovelt**, dont le mainteneur est administrateur : active le signalement de contenu aux administrateurs, sans adresse publiée.

## Décision

Option 3. Le dépôt est transféré vers `github.com/Kovelt/DevEye` (le 2026-11-27). Le mainteneur active le réglage Settings, Access, Moderation options, Reported content. Le repli par issue de demande de contact est conservé pour les comportements hors de GitHub.

## Conséquences

**Positives**

- le signalement de contenu fonctionne comme le décrit le code de conduite ;
- GitHub redirige les anciennes adresses (git et web), donc les liens existants et les clones continuent de fonctionner.

**Mises à jour induites**

- tous les liens du dépôt (README, CHANGELOG, documentation, code et commentaires) pointent vers `Kovelt/DevEye`, sauf l'archive `docs/historique/` ;
- les champs `repository` et `homepage` des crates, et les identifiants `$id` des schémas JSON, utilisent la nouvelle adresse ;
- des modèles d'issue (formulaires de bogue et de fonctionnalité, issues vierges désactivées, liens vers le signalement privé de vulnérabilités et le code de conduite) et de demande de fusion sont ajoutés sous `.github/`.

**Coûts et risques**

- les redirections tiennent tant qu'aucun dépôt du même nom n'est recréé sous l'ancien compte : il ne faut pas le faire ;
- les crates déjà publiés gardent l'ancienne adresse dans leurs métadonnées jusqu'à la version suivante ; la redirection couvre l'intervalle.
