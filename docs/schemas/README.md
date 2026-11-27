# Schémas JSON du wire protocol

JSON Schema (brouillon 2020-12) des corps du wire protocol HTTP de DevEye, un dossier par version du protocole (`1.0/`). Ils sont référencés par la spécification [`docs/wire-protocol.md`](../wire-protocol.md) (section 8), qui reste la source de vérité des comportements : codes de statut, tri, drain des logs, garde Host.

Règles de version : un ajout rétrocompatible (`1.x`) ajoute des propriétés optionnelles dans `1.0/` sans rien retirer ; une rupture (`2.0`) crée `2.0/`. Les schémas n'interdisent pas les propriétés inconnues, car les clients les ignorent.
