# Spike AT-SPI2 — `accessibility_tree` pour les cibles Uno/Skia (kwin-eis)

> **Contexte** : candidat D2.4 de `docs/audit-2026-07-04.md` — spike obligatoire avant ticket. Question posée : l'arbre AT-SPI2 (`org.a11y.atspi` via D-Bus) peut-il servir d'équivalent structurel du DOM (rôles, noms accessibles, états, bounds) pour une app native Uno/Skia sous `kwin-eis`, là où `eval_js` est `Unsupported` et où seule la capture pixel existe aujourd'hui comme modalité d'inspection ?
> **Investigation système uniquement** — aucun fichier du dépôt DevEye modifié, aucun commit.

---

## Verdict

**NO-GO — infaisable en l'état actuel de la stack (Uno.Sdk 6.5.36 / `net10.0-desktop` Skia X11).**

Ce n'est pas une limite de configuration ni un réglage manquant côté DevEye : le head desktop Skia d'Uno Platform **ne se connecte tout simplement pas** au bus AT-SPI, même quand l'accessibilité système est activée. Aucun code AT-SPI n'existe dans les assemblies Uno inspectées. La méthode d'introspection elle-même (D-Bus → `org.a11y.atspi.Accessible`/`Component`) est validée et fonctionne parfaitement sur une vraie app Qt (preuve ci-dessous) — le problème n'est donc pas la méthode, c'est l'absence de bridge côté Uno pour ce head précis.

Le ticket `accessibility_tree` ne doit **pas** être créé maintenant. Condition de réouverture précise en fin de document.

---

## 1. Méthode d'introspection AT-SPI2 — validée sur une vraie app (Qt/Dolphin)

### 1.1 Le bus a11y existe et répond

`org.a11y.Bus` est présent sur le bus session (`busctl --user list`), avec un service `GetAddress` qui renvoie l'adresse d'un **second bus D-Bus dédié** :

```
$ busctl --user call org.a11y.Bus /org/a11y/bus org.a11y.Bus GetAddress
s "unix:path=/run/user/1000/at-spi/bus_0"
```

C'est un piège pour qui découvre AT-SPI : les appels doivent cibler **cette adresse-là** (`--address=...`), pas le bus session classique. `org.a11y.atspi.Registry` (process `at-spi2-registryd`) y vit et sert de racine (`GetChildren` sur `/org/a11y/atspi/accessible/root` liste les apps enregistrées).

### 1.2 Mécanisme d'activation à la demande — le point le plus important de ce spike

À froid, très peu d'apps sont enregistrées sur le bus a11y (`ksmserver`, quelques daemons système). **`plasmashell` lui-même n'y est pas connecté par défaut.** La raison : la propriété `org.a11y.Status.IsEnabled` (sur `org.a11y.Bus`) valait `false` sur cette machine — c'est le signal que Qt/GTK surveillent pour décider d'activer ou non leur bridge AT-SPI interne.

Bascule live testée :

```
$ busctl --user set-property org.a11y.Bus /org/a11y/bus org.a11y.Status IsEnabled b true
$ busctl --address="unix:path=/run/user/1000/at-spi/bus_0" --user list
```

