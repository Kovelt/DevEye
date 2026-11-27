# ADR-0007 — Exception d'audit pour quick-xml en build-dependency (RUSTSEC-2026-0194 et 0195)

- **Statut** : Proposé
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0006 (autre exception d'audit)

## Contexte et problème

`quick-xml` 0.30.0 est touché par deux avis de déni de service (RUSTSEC-2026-0194 : coût quadratique à l'exécution ; RUSTSEC-2026-0195 : allocation non bornée des espaces de noms). Il n'est tiré qu'en *build-dependency* par `xcb`, lui-même tiré par `xcap` (adapter `adapter-screencap-xcap`), pour parser les définitions du protocole X11 **à la compilation**. Aucune version corrigée n'est atteignable sans montée de `xcb`/`xcap`. L'exception figure dans `deny.toml` depuis l'audit du 04/07/2026 (item I3) ; cet ADR la consigne rétroactivement.

## Facteurs de décision

- le code vulnérable ne s'exécute qu'à la compilation, sur des fichiers du dépôt `xcb`, jamais sur une entrée non fiable à l'exécution ;
- pas de version corrigée atteignable sans changement hors de notre contrôle ;
- ne pas supprimer la capture générique `xcap`.

## Options envisagées

1. **Supprimer `adapter-screencap-xcap`** — perte d'une capacité de repli.
2. **Forker `xcb`/`xcap`** — maintenance disproportionnée.
3. **Ignorer les deux avis, avec justification** dans `deny.toml`, jusqu'à ce que `xcap` remonte `quick-xml`.

## Décision

Option 3 : exception documentée dans `deny.toml`, à lever dès que `xcap` met à jour `quick-xml`.

**Justification** : le vecteur d'attaque (fichier XML hostile à la compilation) exige déjà de contrôler les sources du build.

## Conséquences

**Positives**

- l'audit reste vert et honnête : l'exception dit pourquoi.

**Coûts et risques**

- à revérifier à chaque montée de `xcap` ; si `quick-xml` 0.30 passe un jour en dépendance d'exécution, l'exception tombe.

## Notes

Statut rétabli en « Proposé » : ce choix a été fait par l'agent de la release 1.0 et aucune trace de validation par le mainteneur n'existe dans le dépôt (décisions du propriétaire du 27/11/2026 : aucune ne le couvre explicitement). Il passera à « Accepté » à la validation du mainteneur.
