# ADR-0065 — Helper SDK `ui`, lecture serveur de `ui.widgets` et contrôles

- **Statut** : Accepté
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0032 (doctor en lecture seule), ADR-0035 (suite de conformité), ADR-0042 (posture dev-only), ADR-0043 (pont natif via le SDK), ADR-0062 (schéma `deveye.ui/1`), ADR-0064 (masquage transversal)

## Contexte et problème

Le schéma `deveye.ui/1` (ADR-0062) fige le contrat de la route `ui.widgets`, mais une app doit encore écrire son handler à la main, et le serveur renvoie la réponse brute sans la contrôler ni la masquer (ADR-0064). Il faut : un helper dans le SDK pour que l'app publie son arbre à bon compte, une lecture serveur qui masque les secrets, un contrôle de l'arbre dans `doctor` et dans la suite de conformité, et une preuve que le helper reste un no-op en release.

## Facteurs de décision

- l'UI en mode immédiat ne doit construire son arbre que lorsqu'un agent regarde ;
- le handler de route ne tourne jamais sur le thread d'interface ;
- sûr par défaut : la valeur d'un champ secret ne quitte pas l'app ;
- un handler tiers qui valide strictement son payload ne doit pas casser ;
- `doctor` reste en lecture seule (ADR-0032) ;
- aucune dépendance nouvelle ; ajout purement additif de la surface stable.

## Options envisagées

1. **Enregistrer la route au premier `publish` seulement** — rejeté : une app qui ne publie que sous `is_observed()` ne serait jamais observée, donc jamais interrogée (interblocage).
2. **Envoyer `{ accept_schema: [1], max_nodes }` dès maintenant** — rejeté : un handler qui désérialise son payload en mode strict casserait, et le port `CommandRouter` n'a pas accès à `/devtools/info` pour savoir s'il peut.
3. **Contrôler `ui.widgets` dans `doctor` par une nouvelle route de lecture** — rejeté : aucun nouvel endpoint n'est justifié ; la route de commande existante suffit.
4. **Route enregistrée au premier usage du helper, payload `{}` conservé, capacité `ui.widgets/1`, masquage sur la valeur brute** — retenue.

## Décision

Option 4.

- **Module `deveye_sdk::ui`** (MIT) : `publish(tree)`, `is_observed()`, `builder(largeur, hauteur)`, constantes `ROUTE` et `CAPABILITY`, réexport des types de `deveye_core::ui`. `publish` remplace un `Arc<UiTree>` sous un `RwLock` (écriture immédiate, sans runtime), horodate l'arbre (`published_at_ms`), numérote `generation` si elle vaut 0 et retire la valeur des widgets secrets même pour un arbre construit sans le builder.
- **Observation** : `is_observed()` est vrai si la route a été lue dans les 2 dernières secondes. La route est enregistrée au premier appel de `publish` **ou** de `is_observed` (option 1). Une lecture qui arrive quand personne n'observait attend jusqu'à 500 ms la publication suivante : une app qui publie sous `is_observed` répond ainsi à la première requête avec un arbre frais ; sans arbre publié, la route répond `{ "error": … }`.
- **Capacité** : `/devtools/info` annonce `ui.widgets/1` dès que le helper est utilisé. Une capacité nouvelle est additive : `protocol_version` reste `1.0`.
- **Négociation** : le serveur envoie toujours `{}` (option 2). L'ajout de `accept_schema` et `max_nodes` est reporté tant qu'une capacité ne l'annonce pas et qu'un port n'expose pas les capacités au lecteur.
- **Seconde ligne de masquage** : le handler du SDK applique les clés déclarées (`Config::secret_keys`) aux valeurs structurées ; `RouteDomInspector` masque, sur la **valeur brute** que renvoie `dom` (sans la convertir : le format legacy reste legacy), la valeur de tout widget secret (`state.secret`, `secret`, rôle ou `kind` `password`) et applique le masquage par clés aux valeurs structurées ; une réponse qui n'est pas une liste de widgets reçoit le masquage par clés sur tout l'arbre. Les règles de clés sont reprises par une copie testée contre `shared-vectors-1.json`.
- **Lecture typée** : `RouteDomInspector::ui_tree()` rend le `UiTree` masqué (prérequis du snapshot par référence). Un arbre plus vieux que 2 s est signalé par un avertissement de journal.
- **`doctor`** : neuvième vérification, facultative : seulement si le SDK annonce `ui.widgets/1`, elle lit la route (une requête `POST /devtools/command`, sans effet sur l'app, option 3) et contrôle l'arbre. Un arbre qui viole le contrat (schéma inconnu, ids dupliqués ou vides, parent ou focus inconnu, cycle) est un **échec**, car l'app promet `ui.widgets/1` ; une route illisible, sans arbre publié, ou des avertissements (widget hors fenêtre) sont des avertissements.
- **Suite de conformité** : `UI-01` (arbre valide) et `UI-02` (aucune valeur publiée pour un widget secret), `N/A` sans la capacité, `SKIP` sans widget secret. `deveye-conformance` dépend de `deveye-core` (AGPL vers MIT, sens permis).
- **Release** : `publish` ne fait rien et `is_observed` est une fonction `const` toujours fausse. La preuve sur binaire (`scripts/verify-sdk-release-noop.sh`) cherche aussi la chaîne `ui.widgets`, avec témoin positif en debug ; `tests/release_noop.rs` vérifie à la compilation que `is_observed()` est constante.

**Justification** : l'enregistrement au premier usage évite l'interblocage ; garder `{}` ne casse aucun handler ; masquer sur la valeur brute fait de `dom` et de `snapshot` une seule vue sûre ; l'échec de `doctor` sur un arbre invalide reflète une promesse non tenue.

## Conséquences

**Positives**

- une app Rust publie son arbre en quelques lignes, sans coût tant que personne ne regarde ;
- `dom` ne renvoie plus la valeur d'un champ secret (note de version : changement de contenu de sécurité) ;
- les implémenteurs tiers ont `UI-01` et `UI-02` pour valider leur route.

**Coûts et risques**

- la lecture « à froid » peut durer jusqu'à 500 ms ;
- la règle des clés existe désormais en quatre copies (SDK, serveur, conformité, documentation), liées par `shared-vectors-1.json` ;
- `doctor` n'est plus strictement passif sur le SDK : il invoque la route `ui.widgets`, par convention en lecture seule.

## Notes

Écart de processus (revue 1.1) : cet ADR a été rédigé après les commits d'implémentation qu'il justifie, et la portée d'ADR-0016 avait d'abord été réécrite dans le corps de cet ADR accepté. Contraire à `docs/adr/README.md` (écrire avant ou avec le changement, ne pas modifier un ADR accepté) : sans effet sur le comportement, corrigé en ramenant ADR-0016 à un simple renvoi. Désormais : ADR avant ou avec le commit, et nouvel ADR plutôt que réécriture.

Application : `deveye-sdk/src/ui.rs` et `tests/ui_helper.rs`, `deveye-adapters/adapter-composite/src/ui_mask.rs` et `lib.rs` (`RouteDomInspector`), `deveye-server/src/doctor.rs` et `doctor_probe.rs`, `deveye-conformance/src/checks.rs`, `docs/wire-protocol.md` §3.7.
