# ADR-0001 — Consigner chaque décision dans un ADR

- **Statut** : Accepté
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : aucun

## Contexte et problème

DevEye passe en open source avec la 1.0. Jusqu'ici, les choix structurants vivaient dans des audits, des plans de sprint et la mémoire des sessions de travail (« pourquoi chrono est-il dans l'API publique ? », « pourquoi cette exception d'audit ? »). Des contributeurs externes, comme le mainteneur dans six mois, n'ont aucun moyen fiable de retrouver le *pourquoi* d'une décision, ni de savoir si elle est encore valable.

## Facteurs de décision

- retrouver la justification d'un choix sans archéologie de l'historique Git ;
- cadrer les contributions externes : savoir ce qui se discute avant de coder ;
- garder un coût d'écriture faible (quelques paragraphes, pas un dossier) ;
- rester cohérent avec la pratique des autres projets du mainteneur.

## Options envisagées

1. **Pas de formalisme** — messages de commit et audits en vrac ; le statu quo.
2. **Un journal de décisions unique** (`DECISIONS.md`) — simple, mais devient illisible et se prête mal au suivi des remplacements.
3. **Un ADR par décision** dans `docs/adr/`, numérotés, avec statut et index.

## Décision

Chaque décision structurante est consignée dans un ADR sous `docs/adr/`, selon le gabarit et le processus de [docs/adr/README.md](README.md), dans le commit qui l'applique ou juste avant.

**Justification** : un fichier par décision se relit, se lie et se remplace sans toucher aux autres ; le format est celui des autres projets du mainteneur ; l'index et le statut rendent visible ce qui est en vigueur.

Sont couverts : architecture, dépendance ajoutée ou retirée, licence, protocole, surface stable, plateforme, outillage, exception de sécurité ou de lint, choix entre plusieurs options pendant un chantier. Un ADR de statut « Proposé » signale un choix qui engage le mainteneur au-delà du chantier en cours.

## Conséquences

**Positives**

- l'historique des raisons est lisible et versionné avec le code ;
- les revues de contribution disposent d'un point d'appui explicite (CLAUDE.md, règle non négociable 9).

**Coûts et risques**

- une discipline d'écriture à tenir à chaque décision ; un ADR oublié n'est détecté qu'en revue ;
- le risque de bureaucratie existe : un ADR reste court, et une décision mineure et réversible sans conséquence externe n'en exige pas.

## Notes

Règle reprise dans `CLAUDE.md` (règle 9 et section « Ce qu'il NE faut PAS faire »).
