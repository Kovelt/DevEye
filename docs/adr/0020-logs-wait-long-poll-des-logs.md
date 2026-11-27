# ADR-0020 — `logs_wait` : long-poll des logs et niveau invalide en `invalid_params`

- **Statut** : Proposé
- **Date** : 2026-11-27
- **Décideurs** : Morgan Voltz, mainteneur
- **ADR liés** : ADR-0011 (surface stable), ADR-0018 (portage depuis l'ancienne lignée)

## Contexte et problème

Pour « agir puis attendre l'effet dans les logs », l'agent n'avait que `logs_tail` (drain ponctuel, à boucler côté agent) et `wait_for_log`, qui attend un message précis mais **consomme** les logs qu'il ignore sans les rendre : l'agent perd les erreurs inattendues arrivées pendant l'attente. De plus, un `level_min` inconnu y produisait une erreur interne (`-32603`) alors que c'est une faute d'appel. L'ancienne lignée GitHub avait un outil `logs_wait` qui répond à ces deux défauts.

## Facteurs de décision

- ne jamais perdre silencieusement un log drainé ;
- un paramètre invalide est une erreur d'appel (`invalid_params`), pas une panne ;
- un long-poll ne doit pas bloquer les autres outils (le gate d'exclusion mutuelle, M1) ;
- le nom des outils et la forme des réponses sont figés par la 1.0 (ADR-0011).

## Options envisagées

1. **Ne rien ajouter** : demander à l'agent de boucler sur `logs_tail`. Coûteux en appels et en tokens.
2. **Faire renvoyer les logs ignorés par `wait_for_log`** : change la forme de réponse d'un outil existant (`{ satisfied, elapsed_ms, detail }`).
3. **Nouvel outil `logs_wait`** : additif, sans toucher à `wait_for_log`.

## Décision

Option 3, plus la correction de `level_min` dans `wait_for_log`.

- `logs_wait { level_min?, timeout_ms?, limit? }` sonde le buffer toutes les 250 ms. Il rend la main dès qu'un log arrive (ou, avec `level_min`, dès qu'un log de niveau supérieur ou égal arrive), sinon à l'expiration de `timeout_ms` (défaut 5000, plafond 30000).
- Réponse : `{ events, timed_out, truncated, total }`. `events` contient **tous** les logs drainés pendant l'attente, y compris ceux sous `level_min` ; `timed_out` vaut `true` si le filtre n'a pas été satisfait. `limit` (défaut 200) borne `events` aux plus récents, comme `logs_tail` ; `total` et `truncated` signalent le reste. Les clés `truncated` et `total` s'ajoutent à la forme `{ events, timed_out }` de l'ancienne lignée.
- Le paramètre s'appelle `level_min` (comme `wait_for_log`), non `min_level` comme dans l'ancienne lignée : une seule orthographe dans la surface.
- Le gate n'est tenu que le temps d'un sondage, relâché pendant la pause.
- Un `level_min` inconnu donne `invalid_params`, dans `logs_wait` **et** dans `wait_for_log` (correction), via un `parse_log_level` commun qui réutilise la désérialisation de `LogLevel`.

**Justification** : l'option 3 comble le manque sans changer la forme d'un outil existant. Le timeout d'un `timed_out: true` n'est pas une erreur MCP (`isError` reste faux) : l'absence de log est une information, pas un échec, contrairement aux `wait_*` dont le but est de satisfaire un prédicat.

## Conséquences

**Positives**

- l'agent récupère les erreurs inattendues au lieu de les perdre ;
- plus de boucle côté agent pour suivre les logs après une action.

**Coûts et risques**

- deux outils proches (`wait_for_log` et `logs_wait`) : la description de chacun renvoie à l'autre ;
- `logs_wait` draine le buffer partagé : deux agents ou outils qui drainent en parallèle se partagent les événements ;
- un statut `Proposé` : le plafond de 30 s et le nom `logs_wait` engagent la surface stable au-delà de la décision D05.
