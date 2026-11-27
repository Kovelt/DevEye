# Politique de sécurité

DevEye est un outil de **développement**. Ce document dit contre quoi il se protège, ce qu'il garantit et comment on le vérifie, ce qu'il ne garantit pas, et comment signaler une faille.

## Signaler une vulnérabilité

**Ne signalez pas une faille dans une issue publique.** Utilisez le **signalement privé de vulnérabilité de GitHub** : onglet **Security** du dépôt, puis **Report a vulnerability** (« Signaler une vulnérabilité »). Le signalement n'est visible que des mainteneurs et de vous, et la discussion, la correction et l'avis publié se font dans un espace privé.

Si le signalement privé n'est pas disponible (le réglage est à activer par le mainteneur dans Settings, Code security, Private vulnerability reporting), ouvrez une issue intitulée « **Demande de contact (sécurité)** » **sans aucun détail technique** : ni composant, ni étapes, ni preuve de concept. Un mainteneur répond avec un canal privé, et l'issue est supprimée ensuite.

Merci d'indiquer : la version (`deveye-server --version`), la cible utilisée, les étapes pour reproduire, et l'impact que vous constatez. Les mainteneurs s'engagent à accuser réception sous sept jours, à évaluer le signalement de bonne foi, à vous tenir informé de l'avancement et à vous créditer dans l'avis si vous le souhaitez. Il s'agit d'un projet maintenu par une personne : les délais de correction dépendent de la gravité et ne sont pas contractuels.

Ce canal est réservé aux **failles de sécurité**. Un comportement abusif d'un participant se signale selon le [code de conduite](CODE_OF_CONDUCT.md#signalement) ; un bogue ordinaire, par une issue.

Versions suivies : la dernière version 1.x publiée reçoit les correctifs de sécurité. Les avis connus sur les dépendances sont surveillés par `cargo deny` en CI ([ADR-0008](docs/adr/0008-montees-de-version-avis-rustsec.md)) ; les exceptions sont consignées et motivées ([ADR-0006](docs/adr/0006-exception-rustsec-chrono.md), [ADR-0007](docs/adr/0007-exception-rustsec-quick-xml.md)).

## Modèle de menace

**Ce qu'on protège.** Le poste d'un développeur qui lance son app en développement, avec le SDK embarqué et `deveye-server` branché à un agent. Le SDK ouvre un canal d'inspection et d'évaluation de JavaScript : c'est ce canal qu'on garde fermé à tout ce qui n'est pas la machine locale, et absent de tout binaire de production.

**Adversaires pris en compte.**

| Adversaire | Exemple | Réponse |
|---|---|---|
| Un binaire de production qui embarque le SDK par erreur | un build release avec la feature activée | `init` est un no-op en release ; preuve sur binaire |
| Le réseau | un autre hôte du réseau local | écoute sur `127.0.0.1` uniquement |
| Une page web hostile dans le navigateur du développeur | rebond DNS, requête inter-origine vers `localhost:31337` | garde `Host`, CORS refusé, drain des logs refusé aux requêtes inter-origine |
| Une app qui se trompe de cible | pilotage de la session graphique vivante par XTEST | garde d'isolation de la cible `x11`, sans contournement |
| Un client qui sature l'app | file d'évaluation, corps énormes | file bornée (32), corps bornés à 1 Mio, tampon de logs borné |

**Hors du modèle** : un processus local malveillant de **votre utilisateur** (il peut déjà tout faire de ce que fait le SDK), un poste partagé entre plusieurs personnes, une machine compromise.

## Garanties et leurs tests

| Garantie | Comment elle est vérifiée |
|---|---|
| **Le SDK n'ouvre aucun port en release.** `deveye_sdk::init` est un no-op hors `debug_assertions`, même sans garde côté appelant. | `deveye-sdk/tests/release_noop.rs` (test release) et `scripts/verify-sdk-release-noop.sh` : un binaire release de l'exemple `release_probe` ne contient aucune trace du serveur HTTP (routes `/devtools/`, en-tête de version), avec un binaire debug comme témoin positif. Job CI « SDK no-op en release » ([ADR-0039](docs/adr/0039-tests-de-bout-en-bout-et-preuves-dev-only-en-ci.md)). |
| **Boucle locale seulement.** Le serveur lie `127.0.0.1`, jamais `0.0.0.0`. | Adresse codée en dur dans `deveye_sdk::init`, relue en revue de code. **Aucun test n'asserte l'adresse d'écoute elle-même** : les tests du SDK et la suite de conformité s'y connectent par la boucle locale. |
| **Garde `Host`.** `403` si `Host` n'est pas `localhost`, `127.0.0.1` ou `[::1]` (port numérique optionnel), avant toute route. | `deveye-sdk/tests/security.rs` (`host_guard_rejects_foreign_and_missing_port_hosts`, `host_guard_accepts_loopback_spellings`) ; tests unitaires de `host_is_local`. |
| **CORS refusé.** Aucune origine n'est autorisée. | `deveye-sdk/tests/security.rs` (`cors_is_deny_all`). |
| **Bornes.** Corps ≤ 1 Mio (`413`), file d'évaluation de 32 requêtes (`503` si pleine), tampon de logs de 1 024 événements (les plus anciens abandonnés). | `deveye-sdk/tests/security.rs` (`eval_queue_saturation_answers_503`), tests du wire protocol. |
| **La cible `x11` ne touche jamais la session vivante.** `--display` obligatoire sans repli sur `$DISPLAY` ; refus d'un display non local, d'un numéro < 100, de la session courante, du socket d'un autre uid, de tout serveur annonçant `XWAYLAND` ; aucune option pour lever ces refus. | Tests unitaires de la garde (`adapter-x11/src/guard.rs`) sans serveur X ; tests contre un `Xvfb` privé et test de bout en bout du binaire, bloquants en CI ([ADR-0044](docs/adr/0044-cible-x11-isolee-par-construction.md)). |
| **Masquage des secrets par clés** dans l'état et les champs de logs. | Tests du SDK (`redact`, `wire_protocol`). Voir les limites ci-dessous. |
| **Version du protocole** vérifiée côté client. | `deveye-sdk-client/tests/protocol_version.rs` ; `deveye-server doctor` signale une version majeure incompatible. |
| **Chaîne d'approvisionnement.** Avis RustSec, licences et sources contrôlés ; actions GitHub épinglées par SHA ; permissions CI minimales ; SBOM CycloneDX. | Job CI `cargo deny check`, job SBOM, workflow de release à permissions séparées ([ADR-0038](docs/adr/0038-distribution-archives-github-et-cargo-install.md)). |
| **Code `unsafe`.** Interdit partout (`forbid`) sauf un unique bloc localisé et justifié dans l'adapter KWin EIS. | Lints du workspace ([ADR-0009](docs/adr/0009-lints-de-workspace-et-exception-kwin-eis.md)). |

