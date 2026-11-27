# Schémas JSON du wire protocol

JSON Schema (brouillon 2020-12) des corps du wire protocol HTTP de DevEye, un dossier par version du protocole (`1.0/`). Ils sont référencés par la spécification [`docs/wire-protocol.md`](../wire-protocol.md) (section 8), qui reste la source de vérité des comportements : codes de statut, tri, drain des logs, garde Host.

Règles de version : un ajout rétrocompatible (`1.x`) ajoute des propriétés optionnelles dans `1.0/` sans rien retirer ; une rupture (`2.0`) crée `2.0/`. Les schémas n'interdisent pas les propriétés inconnues, car les clients les ignorent.

## Autres fichiers

- [`ui-widgets-1.json`](ui-widgets-1.json) : schéma `deveye.ui/1` de la réponse de la route `ui.widgets` (arbre de widgets d'une app native). **Généré** par les types de `deveye-core` (feature `schema`) : ne pas l'éditer à la main, relancer `just schema` après un changement voulu des types ; un test échoue si le fichier diverge. Ce schéma a sa propre version (`deveye.ui/1`), indépendante de celle du protocole.
- [`shared-vectors-1.json`](shared-vectors-1.json) : vecteurs partagés du masquage des secrets (clés sensibles, clés ordinaires, clés déclarées, cas de masquage) et de la garde `Host` (hôtes acceptés et rejetés), consommés par le SDK Rust, la suite de conformité et à reprendre par tout SDK tiers ([ADR-0064](../adr/0064-masquage-des-secrets-transversal-et-vecteurs-partages.md)).
