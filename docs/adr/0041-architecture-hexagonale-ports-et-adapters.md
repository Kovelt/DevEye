# ADR-0041 — Architecture hexagonale : ports et adapters

- **Statut** : Accepté
- **Date** : 2026-06-02
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0011 (surface stable : ports et domaine), ADR-0013 (neutralité vis-à-vis des apps clientes)

## Contexte et problème

DevEye doit observer et piloter des apps graphiques très différentes : webview Chromium par CDP, webview WebKitGTK sans CDP, app native Wayland, app X11 native. Chaque technique d'observation (capture, entrées, DOM, logs) a ses propres bibliothèques, ses plates-formes et ses pannes. Mélanger le métier (« voir et agir sur une app ») avec le comment (CDP, D-Bus, X11, MCP) obligerait à toucher tout le code à chaque nouvelle cible.

## Facteurs de décision

- ajouter une cible sans refonte du reste ;
- tester le métier et l'orchestration sans I/O réel (adapters factices) ;
- plusieurs consommateurs possibles (un agent par MCP aujourd'hui, un autre transport demain) ;
- des dépendances lourdes ou propres à une plate-forme cantonnées à un seul crate ;
- une surface stable étroite à promettre en 1.0 (ADR-0011).

## Options envisagées

1. **Un crate unique** avec des modules par technique : simple au départ, mais le domaine finit par dépendre de `reqwest`, de D-Bus ou de X11, et tout se recompile ensemble.
2. **Couches classiques** (présentation, service, accès) : ne dit rien du sens des dépendances entre techniques interchangeables.
3. **Architecture hexagonale** : un domaine pur, des ports (traits Rust) qui décrivent ce dont le métier a besoin, un adapter par technologie, un binaire qui compose le tout.

## Décision

Option 3, en workspace Cargo multi-crates. Le sens des dépendances pointe toujours vers l'intérieur :

```
deveye-server -> adapters (adapter-*) -> deveye-ports -> deveye-core
```

- `deveye-core` : domaine pur (types, commandes, prédicats), sans dépendance technique ;
- `deveye-ports` : les traits (douze aujourd'hui) et leurs erreurs ;
- `adapter-*` : un crate par technologie ; **jamais un adapter ne dépend d'un autre adapter** ;
- `deveye-server` : binaire orchestrateur qui compose les adapters selon `--target` ;
- le serveur MCP est lui-même un adapter (`adapter-mcp-server`) qui consomme les ports.

**Justification** : seule cette structure rend une cible nouvelle (comme `x11`, ajoutée en novembre 2026) additive : un adapter, un câblage dans `deveye-server`, aucune retouche du domaine ni des autres adapters. Elle isole aussi les bibliothèques propres à une plate-forme et permet de tester le serveur MCP sur des ports factices.

## Conséquences

**Positives**

- ajouter une cible ou un port ne remet pas en cause les autres ;
- le domaine et les ports (les crates sous MIT, promesse de stabilité) restent petits et portables ;
- les règles se vérifient dans les manifestes : le graphe de dépendances est lisible.

**Coûts et risques**

- plus de crates et de traits à tenir à jour qu'un module unique ; ajouter une méthode sans implémentation par défaut à un port est un changement majeur (docs/stability.md) ;
- le composé (façades de repli, choix du backend) vit dans `deveye-server` et doit rester la seule pièce qui connaît plusieurs adapters.

## Notes

ADR rétroactif : la décision date du premier commit de l'historique du dépôt (`c88e0ea`, 2026-06-02), qui pose déjà `deveye-core`, `deveye-ports`, les adapters et le binaire orchestrateur. Elle est consignée en novembre 2026 pour que les contributeurs en comprennent la raison. Voir [docs/architecture.md](../architecture.md).