Résultat : **~20 process supplémentaires** apparaissent instantanément sur le bus a11y (kwin_wayland, plasmashell, kwalletd6, kactivitymanagerd, kiod6, dolphin, plasma-discover, DiscoverNotifier, nextcloud...) — la totalité de la session Plasma bascule en mode accessible dès que le flag passe à `true`, sans redémarrage. C'est la preuve que le mécanisme d'activation est bien ce flag global (ou l'équivalent `QT_LINUX_ACCESSIBILITY_ALWAYS_ON`), **et non** un réglage par app.

**Conséquence pour un futur outil `accessibility_tree`** : si jamais une cible expose AT-SPI, l'outil devrait forcer `IsEnabled=true` avant la première requête (idempotent, réversible, mais **effet de bord global sur la session utilisateur** — à documenter si le ticket voit le jour un jour).

État remis à `false` en fin de spike (valeur trouvée au départ).

### 1.3 Arbre navigable — preuve concrète sur Dolphin (Qt 6.11.1)

Une fenêtre Dolphin fraîchement lancée (`dolphin --new-window`), avec `IsEnabled=true` :

```
$ busctl --address=$ATSPI_BUS --user call ":1.63" /org/a11y/atspi/accessible/root \
    org.a11y.atspi.Accessible GetChildren
a(so) 1 ":1.63" "/org/a11y/atspi/accessible/2147483667"

$ busctl --address=$ATSPI_BUS --user get-property ":1.63" \
    /org/a11y/atspi/accessible/2147483667 org.a11y.atspi.Accessible Name
s "Dossier Personnel — Dolphin"      # Role = "frame", 8 enfants directs
```

Descente d'un niveau (panneaux nommés, rôles cohérents) :

```
2147483668 -> role="filler"   name=""
2147483669 -> role="panel"    name="Informations"
2147483670 -> role="panel"    name="Dossiers"
2147483671 -> role="panel"    name="Terminal"
2147483672 -> role="panel"    name="Emplacements"
2147483673 -> role="menu bar" name=""
2147483674 -> role="tool bar" name="Barre principale"
2147483675 -> role="filler"   name=""
```

Et les **bounds** via l'interface `Component` (le point le plus incertain a priori, vu les restrictions Wayland sur les coordonnées globales) :

```
$ gdbus call --session -d ":1.63" -o /org/a11y/atspi/accessible/2147483677 \
    -m org.a11y.atspi.Component.GetExtents 0   # 0 = SCREEN
((6, 6, 34, 34),)
$ ... GetExtents 1                              # 1 = WINDOW
((6, 6, 34, 34),)
```

Bounds cohérents et non nuls sur les boutons de la barre d'outils (position + taille en pixels). **Réserve notée** : sous ce compositeur Wayland (KWin), `SCREEN` et `WINDOW` renvoient la même valeur — les coordonnées absolues multi-écran ne sont probablement pas fiables telles quelles (limite connue d'AT-SPI sous Wayland, l'absence d'API compositeur pour la position globale d'un top-level). Un futur outil devrait composer `bounds fenêtre-relatifs` + `origine fenêtre` (déjà disponible côté DevEye via `window_geometry_get`, cf. D2.2 du même audit) plutôt que de faire confiance à `CoordType::Screen`.

**Bilan §1** : la méthode fonctionne intégralement — bus, activation, arbre, rôles, noms, états (`GetState`), bounds. Sur une app Qt/GTK classique, un outil `accessibility_tree` serait un **M**, pas un **L** : la donnée est là, il suffit de la sérialiser.

---

## 2. Le cas Uno/Skia (`Atlas.App`, cible réelle de `kwin-eis`)

### 2.1 Contexte établi

`Atlas.App` (solution `atlas`, `~/Dropbox/Nextcloud/Kovelt/Projets/atlas`) est le client Uno Platform visé par `kwin-eis`. `Atlas.App.csproj` :

```xml
<TargetFrameworks>net10.0-browserwasm;net10.0-desktop</TargetFrameworks>
<UnoFeatures>
  SkiaRenderer;
</UnoFeatures>
```

`UnoFeatures=SkiaRenderer` s'applique **sans condition de TargetFramework** — donc aux deux têtes, y compris WASM (rendu CanvasKit, pas DOM natif ; hors périmètre de ce spike mais notable). La tête desktop tourne via `Uno.UI.Runtime.Skia.X11` (mémoire `deveye-platform-constraints` : `.UseX11()`), confirmée au runtime (§2.3).

Le XAML d'Atlas.App **utilise réellement** la sémantique d'accessibilité WinUI (`AutomationProperties.Name`, `AutomationProperties.AccessibilityView`, ex. `SectionCard.xaml`, `CardSkeleton.xaml`) — l'intention d'accessibilité existe côté app (cohérent avec `docs/06-accessibilite.md` du projet atlas, DoD bloquante). La question est uniquement : **qu'est-ce qui consomme cette sémantique sous Linux desktop ?**

### 2.2 Analyse statique des binaires Uno — zéro trace AT-SPI

Un build Debug `net10.0-desktop` existant (25/06) a été inspecté sans recompilation :

```
$ strings Uno.UI.dll Uno.UI.Runtime.Skia.X11.dll Uno.UI.Runtime.Skia.dll \
          Uno.dll Uno.UI.Composition.dll | grep -i "atspi\|a11y\|AT-SPI"
(aucune occurrence)
```

En revanche, l'arbre `AutomationPeer` (l'abstraction cross-platform WinUI) **existe bien** dans `Uno.UI.dll` (`AppBarAutomationPeer`, `AutomationPeerAddToSelection`, etc.) — Uno construit donc en mémoire un arbre sémantique. Mais le pont vers l'OS est explicitement désactivé par substitution du linker (ILLink) :

