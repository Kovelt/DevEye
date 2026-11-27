# ADR-0032 — Sous-commande `doctor` en lecture seule

- **Statut** : Proposé
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0011 (surface stable : une sous-commande s'ajoute à la CLI), ADR-0031 (validation des flags), ADR-0033 (version affichée), ADR-0009 (lints et exception unsafe de KWin EIS)

## Contexte et problème

Les pannes de mise en place de DevEye sont toujours les mêmes et se découvrent tard : binaire périmé, port du SDK pris par autre chose, filtre CDP, `KWIN_SCREENSHOT_NO_PERMISSION_CHECKS` absent de l'environnement de KWin, interface EIS indisponible, display X privé non lancé. Aujourd'hui l'agent ou l'humain le découvre en appelant un outil qui renvoie une erreur réseau ou `Unsupported`. Le mode « composition validée » (sortie silencieuse sans `--serve-mcp`) ne vérifiait rien de cela et est retiré (ADR-0031).

## Facteurs de décision

- aucune action intrusive : un diagnostic ne doit rien démarrer, créer, écrire ni déclencher de demande d'autorisation ;
- un statut et un conseil actionnable par vérification ;
- un code de sortie exploitable par un script ou un agent ;
- gravité dépendante de la cible : le SDK est bloquant pour `sdk`, pas pour `auto` ;
- partie décisionnelle testable sans machine particulière ;
- pas de nouvelle dépendance externe.

## Options envisagées

1. **Ne rien ajouter** : chaque panne reste découverte à l'usage.
2. **Un script shell** : non testable, divergent du binaire, ne partage ni la garde X11 ni le client du SDK.
3. **Sous-commande `deveye-server doctor [--target …]`**, avec les options de cible partagées.

## Décision

Option 3. `doctor` est une sous-commande unique, en lecture seule, qui réutilise les options globales (`--target`, `--sdk-url`, `--cdp-url`, `--display`, `--screencap-window`…) et la validation par cible (ADR-0031).

Sondes (toutes des lectures) : variables `XDG_SESSION_TYPE` et `XDG_CURRENT_DESKTOP` ; processus KWin du même utilisateur dans `/proc` et sa variable `KWIN_SCREENSHOT_NO_PERMISSION_CHECKS` (repli sur l'environnement de `doctor` quand `/proc/<pid>/environ` est protégé) ; interface EIS par une **introspection** D-Bus (`busctl --user introspect`, jamais `connectToEIS`, qui ouvre une session de saisie) ; `Xvfb` dans le `PATH` ; garde d'isolation d'`adapter-x11` (`check_display`) et socket `/tmp/.X11-unix/X<n>` ; `GET /devtools/info` du SDK (version, protocole, capacités) ; `GET /json/version` du CDP ; connexion TCP pour distinguer « port occupé par autre chose » de « rien n'écoute ». Chaque sonde a un délai de 3 s.

Décision : trois statuts de gravité (`[ATTENTION]` utile mais non bloquant, `[ÉCHEC]` prérequis bloquant, `[SANS OBJET]`) selon une table de prérequis par cible (`need` dans `doctor.rs`). Code de sortie : **1 s'il y a au moins un échec, 0 sinon** (les avertissements décrivent une capacité réduite, jamais une panne) ; 2 pour une erreur d'usage (clap). Un fait non vérifiable (environnement de KWin illisible, `busctl` absent) est un avertissement, jamais un échec : le doute ne bloque pas. L'architecture sépare la collecte (`doctor_probe.rs`, impure) de la décision et du rendu (`doctor.rs`, pures, testées sur des faits fabriqués).

Le coût en dépendances : `deveye-server` dépend désormais de `deveye-sdk-client` (crate du workspace, MIT, qui n'ajoute aucun crate externe), de `reqwest` (déjà dans le workspace et le graphe) et de trois features de `tokio` déjà compilées (`net`, `process`, `time`). Aucune dépendance externe nouvelle.

**Justification** : une sous-commande du binaire réutilise exactement la garde, le client et les constantes que le serveur utilise, donc ne peut pas diverger ; la séparation faits/décision permet de tester chaque cible sans KWin ni X. L'introspection plutôt que la connexion EIS évite toute fenêtre d'autorisation.

## Conséquences

**Positives**

- une commande répond à « pourquoi ça ne marche pas ? » avec le correctif ;
- utilisable en CI ou par un agent (code de sortie) ;
- tests de décision et de rendu (`doctor::tests`), de sondes (`doctor_probe::tests`) et de bout en bout du binaire (`tests/cli.rs`).

**Coûts et risques**

- le processus lancé est `busctl`, en lecture (`introspect`) : sur une machine sans `busctl`, l'accès EIS reste « non vérifiable » ;
- `doctor` sonde des ports locaux : sans effet de bord, mais visible dans les journaux d'un service qui les écouterait ;
- le texte du diagnostic n'est pas couvert par la stabilité ; les codes de sortie le sont.

## Notes

Application : `deveye-server/src/{doctor,doctor_probe,main,cli}.rs`, `deveye-server/tests/cli.rs`, `docs/usage.md` (« Diagnostiquer avant de lancer »).
