# ADR-0024 — Sérialisation sans panic et arguments `invoke_command` stringifiés

- **Statut** : Proposé
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0011 (surface stable), ADR-0018 (portage depuis l'ancienne lignée)

## Contexte et problème

Deux défauts voisins dans la couche des outils MCP et des adapters.

1. La règle 6 du projet interdit `unwrap`, `expect` et `panic!` en production hors justification. Plusieurs adapters sérialisaient pourtant une chaîne en littéral JS par `serde_json::to_string(..).expect("string serialise")` (outils `click_selector` et `click_text`, commande `Navigate` de l'adapter CDP, pont SDK, scripts KWin) et l'attache à une cible CDP faisait de même. Sérialiser un `&str` ne peut pas échouer, mais un `expect` non justifié viole la règle et masque la raison de l'invariant.
2. Certains clients MCP sérialisent l'argument objet d'`invoke_command` en **chaîne** JSON (`"{\"page\":\"Agenda\"}"` au lieu de `{"page":"Agenda"}`). Le handler de l'app recevait alors une chaîne et échouait. Le correctif existait dans l'ancienne lignée et s'est perdu.

## Facteurs de décision

- aucune panique possible dans le chemin de production ;
- pas d'API publique ajoutée au gel 1.0 pour un détail interne ;
- ne pas altérer un argument qui est réellement du texte.

## Options envisagées

Pour la sérialisation :

1. **Helper public partagé** (dans `deveye-ports` ou `deveye-core`) : une seule copie, mais une fonction de plus dans une bibliothèque MIT figée.
2. **Helper privé par crate**, d'une ligne, fondé sur `serde_json::Value::from(s).to_string()` : l'affichage d'un `Value` ne peut pas échouer pour une chaîne en mémoire ; pas de `expect`, pas de valeur de repli silencieuse.
3. **`unwrap_or_default()`** : sans panic, mais une erreur impossible produirait un littéral vide qui change le sens du script en silence.

Pour `invoke_command` :

4. **Ne rien faire** : le client doit envoyer un objet.
5. **Relire toute chaîne qui est du JSON valide**, quelle que soit sa forme.
6. **Relire seulement une chaîne qui contient un objet ou un tableau JSON.**

## Décision

Options 2 et 6.

- Les crates `adapter-mcp-server` et `adapter-dioxus-signals` portent chacun un `js_string` privé ; les adapters CDP et KWin appellent directement `Value::from(..)`. `Target.attachToTarget` utilise `Value::to_string()` sur le message. Aucune API publique n'est ajoutée.
- `invoke_command` relit un argument `Value::String` si, et seulement si, son contenu est un objet ou un tableau JSON valide. Les chaînes ordinaires, ou qui ressemblent à un scalaire (`"42"`, `"true"`), sont transmises inchangées : un argument voulu comme texte n'est pas transformé en nombre.
- Reste volontairement justifié : l'`expect("session ouverte ci-dessus")` d'`adapter-x11`, un invariant de la ligne précédente (la session vient d'être posée), et l'initialisation du runtime du SDK.

**Justification** : l'option 2 supprime tout chemin de panique sans élargir la surface stable ; l'option 6 corrige le cas réel des clients qui stringifient sans corrompre les chaînes légitimes (l'option 5 transformerait `"42"` en `42`).

## Conséquences

**Positives**

- plus aucun `expect` de sérialisation dans les adapters et les outils MCP ;
- `invoke_command` fonctionne avec les clients qui stringifient leurs arguments.

**Coûts et risques**

- un `js_string` d'une ligne existe en deux exemplaires : à factoriser si un troisième besoin apparaît dans un crate public ;
- un argument qui est un objet **voulu comme texte** (une chaîne JSON destinée telle quelle au handler) est désormais relu comme objet : cas jugé improbable, à signaler dans la documentation de la commande si elle le prévoit ;
- les cinq `RwLock::unwrap` du SDK (poison de verrou) ne sont pas traités ici : ils relèvent du SDK.