```
get_Is_Microsoft_UI_Xaml_Automation_Provider_IRawElementProviderSimple_Available
  ... body="stub" value="false" feature="Is_Microsoft_UI_Xaml_Automation_Provider_IRawElementProviderSimple_Available" featurevalue="false"
```

Traduction : l'interop UI Automation (`IRawElementProviderSimple`, l'équivalent Windows d'AT-SPI côté Microsoft) est **compilée à `false`** sur ce head — cohérent avec le fait que UIA est une techno Windows, pas Linux. Il n'existe **aucun équivalent Linux** substitué à la place : pas de `NSAccessibility` (macOS, hors sujet), pas d'AT-SPI. L'arbre `AutomationPeer` est donc aujourd'hui une donnée orpheline sous ce head : peuplée par le framework, consommée par personne sur Linux desktop.

### 2.3 Test dynamique — lancement réel du binaire, aucune inscription au bus a11y

Pour éviter un build long (solution complète EF Core/ASP.NET/MediatR — hors périmètre du spike), le binaire natif déjà compilé (`bin/Debug/net10.0-desktop/Atlas.App`, apphost ELF) a été lancé directement, **sans recompilation**, avec `org.a11y.Bus IsEnabled=true` (le déclencheur validé en §1.2) :

```
$ ./Atlas.App &            # process vivant, fenêtre X11/XWayland créée (logs EWMH non fatals)
$ busctl --address=$ATSPI_BUS --user list | grep -i atlas
(aucune ligne)
$ grep -i "DBUS\|ATSPI\|A11Y" /proc/<pid>/environ
DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus     # bus session normal
(aucune variable AT-SPI)
$ grep -w <inode-des-sockets-ouverts> /proc/net/unix
/tmp/dotnet-diagnostic-...-socket                          # diag .NET
(paires de sockets D-Bus non nommées — connexion au bus session uniquement)
```

