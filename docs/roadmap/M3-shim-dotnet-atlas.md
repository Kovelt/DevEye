# M3 — Shim .NET Atlas.DevEye : état interne d'atlas sans SDK Rust

## Objectif

Fournir à `deveye-server` le même wire protocol HTTP que `deveye-sdk` Rust, côté atlas : un projet C# .NET 10 (`Atlas.DevEye`) injecté uniquement en `#if DEBUG` qui expose ViewModels, logs Serilog et commandes MediatR sur `http://127.0.0.1:31337`. Aucun code DevEye ne s'embarque dans un build Release d'`Atlas.App` ou d'`Atlas.Api`.

- **Effort** : M
- **Dépend de** : M0 (wire protocol formalisé, page `docs/wire-protocol.md`), structure actuelle d'`Atlas.App` (Uno, `net10.0-browserwasm` + `net10.0-desktop`)
- **Statut décision** : tranché (décision 3 Morgan) — prioritaire, remonté avant M2

## Pourquoi (verrou levé)

`Atlas.App` est un client **Uno Platform** multi-cible (`net10.0-browserwasm` + `net10.0-desktop` Skia, `Atlas.App/Atlas.App.csproj` lignes 3-4) qui ne référence que `Atlas.Domain` et `Atlas.Shared` (règles ADR-002 / ADR-029, `Atlas.App/Atlas.App.csproj` lignes 34-37). Il n'existe **aucun runtime WebKitGTK, WebView2 ou CDP** disponible pour cette topologie : la tête desktop est rendue par **SkiaRenderer** (pas un webview), la tête WASM s'exécute dans le navigateur. L'adapter CDP de DevEye (`adapter-cdp-webview`) est donc sans prise sur atlas, et le `deveye-sdk` Rust ne peut pas non plus s'intégrer (atlas est en C#).

Sans shim, `deveye-server` ne peut ni interroger l'état de l'application (ViewModels), ni collecter ses logs structurés, ni déclencher une commande depuis un outil MCP. Claude travaille à l'aveugle sur la couche présentation d'atlas.

Verrou primaire : l'absence de pont entre le wire protocol DevEye (`GET /devtools/state`, `GET /devtools/logs/poll`, `POST /devtools/command`, `GET /devtools/commands`, `POST /devtools/eval`, définis dans `deveye-sdk/src/lib.rs` lignes 141-147) et l'écosystème .NET d'atlas.

## État actuel du code

### Côté DevEye

Le wire protocol est entièrement implémenté côté Rust :

- `deveye-sdk/src/lib.rs` l. 141-147 : routage axum des cinq endpoints.
- `deveye-sdk/src/lib.rs` l. 72-96 : structs `StatePayload`, `LogsPayload`, `CommandRequest` — forme JSON de référence.
- `deveye-sdk/src/lib.rs` l. 173-197 : middleware `guard_local_host` — rejette tout header `Host` ne désignant pas localhost (anti DNS-rebinding).
- `deveye-sdk/src/lib.rs` l. 47-48 : `DEFAULT_PORT = 31337`, surchargeable par `DEVEYE_PORT`.
- `adapter-dioxus-signals/src/bridge.rs` l. 39-73 : `SdkEvaluator` — attend `POST /devtools/eval` avec body `{ "expression": "..." }` et réponse `{ "value": ... }` ou `{ "error": "..." }` ; un 503 signifie pompe absente.
- `adapter-logs-tracing/src/lib.rs` l. 55-76 : `SdkTracingLogSource::poll` — `GET /devtools/logs/poll` ; attend un JSON `{ "events": [ ... ] }` ; le poll **draine** le buffer (les événements ne sont pas ré-émis).

### Côté atlas

- Aucun projet `Atlas.DevEye` n'existe.
- `Atlas.App/Atlas.App.csproj` : multi-cible `net10.0-browserwasm;net10.0-desktop`, références restreintes à `Atlas.Domain` + `Atlas.Shared` + paquets Microsoft.Extensions.
- `Directory.Packages.props` : `Serilog 4.2.0`, `Serilog.AspNetCore 9.0.0`, `MediatR 13.0.0`, `Microsoft.AspNetCore.Authentication.JwtBearer 10.0.0` — toutes les dépendances utiles sont déjà déclarées en CPM.
- `ARCHITECTURE.md` l. 38 : `Atlas.Mcp` prévu pour F-052, non créé — ADR-011 documente la délégation OAuth 2.1 (différée à F-052). `Atlas.DevEye` est distinct : c'est un outil de dev local, pas une surface agentique authentifiée.
- `ARCHITECTURE.md` l. 43-58 : règles de dépendance hexagonales vérifiées par `Atlas.Architecture.Tests` (NetArchTest). `Atlas.DevEye` devra être **exclu** de ces tests ou positionné hors de la pyramide hexagonale, puisqu'il est un outil transverse de debug.

## Changements détaillés

### `Atlas.DevEye/Atlas.DevEye.csproj` (à créer)

Projet de type **classlib** multi-cible, miroir de la portée de `Atlas.App` :

```xml
<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <!--
      Meme double cible qu'Atlas.App pour que la reference conditionnelle
      compile sur les deux tetes sans TFM mismatch.
    -->
    <TargetFrameworks>net10.0-browserwasm;net10.0-desktop</TargetFrameworks>
    <Nullable>enable</Nullable>
    <ImplicitUsings>enable</ImplicitUsings>
    <!--
      Securite dev-only : le shim ne s'assemble pas en Release.
      Un build Release qui inclurait ce projet echouerait ici.
    -->
    <DefineConstants Condition="'$(Configuration)' == 'Release'">DEVEYE_EXCLUDED</DefineConstants>
  </PropertyGroup>

  <!--
    Le shim n'a besoin d'aucune reference vers Atlas.Domain ou Atlas.Shared :
    il recoit ses donnees via les interfaces qu'on lui fournit (IDevEyeStateProvider,
    IDevEyeCommandRegistry). Respecte ADR-002 : aucune reference vers Infrastructure.
  -->
  <ItemGroup>
    <PackageReference Include="Microsoft.Extensions.Hosting.Abstractions" />
    <PackageReference Include="Microsoft.Extensions.DependencyInjection.Abstractions" />
    <PackageReference Include="Microsoft.Extensions.Logging.Abstractions" />
    <!--
      Serilog : pour le sink dedie qui capture les evenements vers le buffer interne.
      Version 4.2.0 declaree dans Directory.Packages.props.
    -->
    <PackageReference Include="Serilog" />
  </ItemGroup>

  <!--
    Sur la tete desktop : ASP.NET Core minimal API disponible dans le runtime .NET 10.
    Sur la tete WASM   : pas de serveur TCP possible depuis le navigateur — le projet
    compile mais DevEyeHost.StartAsync() est une no-op (voir ci-dessous).
  -->
  <ItemGroup Condition="'$(TargetFramework)' == 'net10.0-desktop'">
    <FrameworkReference Include="Microsoft.AspNetCore.App" />
  </ItemGroup>
</Project>
```

Les versions ne sont **pas** spécifiées dans les `<PackageReference>` : règle CPM atlas (`CLAUDE.md` l. 62).

### `Atlas.DevEye/IDevEyeStateProvider.cs` (à créer)

Interface que le ViewModel racine d'`Atlas.App` doit implémenter pour fournir un snapshot de son état. Positionnée dans `Atlas.DevEye` pour ne pas polluer `Atlas.Domain`.

```csharp
namespace Atlas.DevEye;

/// <summary>
/// Contrat que l'application hote implemente pour exposer son etat courant
/// a DevEye. Ne depend d'aucun concept metier Atlas : les valeurs sont
/// serialisees en JSON generique.
/// </summary>
public interface IDevEyeStateProvider
{
    /// <summary>
    /// Retourne un dictionnaire cle/valeur serialisable representant l'etat
    /// observable courant de l'application (page active, entite selectionnee,
    /// indicateurs de chargement...). Appelee a chaque GET /devtools/state.
    /// </summary>
    IReadOnlyDictionary<string, object?> GetCurrentState();
}
```

### `Atlas.DevEye/IDevEyeCommandRegistry.cs` (à créer)

Registre des commandes logiques exposables via `POST /devtools/command`, miroir de `deveye_sdk::expose_route` (Rust, `deveye-sdk/src/lib.rs` l. 217-230).

```csharp
namespace Atlas.DevEye;

/// <summary>
/// Registre des commandes logiques invocables par DevEye.
/// L'application hote enregistre ses actions via Register() au demarrage.
/// </summary>
public interface IDevEyeCommandRegistry
{
    void Register(
        string name,
        string description,
        Func<System.Text.Json.JsonElement, System.Text.Json.JsonElement> handler);

    IReadOnlyList<CommandDescriptor> List();

    bool TryInvoke(
        string name,
        System.Text.Json.JsonElement payload,
        out System.Text.Json.JsonElement result);
}

public sealed record CommandDescriptor(string Name, string Description);
```

### `Atlas.DevEye/DevEyeCommandRegistry.cs` (à créer)

Implémentation thread-safe (lecture majoritaire, écriture rare au démarrage) :

```csharp
namespace Atlas.DevEye;

internal sealed class DevEyeCommandRegistry : IDevEyeCommandRegistry
{
    private readonly Dictionary<string, (string Description,
        Func<System.Text.Json.JsonElement, System.Text.Json.JsonElement> Handler)> _routes = new();
    private readonly ReaderWriterLockSlim _lock = new();

    public void Register(string name, string description,
        Func<System.Text.Json.JsonElement, System.Text.Json.JsonElement> handler)
    {
        _lock.EnterWriteLock();
        try { _routes[name] = (description, handler); }
        finally { _lock.ExitWriteLock(); }
    }

    public IReadOnlyList<CommandDescriptor> List()
    {
        _lock.EnterReadLock();
        try
        {
            return _routes
                .OrderBy(kv => kv.Key)
                .Select(kv => new CommandDescriptor(kv.Key, kv.Value.Description))
                .ToList();
        }
        finally { _lock.ExitReadLock(); }
    }

    public bool TryInvoke(string name, System.Text.Json.JsonElement payload,
        out System.Text.Json.JsonElement result)
    {
        _lock.EnterReadLock();
        Func<System.Text.Json.JsonElement, System.Text.Json.JsonElement>? handler;
        try { _routes.TryGetValue(name, out var entry); handler = entry.Handler; }
        finally { _lock.ExitReadLock(); }

        if (handler is null) { result = default; return false; }
        result = handler(payload);
        return true;
    }
}
```

### `Atlas.DevEye/DevEyeLogSink.cs` (à créer)

Sink Serilog qui pousse les événements dans un buffer borné (politique drop-oldest, capacité 1024 — miroir de `LOG_BUFFER_CAPACITY` dans `deveye-sdk/src/lib.rs` l. 52). Le sink est **dédié à DevEye** : il n'interfère pas avec les autres sinks (console, OpenTelemetry) déjà configurés dans `Atlas.Api`.

```csharp
using Serilog.Core;
using Serilog.Events;

namespace Atlas.DevEye;

/// <summary>
/// Sink Serilog dedie a DevEye. Pousse les evenements dans un buffer
/// circulaire draine par GET /devtools/logs/poll. Thread-safe.
/// </summary>
public sealed class DevEyeLogSink : ILogEventSink
{
    private const int Capacity = 1024;
    private readonly Queue<LogEventDto> _buffer = new(Capacity);
    private readonly object _lock = new();

    public void Emit(LogEvent logEvent)
    {
        var dto = new LogEventDto(
            Timestamp: logEvent.Timestamp.UtcDateTime,
            Level: logEvent.Level.ToString(),
            Message: logEvent.RenderMessage(),
            Fields: logEvent.Properties.ToDictionary(
                kv => kv.Key,
                kv => (object?)kv.Value.ToString()));

        lock (_lock)
        {
            if (_buffer.Count >= Capacity)
                _buffer.Dequeue(); // drop-oldest
            _buffer.Enqueue(dto);
        }
    }

    /// <summary>
    /// Draine le buffer : retourne tous les evenements accumules depuis le
    /// dernier poll et les supprime du buffer. Semantique identique au
    /// logs_poll_handler Rust (deveye-sdk/src/lib.rs l. 241-248).
    /// </summary>
    public IReadOnlyList<LogEventDto> DrainAll()
    {
        lock (_lock)
        {
            var events = _buffer.ToList();
            _buffer.Clear();
            return events;
        }
    }
}

public sealed record LogEventDto(
    DateTime Timestamp,
    string Level,
    string Message,
    IReadOnlyDictionary<string, object?> Fields);
```

### `Atlas.DevEye/DevEyeHost.cs` (à créer)

Service hébergé (`IHostedService`) qui démarre le mini-serveur HTTP sur `127.0.0.1:DEVEYE_PORT`. Les cinq endpoints respectent exactement le contrat défini dans `deveye-sdk/src/lib.rs` l. 141-147 et attendu par `adapter-dioxus-signals/src/bridge.rs` l. 49-74 et `adapter-logs-tracing/src/lib.rs` l. 55-76.

Sur la tête `net10.0-browserwasm`, `StartAsync` est une no-op : le contexte navigateur n'autorise pas d'écoute TCP. La compilation reste possible grâce aux directives de préprocesseur.

```csharp
using Microsoft.AspNetCore.Builder;
using Microsoft.AspNetCore.Http;
using Microsoft.Extensions.Hosting;
using System.Text.Json;

namespace Atlas.DevEye;

public sealed class DevEyeHost : IHostedService
{
#if NET10_0_DESKTOP || (!BROWSERWASM)
    private WebApplication? _app;
#endif

    private readonly IDevEyeStateProvider _stateProvider;
    private readonly DevEyeLogSink _logSink;
    private readonly IDevEyeCommandRegistry _commands;
    private readonly int _port;

    public DevEyeHost(
        IDevEyeStateProvider stateProvider,
        DevEyeLogSink logSink,
        IDevEyeCommandRegistry commands)
    {
        _stateProvider = stateProvider;
        _logSink = logSink;
        _commands = commands;
        _port = int.TryParse(
            Environment.GetEnvironmentVariable("DEVEYE_PORT"), out var p) ? p : 31337;
    }

    public async Task StartAsync(CancellationToken cancellationToken)
    {
#if BROWSERWASM
        // Pas de serveur TCP en contexte navigateur.
        // La tete WASM sert /devtools/eval via EvaluateJavaScriptAsync (voir ci-dessous).
        await Task.CompletedTask;
#else
        var builder = WebApplication.CreateSlimBuilder();
        builder.WebHost.UseUrls($"http://127.0.0.1:{_port}");
        var app = builder.Build();

        // Garde anti DNS-rebinding : meme logique que guard_local_host Rust
        // (deveye-sdk/src/lib.rs l. 173-197).
        app.Use(async (ctx, next) =>
        {
            var host = ctx.Request.Host.Host;
            if (!HostIsLocal(host))
            {
                ctx.Response.StatusCode = 403;
                return;
            }
            await next(ctx);
        });

        // GET /devtools/state
        // Retourne { "entries": { "cle": valeur, ... } }
        // Forme attendue par adapter-dioxus-signals (StatePayload, deveye-sdk/src/lib.rs l. 72-75).
        app.MapGet("/devtools/state", (HttpContext ctx) =>
        {
            var entries = _stateProvider.GetCurrentState();
            return Results.Json(new { entries });
        });

        // GET /devtools/logs/poll
        // Draine le buffer Serilog et retourne { "events": [ ... ] }.
        // Forme attendue par SdkTracingLogSource (adapter-logs-tracing/src/lib.rs l. 49-52).
        app.MapGet("/devtools/logs/poll", (HttpContext ctx) =>
        {
            var events = _logSink.DrainAll();
            return Results.Json(new { events });
        });

        // POST /devtools/command
        // Body : { "route": "nom", "payload": { ... } }
        // Forme attendue par CommandRequest (deveye-sdk/src/lib.rs l. 86-91).
        app.MapPost("/devtools/command", async (HttpContext ctx) =>
        {
            using var doc = await JsonDocument.ParseAsync(ctx.Request.Body, cancellationToken: cancellationToken);
            var root = doc.RootElement;
            var route = root.GetProperty("route").GetString() ?? string.Empty;
            var payload = root.TryGetProperty("payload", out var p) ? p : default;

            if (!_commands.TryInvoke(route, payload, out var result))
            {
                ctx.Response.StatusCode = 404;
                await ctx.Response.WriteAsJsonAsync(
                    new { error = $"commande inconnue : {route}" }, cancellationToken);
                return;
            }
            await ctx.Response.WriteAsJsonAsync(result, cancellationToken);
        });

        // GET /devtools/commands
        // Retourne la liste triee des commandes enregistrees.
        // Forme attendue par commands_handler Rust (deveye-sdk/src/lib.rs l. 276-291).
        app.MapGet("/devtools/commands", (HttpContext ctx) =>
        {
            var list = _commands.List()
                .Select(d => new { name = d.Name, description = d.Description });
            return Results.Json(list);
        });

        // POST /devtools/eval
        // Body : { "expression": "..." }
        // Reponse : { "value": ... } ou { "error": "..." } ou 503 si pompe absente.
        // Forme attendue par SdkEvaluator (adapter-dioxus-signals/src/bridge.rs l. 39-73).
        //
        // Sur la tete desktop Skia, aucune pompe JS n'est disponible : on retourne 503.
        // Sur la tete WASM, l'eval est geree cote navigateur via EvaluateJavaScriptAsync
        // (voir DevEyeWasmEvalBridge.cs) et transmise via un canal en memoire partagee.
        app.MapPost("/devtools/eval", async (HttpContext ctx) =>
        {
            ctx.Response.StatusCode = 503;
            await ctx.Response.WriteAsJsonAsync(
                new { error = "eval JS indisponible sur la tete desktop Skia (aucune pompe WebView)" },
                cancellationToken);
        });

        _app = app;
        await _app.StartAsync(cancellationToken);
#endif
    }

    public async Task StopAsync(CancellationToken cancellationToken)
    {
#if !BROWSERWASM
        if (_app is not null)
            await _app.StopAsync(cancellationToken);
#else
        await Task.CompletedTask;
#endif
    }

    private static bool HostIsLocal(string host)
    {
        // Meme logique que host_is_local Rust (deveye-sdk/src/lib.rs l. 188-197).
        host = host.Trim();
        if (host.StartsWith('['))
            return host.TrimStart('[').Split(']')[0] == "::1";
        var hostname = host.Contains(':') ? host[..host.LastIndexOf(':')] : host;
        return string.Equals(hostname, "localhost", StringComparison.OrdinalIgnoreCase)
            || hostname == "127.0.0.1";
    }
}
```

### `Atlas.DevEye/DevEyeWasmEvalBridge.cs` (à créer, tête WASM uniquement)

Sur `net10.0-browserwasm`, le serveur HTTP n'est pas disponible. Le canal entre `deveye-server` et la tête WASM ne peut donc pas passer par TCP direct. Deux options sont envisageables à M3 :

**Option A (recommandée pour M3)** : `deveye-server` se connecte à `Atlas.Api` (qui, lui, tourne en desktop), qui propage la requête eval vers le client WASM via SignalR ou un canal WebSocket. Nécessite `Atlas.Api` vivant (cas normal en dev).

**Option B (autonome)** : un Service Worker intercepte les requêtes sur `http://127.0.0.1:31337` et les route vers `EvaluateJavaScriptAsync`. Complexité de mise en œuvre élevée sur Uno WASM.

À M3, l'endpoint `POST /devtools/eval` retourne **503** sur les deux têtes et la pompe est documentée comme "à brancher en M3b ou M4". La priorité de M3 est `state`, `logs/poll` et `command`.

```csharp
// Fichier placeholder — implementation differee.
// Rappel : sur net10.0-browserwasm, DevEyeHost.StartAsync est une no-op.
// L'eval WASM sera implementee dans un second temps via le canal Atlas.Api <-> client WASM.
namespace Atlas.DevEye;

#if BROWSERWASM
internal static class DevEyeWasmEvalBridge
{
    // A implementer : recevoir une expression JS depuis Atlas.Api
    // et l'executer via Microsoft.JSInterop.IJSRuntime.InvokeAsync.
}
#endif
```

### Intégration dans `Atlas.App` (côté UI Uno)

La référence au projet `Atlas.DevEye` est conditionnelle à la configuration Debug. Elle est ajoutée dans `Atlas.App/Atlas.App.csproj` **après** les références existantes :

```xml
<!-- Atlas.App/Atlas.App.csproj — ajout apres les ItemGroup existants -->

<!--
  Atlas.DevEye : shim DevEye injecte uniquement en Debug.
  Ne reference pas Atlas.Infrastructure.* : respecte ADR-002.
  Exclu des verifications NetArchTest via l'attribut de namespace
  (voir Atlas.Architecture.Tests — a mettre a jour).
-->
<ItemGroup Condition="'$(Configuration)' == 'Debug'">
  <ProjectReference Include="..\Atlas.DevEye\Atlas.DevEye.csproj" />
</ItemGroup>
```

Dans le point d'entrée de l'application (fichier `App.xaml.cs` ou équivalent Uno), l'initialisation du shim se fait sous garde `#if DEBUG` :

```csharp
// Atlas.App — App.xaml.cs (extrait, a adapter au point d'entree reel)
#if DEBUG
services.AddSingleton<IDevEyeStateProvider, AtlasDevEyeStateProvider>();
services.AddSingleton<DevEyeLogSink>();
services.AddSingleton<IDevEyeCommandRegistry, DevEyeCommandRegistry>();
services.AddHostedService<DevEyeHost>();
#endif
```

`AtlasDevEyeStateProvider` est une classe interne à `Atlas.App` qui implémenterait `IDevEyeStateProvider`. Elle exposerait par exemple la page courante, l'identifiant de l'entité sélectionnée et l'état de chargement. Son contenu est spécifique à l'app mais sa forme est générique (dictionnaire JSON).

### Configuration du sink Serilog dans `Atlas.Api`

Les logs d'`Atlas.Api` sont configurés via `Serilog.AspNetCore 9.0.0` (`Directory.Packages.props` l. 77). On ajoute le sink DevEye **uniquement en Debug** dans `Program.cs` :

```csharp
// Atlas.Api/Program.cs — extrait
#if DEBUG
var devEyeLogSink = new Atlas.DevEye.DevEyeLogSink();
Log.Logger = new LoggerConfiguration()
    .WriteTo.Sink(devEyeLogSink)   // drain vers DevEye
    // ... autres sinks existants (Console, OpenTelemetry) ...
    .CreateLogger();

builder.Services.AddSingleton(devEyeLogSink);
#endif
```

Note : `Atlas.Api` aura aussi besoin d'une référence conditionnelle vers `Atlas.DevEye` pour ce cas d'usage logs. Cela reste conforme à ADR-002 (l'Api peut référencer tout le monde) et ne crée pas de violation hexagonale.

### Mise à jour de `tests/Atlas.Architecture.Tests`

`Atlas.Architecture.Tests` utilise NetArchTest pour vérifier les dépendances hexagonales (`ARCHITECTURE.md` l. 43-44). `Atlas.DevEye` est un outil transverse de debug, pas une couche hexagonale. Il convient d'**exclure** ses namespaces des règles existantes pour éviter de faux positifs :

```csharp
// Dans le fichier de tests d'architecture existant (pattern a adapter)
// Exclure Atlas.DevEye des regles de dependances hexagonales.
Types.InAssembly(assembly)
    .That()
    .ResideInNamespaceStartingWith("Atlas.")
    .And()
    .DoNotResideInNamespace("Atlas.DevEye")  // shim debug transverse
    // ... reste de la regle ...
```

## Nouveaux artefacts

| Artefact | Type | Forme |
|---|---|---|
| `src/Atlas.DevEye/Atlas.DevEye.csproj` | Projet C# classlib | cibles `net10.0-browserwasm;net10.0-desktop`, référence conditionnelle `Microsoft.AspNetCore.App` sur desktop uniquement |
| `Atlas.DevEye.IDevEyeStateProvider` | Interface C# | `IReadOnlyDictionary<string, object?> GetCurrentState()` |
| `Atlas.DevEye.IDevEyeCommandRegistry` | Interface C# | `Register / List / TryInvoke` |
| `Atlas.DevEye.DevEyeCommandRegistry` | Implémentation | `ReaderWriterLockSlim`, thread-safe |
| `Atlas.DevEye.DevEyeLogSink` | Sink Serilog | `ILogEventSink`, buffer circulaire 1024, `DrainAll()` |
| `Atlas.DevEye.DevEyeHost` | `IHostedService` | ASP.NET Core Minimal API, bind `127.0.0.1:DEVEYE_PORT`, garde `HostIsLocal` |
| `Atlas.DevEye.DevEyeWasmEvalBridge` | Placeholder | no-op en M3, canal API<->WASM à brancher ultérieurement |
| Outil MCP aucun nouveau | — | Les outils MCP existants (`get_state`, `poll_logs`, `invoke_command`, `list_commands`) fonctionnent sans changement côté DevEye |

## Critère de done

```bash
# 1. Build Debug : les deux tetes compilent avec Atlas.DevEye inclus.
dotnet build src/Atlas.App -f net10.0-desktop   -c Debug
dotnet build src/Atlas.App -f net10.0-browserwasm -c Debug

# 2. Build Release : Atlas.DevEye n'est pas inclus.
dotnet build src/Atlas.App -f net10.0-desktop   -c Release
# -> aucune reference a Atlas.DevEye dans les binaires produits

# 3. Tests d'architecture : toujours verts apres l'exclusion du namespace.
dotnet test tests/Atlas.Architecture.Tests

# 4. Smoke test manuel : lancer Atlas.App (desktop Debug) + deveye-server.
dotnet run --project src/Atlas.App -f net10.0-desktop -c Debug

# Dans un second terminal :
cargo run -p deveye-server -- --serve-mcp
# ou appel HTTP direct :
curl -s -H "Host: localhost" http://127.0.0.1:31337/devtools/state
# -> { "entries": { "page": "...", ... } }

curl -s -H "Host: localhost" http://127.0.0.1:31337/devtools/logs/poll
# -> { "events": [ ... ] }  (vide si aucun log depuis le dernier poll)

curl -s -H "Host: localhost" http://127.0.0.1:31337/devtools/commands
# -> [ { "name": "...", "description": "..." }, ... ]

# 5. Garde anti DNS-rebinding : un Host etranger doit etre rejete.
curl -s -H "Host: evil.com" http://127.0.0.1:31337/devtools/state
# -> 403 Forbidden

# 6. Endpoint eval sur desktop : 503 attendu.
curl -s -X POST -H "Host: localhost" -H "Content-Type: application/json" \
     -d '{"expression":"1+1"}' http://127.0.0.1:31337/devtools/eval
# -> 503 avec { "error": "eval JS indisponible sur la tete desktop Skia..." }
```

Comportement observable via MCP : après `cargo run -p deveye-server -- --serve-mcp`, les outils `get_app_state` et `poll_logs` retournent des données réelles en provenance d'atlas (page courante, événements Serilog).

## Risques et pièges

**Compilation conditionnelle multi-cible Uno.** La directive `#if BROWSERWASM` n'est pas un symbole standard .NET. Uno SDK expose `__WASM__` ou `HAS_UNO_WASM` selon la version. Vérifier le symbole exact en consultant `Atlas.App.csproj` une fois le SDK Uno initialisé. Variante sûre : utiliser `#if !WINDOWS && !MACOS && !LINUX` (symboles Uno pour la tête desktop) ou conditionner sur `TargetFramework` via MSBuild plutôt que dans le code.

**ADR-002 et tests NetArchTest.** `Atlas.DevEye` n'est pas une couche hexagonale mais un outil de debug. Si `Atlas.Architecture.Tests` scanne tous les assemblages de la solution, ses règles de dépendance pourraient rejeter `Atlas.DevEye` (qui référence `Microsoft.AspNetCore.App` — une dépendance d'infrastructure). L'exclusion explicite du namespace `Atlas.DevEye` dans les tests d'architecture est non-optionnelle.

**Port déjà pris.** Si `Atlas.Api` et `Atlas.App` tournent tous les deux en Debug sur le même poste et tentent tous les deux de binder `127.0.0.1:31337`, le second échouera silencieusement (comportement voulu, comme dans `deveye-sdk/src/lib.rs` l. 153-165). Ne déployer qu'un seul composant atlas comme source DevEye à la fois, ou utiliser des ports distincts (`DEVEYE_PORT=31338` pour le second).

**Sécurité dev-only.** Le serveur HTTP en local est intentionnellement non authentifié. La garde `HostIsLocal` (miroir de `host_is_local` Rust, `deveye-sdk/src/lib.rs` l. 188-197) est la seule protection. Elle suffit dans un contexte de dev local mais ne doit **jamais** se retrouver dans un build Release. La condition `Condition="'$(Configuration)' == 'Debug'"` sur la `<ProjectReference>` dans `Atlas.App.csproj` est le premier rempart ; l'absence de `FrameworkReference` ASP.NET Core sur la tête WASM en est un second.

**MediatR dans Atlas.DevEye.** Si un scénario futur exige de dispatcher une `IMediator` depuis le shim, cela impliquerait de référencer `MediatR` et de recevoir `IMediator` par injection. Attention : MediatR 13 (`Directory.Packages.props` l. 9) est déjà déclaré en CPM. La référence serait licite dans `Atlas.DevEye`. En revanche, dispatcher via `IMediator` signifie que le shim obtient un accès indirect à toute la couche `Application` — ce qui est acceptable pour un outil de debug local mais doit être documenté comme un écart de la règle ADR-002 (qui ne vise pas `Atlas.DevEye`).

**Tête WASM et eval JS.** L'option A (canal via `Atlas.Api`) décrite dans `DevEyeWasmEvalBridge.cs` nécessite que `Atlas.Api` soit en cours d'exécution (cas normal en dev, mais pas garanti en session de debug isolée). Documenter cette prérequis dans `docs/usage.md` DevEye.

**Versions NuGet.** Ne jamais spécifier de version dans les `<PackageReference>` d'`Atlas.DevEye` : règle CPM stricte d'atlas (`CLAUDE.md` l. 62). Si une dépendance n'est pas encore dans `Directory.Packages.props`, l'y ajouter avant de l'utiliser dans le `.csproj`.

## Checklist d'exécution

- [ ] Lire `docs/wire-protocol.md` (DevEye, à rédiger en M0) pour le contrat JSON exact avant d'implémenter les handlers — les snippets ci-dessus sont une extrapolation du code Rust existant.
- [ ] Vérifier le symbole de préprocesseur exact pour détecter la tête WASM Uno (chercher `__WASM__`, `HAS_UNO_WASM`, `BROWSERWASM` dans les projets existants d'`Atlas.App`).
- [ ] Créer `src/Atlas.DevEye/` et y déposer `Atlas.DevEye.csproj` (sans version dans les `<PackageReference>`).
- [ ] Implémenter `IDevEyeStateProvider`, `IDevEyeCommandRegistry`, `DevEyeCommandRegistry`, `DevEyeLogSink` (pas de dépendance Uno, pure .NET).
- [ ] Implémenter `DevEyeHost` avec le serveur ASP.NET Core Minimal API, les cinq endpoints et la garde `HostIsLocal`.
- [ ] Ajouter la `<ProjectReference>` conditionnelle (`Configuration == Debug`) dans `Atlas.App/Atlas.App.csproj`.
- [ ] Créer `AtlasDevEyeStateProvider` dans `Atlas.App` (implémentation de `IDevEyeStateProvider` exposant l'état courant de l'UI).
- [ ] Enregistrer les services DevEye dans le conteneur DI d'`Atlas.App` sous `#if DEBUG`.
- [ ] Ajouter la référence conditionnelle `Atlas.DevEye` dans `Atlas.Api/Atlas.Api.csproj` (pour le sink Serilog).
- [ ] Brancher `DevEyeLogSink` dans la configuration Serilog d'`Atlas.Api` sous `#if DEBUG`.
- [ ] Mettre à jour `tests/Atlas.Architecture.Tests` : exclure le namespace `Atlas.DevEye` des règles hexagonales.
- [ ] Vérifier que `dotnet build -c Release` ne produit aucune référence à `Atlas.DevEye` dans les artefacts.
- [ ] Smoke test complet : `GET /devtools/state`, `GET /devtools/logs/poll`, `GET /devtools/commands`, `POST /devtools/command`, `POST /devtools/eval` (503 attendu sur desktop).
- [ ] Tester la garde : requête avec `Host: evil.com` -> 403.
- [ ] Mettre à jour `docs/usage.md` DevEye : section atlas, prérequis (`Atlas.App` en Debug, `Atlas.Api` en Debug pour les logs), variable `DEVEYE_PORT`.
- [ ] Ajouter `Atlas.DevEye` dans le `Directory.Build.props` ou la solution `.sln` si nécessaire pour que le build solution le prenne en compte.
- [ ] Valider que `dotnet test tests/Atlas.Architecture.Tests` reste vert.