## Limites : ce que DevEye ne garantit pas

- **Pas d'authentification.** Tout processus de la machine peut parler au SDK sur la boucle locale. Le modèle suppose un poste de développement de confiance, pas une machine partagée.
- **`eval_js` évalue du code arbitraire** dans la page ou l'app. C'est sa fonction. Le résultat n'est **pas** filtré.
- **Le masquage des secrets est un filet, pas une garantie.** Il agit par **noms de clés** dans l'état et les champs structurés des logs. Il ne couvre pas : le résultat d'`eval_js`, les valeurs de retour de `invoke_command` (hors réponse de la route `ui.widgets`, masquée en seconde ligne par le serveur), le texte libre des messages de log, le contenu des champs de log formatés par `Debug` (`?valeur`), ni un secret rangé sous un nom de clé neutre non déclaré. Ne publiez pas de secret dans l'état ([ADR-0016](docs/adr/0016-masquage-des-secrets-par-cles-dans-le-sdk.md)).
- **Le masquage des champs de `snapshot` / `observe elements` est heuristique.** Sur une page web, le script du pont ne rend pas la valeur d'un champ qu'il reconnaît comme secret (type `password`, champ vu un jour en `password`, `autocomplete` sensible, `name`, `id`, libellé ou `placeholder` évoquant un secret, attribut `data-deveye-secret`). Un champ sans ces indices (un `text` nommé `champ1` qui reçoit un secret) est rendu en clair, jusqu'à 200 caractères, au modèle. Marquer `data-deveye-secret` les champs sensibles de l'app ; côté widgets natifs, déclarer `secret` dans l'arbre `ui.widgets` ([ADR-0074](docs/adr/0074-pont-js-d-elements-pour-webkitgtk.md), [ADR-0064](docs/adr/0064-masquage-des-secrets-transversal-et-vecteurs-partages.md)).
- **Captures et logs vont à l'agent.** Ce que l'écran affiche, ce que le DOM contient et ce que les logs disent peut contenir des données sensibles ; ils sont envoyés au client MCP, donc au modèle. Ne pilotez pas une app qui affiche des secrets réels.
- **Les entrées sont réelles.** Les cibles `kwin-eis` (vrais événements dans la fenêtre active) et `x11` (sur un serveur privé) injectent de vrais événements : un focus détourné (notification, dialogue) peut envoyer des frappes à une autre fenêtre. Le réglage `KWIN_SCREENSHOT_NO_PERMISSION_CHECKS` de la cible `kwin-eis` désactive un contrôle de KWin pour la session : ne le posez que sur une machine de développement de confiance.
- **`clipboard_get` lit le vrai presse-papiers** de l'utilisateur (mots de passe copiés, données d'autres applications), hors de tout masquage. L'outil est annoté « monde ouvert » pour que le client demande confirmation ; un client qui l'approuve automatiquement expose ce contenu au modèle ([ADR-0050](docs/adr/0050-clipboard-get-monde-ouvert-et-limites-de-la-classification.md)).
- **`assert_visual` écrit des fichiers** : `baseline_path` est confiné à `DEVEYE_BASELINE_DIR` (défaut : répertoire courant du serveur), en `.png`, sans `..` ni lien symbolique sortant, et refuse d'écraser un fichier qui n'est pas un PNG lisible ([ADR-0049](docs/adr/0049-confinement-des-baselines-d-assert-visual.md)). Le versionnement des baselines par git reste le filet contre une baseline écrasée à tort.
- **Un SDK écrit hors de ce dépôt** n'est sûr que s'il respecte le contrat ([wire-protocol.md](docs/wire-protocol.md)) ; la suite de conformité le vérifie, elle ne le certifie pas.
- **Linux seul** : aucune garantie sur d'autres plateformes ([docs/platforms.md](docs/platforms.md)).

## Bonnes pratiques pour l'intégrateur

1. Déclarer le SDK en dépendance **optionnelle** derrière une feature de développement, et garder `#[cfg(debug_assertions)]` autour de `init` ([docs/integration.md](docs/integration.md)).
2. Ne jamais publier de secret via `set_state` ou une commande ; déclarer secret tout champ de saisie sensible publié par `deveye_sdk::ui` (sa valeur n'est alors jamais publiée, et `dom` la masque en seconde ligne) ; déclarer les clés propres à l'app avec `Config::with_secret_keys`.
3. Un `DEVEYE_PORT` par app quand plusieurs tournent ; garder `--sdk-url` sur `127.0.0.1`.
4. Lancer la cible `x11` sur un `Xvfb` dédié, jamais sur un serveur que vous utilisez.
5. Garder `deveye-server` et les bibliothèques à jour.