Le process est resté vivant plusieurs secondes (pas un crash), a ouvert son socket vers le bus **session** habituel (`/run/user/1000/bus`, pour ses propres besoins D-Bus internes — probablement le remote-control d'Uno / futures notifications desktop), mais **n'a, à aucun moment, ouvert de connexion vers `/run/user/1000/at-spi/bus_0`**. C'est la confirmation dynamique, en conditions réelles, du constat statique du §2.2 : même avec le déclencheur système activé, ce head n'essaie même pas de parler AT-SPI.

Nettoyage effectué : process de test tué, fenêtre Dolphin de test fermée, `IsEnabled` remis à `false` (état initial constaté avant le spike).

### 2.4 Synthèse — pourquoi c'est structurel, pas un réglage manquant

| Couche | État constaté |
|---|---|
| Bus a11y système | Présent, fonctionnel, activable (§1) |
| Méthode d'introspection D-Bus | Validée de bout en bout sur Qt (§1.3) |
| Sémantique déclarée côté app Uno (XAML) | Présente (`AutomationProperties.*`) |
| Arbre `AutomationPeer` en mémoire (Uno.UI) | Présent, peuplé |
| Bridge OS Linux (AT-SPI) consommant cet arbre | **Absent** — aucune chaîne, aucune classe, aucune connexion runtime |
| Bridge Windows (UIA) équivalent | Présent dans le code mais désactivé par le linker sur ce head (`false` en dur) |

Il ne s'agit donc pas d'un flag à activer côté DevEye ou côté atlas : **Uno Platform (6.5.36) n'a simplement pas encore implémenté de bridge AT-SPI pour son head Skia/X11 Linux**, alors qu'il en a pour WASM (DOM/ARIA — sous réserve, hors périmètre ici) et potentiellement Windows (UIA, désactivé ici car head Linux). C'est un gap connu de ce type de framework de rendu "canvas-only" multi-plateforme (le même problème existe historiquement chez Flutter Desktop vs Flutter Web/Mobile pour la même raison : un moteur de rendu Skia/CanvasKit ne crée pas de widgets natifs consommables par le bridge d'accessibilité de l'OS, il faut un pont dédié écrit à la main par le framework).

---

## 3. Coût si (un jour) le bridge existe côté Uno

Cette section documente le coût pour ne pas avoir à refaire l'analyse au prochain spike — **elle ne justifie pas un ticket aujourd'hui**, faute de donnée à lire côté cible.

- **Crate Rust** : [`atspi`](https://docs.rs/atspi) (pur Rust, zbus, pas de FFI/`libei`-like — confirme l'hypothèse de la mission). Existe en version maintenue (0.30.0 courant).
- **Piège de version décelé** : `atspi = "0.30"` tire **zbus 5.16** en dépendance transitive, alors que le workspace DevEye épingle `zbus = "4"` (racine `Cargo.toml:46`, utilisé par `adapter-interaction-kwin-eis`/`adapter-screencap-kwin`). Deux majors de zbus dans le même graphe = deux piles D-Bus non interopérables (impossible de partager une `Connection`), coût de compilation/binaire double, franchement à éviter.
- **Contournement testé** : `atspi = "=0.24"` (et jusqu'à `0.22`) dépend de **zbus 4.4.0**, compatible avec la contrainte `zbus = "4"` du workspace — cargo peut alors **unifier** sur une seule pile zbus 4.x partagée avec les adapters existants. Vérifié par `cargo tree` dans un scratch project (pas dans le workspace DevEye — aucun `Cargo.toml` du dépôt touché). C'est l'option à retenir si le spike est un jour refait GO : figer sur `0.22`–`0.24`, pas sur `latest`.
- **Effort d'implémentation si la cible expose l'arbre** : proxies `AccessibleProxy`/`ComponentProxy` fournis par la crate (pas de D-Bus à la main comme dans ce spike) → connexion + `GetChildren` récursif + `GetRoleName`/`GetState`/`GetExtents` par nœud. Le vrai risque n'est pas le binding Rust (M), mais (a) la profondeur/volume de l'arbre pour une UI riche (perf/tokens, cf. sprint P5 de l'audit) et (b) la fiabilité des bounds sous Wayland (réserve §1.3, à composer avec `window_geometry_get`).
- **Piste alternative hors périmètre de ce spike** : `Atlas.App` référence déjà en Debug un shim `Atlas.DevEye` (`DevEyeServer`, `IDevEyeStateProvider`, `DevEyeCommandRegistry` — vu dans `~/Dropbox/Nextcloud/Kovelt/Projets/atlas/src/Atlas.DevEye`). Ce shim tourne **dans le process Uno** et pourrait, en théorie, exposer directement l'arbre `AutomationPeer` interne (celui du §2.2, déjà peuplé) via son propre canal HTTP/DevEye — sans passer par AT-SPI ni attendre qu'Uno implémente un bridge OS. C'est une piste structurellement différente (in-process .NET, pas D-Bus/AT-SPI) qui mériterait son **propre spike** si l'équivalent-DOM pour Uno reste un besoin réel ; elle n'a pas été creusée ici (hors question posée) et n'est pas comptée dans le verdict ci-dessus.

---

## 4. Condition GO/NO-GO précise pour rouvrir `accessibility_tree`

**NO-GO maintenant.** Rouvrir uniquement si **les deux** conditions suivantes sont vérifiées :

1. Une version d'Uno Platform (Uno.Sdk > 6.5.36) documente explicitement un support d'accessibilité pour le head desktop Linux (Skia/X11 ou tout futur head Wayland natif) — changelog Uno ou issue GitHub fermée sur le sujet.
2. **Re-run empirique de ce spike exact** après mise à jour : `busctl --user set-property org.a11y.Bus … IsEnabled b true` puis lancer `Atlas.App` et vérifier qu'il apparaît dans `busctl --address=$(bus a11y) --user list`, avec un `GetChildren` sur sa racine renvoyant un arbre non trivial (au moins les panneaux/boutons principaux de la fenêtre, avec noms non vides correspondant aux `AutomationProperties.Name` du XAML).

Tant que le §2.3 (test dynamique, zéro connexion au bus a11y) n'est pas contredit par un nouveau run, **la capture pixel reste la seule modalité d'inspection pour `kwin-eis`** — ce spike ne change pas cet état de fait, il en confirme la cause exacte.
