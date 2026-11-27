# ADR-0064 — Masquage des secrets transversal et vecteurs partagés

- **Statut** : Accepté
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0016 (masquage par clés dans le SDK), ADR-0042 (posture dev-only, garde Host), ADR-0035 (suite de conformité), ADR-0052 (clés secrètes cumulatives), ADR-0062 (schéma `deveye.ui/1`)

## Contexte et problème

Le masquage des secrets (ADR-0016) est appliqué par le SDK à l'état et aux champs de logs. Trois sorties n'y échappent pas, et elles s'ajoutent : le résultat d'`eval_js`, les valeurs de retour des commandes, et la réponse de la route `ui.widgets`, que l'outil `dom` renvoie brute. Avec le schéma `deveye.ui/1` (ADR-0062) la route porte des valeurs de champs de saisie : un agent qui appelle `dom` lirait un mot de passe que `snapshot` masquerait. Par ailleurs la règle des clés sensibles existe en trois copies (SDK, suite de conformité, documentation) et la liste des hôtes de la garde `Host` en deux ; les SDK à venir (.NET, Godot) en feraient d'autres. Sans source commune, elles divergent.

## Facteurs de décision

- sûr par défaut, y compris pour un widget secret ;
- le SDK, exécuté dans l'app, est la seule couche qui sait ce qui est secret : il ne doit pas publier la valeur ; le serveur masque en seconde ligne ;
- un seul jeu de cas de test pour toutes les implémentations ;
- retirer de l'information sensible d'une sortie est défendable en 1.x (aucun contrat de valeur n'était promis, ADR-0016 listait ces sorties comme non filtrées) ;
- aucune dépendance nouvelle.

## Options envisagées

1. **Chaque implémentation garde ses listes** — dérive garantie.
2. **Générer le code de chaque langage depuis une liste** — outillage lourd pour une dizaine de cas.
3. **Un fichier de vecteurs JSON unique, consommé par les tests de chaque implémentation** — retenue.

## Décision

Option 3, et trois règles transversales.

- **Fichier unique** `docs/schemas/shared-vectors-1.json` : placeholder `<secret>`, clés sensibles exigées par la spécification (`required_secret_keys`), clés ajoutées par le SDK (`extended_secret_keys`), clés ordinaires qui ne doivent pas être masquées, clés déclarées par l'app, cas de masquage entrée / sortie, et garde `Host` (hôtes acceptés et rejetés, `{port}` substitué). Le SDK Rust et la suite de conformité le consomment par test ; tout SDK tiers doit le reprendre.
- **Au SDK** : un widget `state.secret` ou de rôle `password` n'a jamais de `value` publiée (`UiTreeBuilder::build`).
- **Au serveur** : le lecteur masque la valeur de tout widget secret reçu et applique le masquage par clés (ADR-0016) aux valeurs de type objet ; **le même masquage s'applique à la sortie brute de l'outil `dom`** (lot suivant de la 1.1). **Amendement (revue 1.1)** : le masquage ne s'applique qu'aux réponses reconnues comme un arbre de widgets (liste racine ou champ `widgets`) ; toute autre forme est renvoyée intacte, pour ne pas altérer la sortie de `dom` d'un handler quelconque. Pour un widget secret, `value` et `text` (format legacy) sont masqués, les enfants imbriqués (`children`) sont parcourus, et la règle « secret » est la même partout (drapeau `state.secret` ou `secret`, rôle `password` sans égard à la casse ni aux espaces). `invoke_command` passe par le même masquage quand la route est `ui.widgets` (décorateur `MaskingCommandRouter`) ; les sorties d'`eval_js` et des autres commandes restent hors filtre, comme documenté.
- **Hors périmètre** : le masquage reste un filet par noms de clés et par drapeau, pas une garantie.

**Justification** : seul un fichier partagé empêche la dérive entre implémentations sans outillage ; le double masquage (SDK puis serveur) couvre une app tierce qui publierait la valeur d'un champ qu'elle déclare secret.

## Conséquences

**Positives**

- une modification du masquage ou de la garde `Host` casse le test de chaque implémentation qui s'en écarte ;
- les SDK tiers ont une référence exécutable.

**Coûts et risques**

- un client qui lisait la valeur d'un champ secret par `dom` ne la reçoit plus : changement de contenu de sécurité, classé « mineure » dans `docs/stability.md` (durcissement) et à signaler en note de version (CHANGELOG, `Security`) ;
- les tests lisent un fichier hors du crate : ils sont ignorés dans une archive crates.io (la suite complète tourne dans le dépôt).

## Notes

Application : `docs/schemas/shared-vectors-1.json`, `deveye-sdk/src/redact.rs` et `deveye-sdk/src/lib.rs` (tests), `deveye-conformance/src/checks.rs` (tests), `deveye-core/src/ui.rs` (`strip_secret_values`, `mask_secret_values`). Le masquage de la sortie de `dom` et du lecteur serveur est livré par le lot « helper SDK et lecture serveur ».
