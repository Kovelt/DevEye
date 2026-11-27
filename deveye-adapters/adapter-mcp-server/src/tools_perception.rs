//! Outils de la famille **perception** : `snapshot` et `act`, **expérimentaux**
//! (hors promesse semver, ADR-0069, ADR-0073).
//!
//! `snapshot` rend un arbre compact d'éléments, chacun avec une référence que
//! l'agent réutilise ; `act` agit sur une référence. Les deux passent par le port
//! `ElementInspector` : la position est **remesurée à chaque action** (le rect d'un
//! instantané est indicatif) et rien n'est relocalisé en silence — une référence
//! périmée est une erreur qui propose des candidats, pas un clic sur un voisin.
//!
//! `act` ne crée aucune commande nouvelle : il se ramène à `InteractionCommand`
//! existantes dispatchées sur le centre remesuré.

use deveye_core::elements::{ElementNode, ElementSnapshot, SnapshotQuery, DEFAULT_MAX_NODES};
use deveye_ports::ElementError;

use super::*;

/// Plafond de `max_nodes` : au-delà, l'instantané n'est plus « compact ».
pub(crate) const SNAPSHOT_MAX_NODES_CAP: usize = 2_000;

/// Longueur maximale (caractères) d'un nom d'élément dans le texte compact.
const NAME_CHARS: usize = 80;

/// Longueur maximale (caractères) d'une valeur dans le texte compact.
const VALUE_CHARS: usize = 80;

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub(crate) struct SnapshotParams {
    /// Référence (`e12`, `w:vcf/cutoff`) du sous-arbre à décrire ; toute la cible
    /// si absent. Les widgets natifs n'acceptent qu'une référence `w:<id>`.
    #[serde(default)]
    pub(crate) scope: Option<String>,
    /// Profondeur maximale (1 = éléments de premier niveau seulement).
    #[serde(default)]
    pub(crate) max_depth: Option<u32>,
    /// Nombre maximal d'éléments (défaut 400, plafond 2000) ; `truncated` signale
    /// la coupe.
    #[serde(default)]
    pub(crate) max_nodes: Option<usize>,
    /// Ne garder que les éléments interactifs et leurs conteneurs utiles (défaut
    /// `true`) ; `false` décrit aussi les textes et conteneurs anonymes.
    #[serde(default)]
    pub(crate) interactive_only: Option<bool>,
    /// Ne garder que les éléments dans la zone visible (défaut `false`).
    #[serde(default)]
    pub(crate) viewport_only: Option<bool>,
    /// `"text"` (défaut : arbre indenté compact) ou `"json"` (même contenu, avec les
    /// rects, aussi en `structuredContent`).
    #[serde(default)]
    pub(crate) format: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SnapshotFormat {
    Text,
    Json,
}

impl SnapshotFormat {
    fn parse(raw: Option<&str>) -> Result<Self, McpError> {
        match raw.map(str::to_ascii_lowercase).as_deref() {
            None | Some("") | Some("text") => Ok(Self::Text),
            Some("json") => Ok(Self::Json),
            Some(other) => Err(McpError::invalid_params(
                format!("format invalide : {other:?} (attendu text ou json)"),
                None,
            )),
        }
    }
}

impl SnapshotParams {
    /// Requête du port, après validation (aucun accès au port avant).
    fn query(&self) -> SnapshotQuery {
        let mut query = SnapshotQuery::new()
            .with_max_nodes(
                self.max_nodes
                    .unwrap_or(DEFAULT_MAX_NODES)
                    .clamp(1, SNAPSHOT_MAX_NODES_CAP),
            )
            .with_interactive_only(self.interactive_only.unwrap_or(true))
            .with_viewport_only(self.viewport_only.unwrap_or(false));
        if let Some(scope) = self.scope.as_deref().filter(|s| !s.trim().is_empty()) {
            query = query.with_scope(scope.trim());
        }
        if let Some(depth) = self.max_depth {
            query = query.with_max_depth(depth.max(1));
        }
        query
    }
}

/// Actions de `act`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ActAction {
    Click,
    DoubleClick,
    RightClick,
    Hover,
    Type,
    ScrollIntoView,
    Scroll,
}

impl ActAction {
    const NAMES: &'static str =
        "click, double_click, right_click, hover, type, scroll_into_view, scroll";

    fn parse(raw: &str) -> Result<Self, McpError> {
        Ok(match raw.trim().to_ascii_lowercase().as_str() {
            "click" => Self::Click,
            "double_click" => Self::DoubleClick,
            "right_click" => Self::RightClick,
            "hover" => Self::Hover,
            "type" => Self::Type,
            "scroll_into_view" => Self::ScrollIntoView,
            "scroll" => Self::Scroll,
            other => {
                return Err(McpError::invalid_params(
                    format!("action inconnue : {other:?} (attendu : {})", Self::NAMES),
                    None,
                ))
            }
        })
    }

    fn name(self) -> &'static str {
        match self {
            Self::Click => "click",
            Self::DoubleClick => "double_click",
            Self::RightClick => "right_click",
            Self::Hover => "hover",
            Self::Type => "type",
            Self::ScrollIntoView => "scroll_into_view",
            Self::Scroll => "scroll",
        }
    }

    /// Les gestes qui n'ont de sens que sur un élément qui accepte les interactions.
    fn needs_enabled(self) -> bool {
        matches!(
            self,
            Self::Click | Self::DoubleClick | Self::RightClick | Self::Type
        )
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct ActParams {
    /// Référence de l'élément, telle que rendue par `snapshot` (`e12`, `w:save`).
    #[serde(rename = "ref")]
    pub(crate) reference: String,
    /// `click`, `double_click`, `right_click`, `hover`, `type` (clique pour donner le
    /// focus puis saisit `text`), `scroll_into_view` (vérifie que l'élément est
    /// visible), `scroll` (molette de `dx`/`dy` pixels ancrée sur l'élément).
    pub(crate) action: String,
    /// Texte à saisir (`type` seulement).
    #[serde(default)]
    pub(crate) text: Option<String>,
    /// Défilement horizontal en pixels (`scroll` seulement, positif = droite).
    #[serde(default)]
    pub(crate) dx: Option<i32>,
    /// Défilement vertical en pixels (`scroll` seulement, positif = bas).
    #[serde(default)]
    pub(crate) dy: Option<i32>,
    /// Bouton (`click` seulement) : `left` (défaut), `right`, `middle`.
    #[serde(default)]
    pub(crate) button: Option<String>,
}

/// Action validée, prête à être exécutée sous le gate.
struct ValidAct {
    action: ActAction,
    reference: String,
    text: Option<String>,
    dx: i32,
    dy: i32,
    button: MouseButton,
}

impl ActParams {
    /// Valide tout avant le moindre accès au port : un paramètre qui n'a pas de
    /// sens pour l'action est refusé plutôt qu'ignoré en silence.
    fn validate(self) -> Result<ValidAct, McpError> {
        let reference = self.reference.trim().to_string();
        if reference.is_empty() {
            return Err(McpError::invalid_params(
                "`ref` vide : passer une référence rendue par `snapshot`".to_string(),
                None,
            ));
        }
        let action = ActAction::parse(&self.action)?;
        let refuse = |name: &str| {
            McpError::invalid_params(
                format!("`{name}` n'a pas de sens pour l'action `{}`", action.name()),
                None,
            )
        };
        if self.text.is_some() && action != ActAction::Type {
            return Err(refuse("text"));
        }
        if (self.dx.is_some() || self.dy.is_some()) && action != ActAction::Scroll {
            return Err(refuse(if self.dx.is_some() { "dx" } else { "dy" }));
        }
        if self.button.is_some() && action != ActAction::Click {
            return Err(refuse("button"));
        }
        if action == ActAction::Type && self.text.is_none() {
            return Err(McpError::invalid_params(
                "l'action `type` exige `text`".to_string(),
                None,
            ));
        }
        let (dx, dy) = (self.dx.unwrap_or(0), self.dy.unwrap_or(0));
        if action == ActAction::Scroll && dx == 0 && dy == 0 {
            return Err(McpError::invalid_params(
                "l'action `scroll` exige `dx` ou `dy` non nul".to_string(),
                None,
            ));
        }
        Ok(ValidAct {
            action,
            reference,
            text: self.text,
            dx,
            dy,
            button: parse_mouse_button(self.button.as_deref())?,
        })
    }
}

/// Acquittement de `act` : ce qui a été fait, où (centre remesuré, px).
#[derive(Clone, Debug, Serialize)]
struct ActAck {
    action: &'static str,
    ok: bool,
    #[serde(rename = "ref")]
    reference: String,
    x: i32,
    y: i32,
}

fn flat(text: &str, max_chars: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max_chars {
        flat
    } else {
        let mut cut: String = flat.chars().take(max_chars).collect();
        cut.push('…');
        cut
    }
}

fn push_node(node: &ElementNode, depth: usize, out: &mut String) {
    out.push_str(&"  ".repeat(depth));
    out.push_str("- ");
    out.push_str(&node.role);
    if let Some(name) = node.name.as_deref().filter(|n| !n.trim().is_empty()) {
        out.push_str(&format!(" \"{}\"", flat(name, NAME_CHARS)));
    }
    out.push_str(&format!(" [ref={}]", node.reference));
    let state = &node.state;
    let flags = [
        (state.disabled, "disabled"),
        (state.checked == Some(true), "checked"),
        (state.checked == Some(false), "unchecked"),
        (state.expanded == Some(true), "expanded"),
        (state.expanded == Some(false), "collapsed"),
        (state.selected, "selected"),
        (state.focused, "focused"),
        (node.offscreen, "offscreen"),
        (node.scrollable, "scrollable"),
        (node.ephemeral, "ephemeral"),
    ];
    for (_, flag) in flags.iter().filter(|(on, _)| *on) {
        out.push_str(&format!(" [{flag}]"));
    }
    if let Some(value) = node.value.as_deref() {
        out.push_str(&format!(" = {}", to_json_text(&flat(value, VALUE_CHARS))));
    }
    out.push('\n');
    for child in &node.children {
        push_node(child, depth + 1, out);
    }
}

/// Texte compact d'un instantané : un élément par ligne, indenté, orienté jetons.
/// Les rects n'y figurent pas (indicatifs, coûteux) : `format: "json"` les donne.
pub(crate) fn render_snapshot_text(snapshot: &ElementSnapshot) -> String {
    let mut out = format!(
        "# {} : {} élément(s), génération {}",
        snapshot.target_kind,
        snapshot.count(),
        snapshot.generation
    );
    if snapshot.truncated {
        out.push_str(" — TRONQUÉ : cibler un `scope` ou relever `max_nodes`");
    }
    out.push('\n');
    for node in &snapshot.nodes {
        push_node(node, 0, &mut out);
    }
    if let Some(focus) = &snapshot.focus {
        out.push_str(&format!("focus: {focus}\n"));
    }
    out
}

/// Traduit l'échec d'un port d'éléments en résultat `isError` structuré, ou en
/// erreur de protocole pour une vraie panne (même répartition que les autres outils).
pub(crate) fn element_failure(
    error: ElementError,
    reference: Option<&str>,
) -> Result<CallToolResult, McpError> {
    let tagged = |error: ToolError| match reference {
        Some(r) => error.with_reference(r),
        None => error,
    };
    let result = match error {
        ElementError::Stale {
            reference: stale,
            candidates,
        } => {
            let mut error = ToolError::new(
                ErrorReason::StaleRef,
                format!("la référence {stale} ne désigne plus d'élément vivant"),
            )
            .with_reference(stale)
            .with_hint(
                "relancer `snapshot` : l'interface a changé depuis l'instantané ; \
                 `candidates` liste les éléments de même rôle et de même nom (aucun n'a été \
                 choisi pour vous)",
            );
            if !candidates.is_empty() {
                error = error.with_candidates(candidates);
            }
            error.into_result()
        }
        ElementError::StaleTree { age_ms } => tagged(ToolError::new(
            ErrorReason::StaleTree,
            format!("l'app n'a pas republié son arbre depuis {age_ms} ms"),
        ))
        .with_hint(
            "l'app est peut-être gelée ou n'a pas redessiné : provoquer un rendu (survol, \
             focus de la fenêtre) puis relancer `snapshot` ; les rects pourraient être périmés",
        )
        .into_result(),
        ElementError::UnknownRef(unknown) => ToolError::new(
            ErrorReason::UnknownRef,
            format!("référence inconnue : {unknown}"),
        )
        .with_reference(unknown)
        .with_hint(
            "les références viennent de `snapshot` : `e<n>` (cibles à DOM) ou `w:<id>` \
             (widgets natifs) ; en relancer un pour obtenir les références courantes",
        )
        .into_result(),
        ElementError::Ambiguous {
            reference: dup,
            count,
        } => ToolError::new(
            ErrorReason::AmbiguousRef,
            format!("{count} éléments portent la référence {dup}"),
        )
        .with_reference(dup)
        .with_hint(
            "l'app publie des identifiants dupliqués (`deveye-server doctor` les signale) : \
             les corriger côté app, ou viser par coordonnées avec `click`",
        )
        .into_result(),
        ElementError::Obscured { by } => {
            let mut error = tagged(ToolError::new(
                ErrorReason::Obscured,
                "un autre élément recouvre le centre de la cible",
            ))
            .with_hint("fermer ce qui recouvre l'élément (menu, boîte de dialogue) puis réessayer");
            if let Some(by) = by {
                error = error.with_covered_by(by);
            }
            error.into_result()
        }
        ElementError::Unsupported { hint } => tagged(ToolError::new(
            ErrorReason::Unsupported,
            "les éléments par référence ne sont pas pris en charge sur cette cible",
        ))
        .with_hint(hint)
        .into_result(),
        ElementError::Port(port) => {
            return match unsupported_aware(Err::<(), _>(port)) {
                Err(issue) => issue,
                // Impossible : on lui passe une erreur ; garde un résultat plausible.
                Ok(()) => Ok(ok_result()),
            };
        }
        // `ElementError` est #[non_exhaustive] : une cause future reste une erreur lisible.
        other => ToolError::new(ErrorReason::InteractionFailed, other.to_string()).into_result(),
    };
    Ok(result)
}

#[tool_router(router = perception_router, vis = "pub(crate)")]
impl DeveyeMcpServer {
    #[tool(
        description = "[expérimental] Décrit les éléments de l'app cible en arbre compact, chacun avec une RÉFÉRENCE réutilisable par `act` (hors promesse semver, ADR-0073). Références : `w:<id>` pour les widgets natifs (cibles x11 et kwin-eis, identifiant fourni par l'app via la route `ui.widgets` : stable d'une relance à l'autre seulement si l'app fournit des identifiants déterministes ; les éléments `[ephemeral]` ont un identifiant dupliqué ou synthétisé, non durable : un identifiant synthétisé dont le rôle ou le nom a changé répond `stale_ref`) ; `e<n>` pour les pages web (cibles sdk et auto, script du pont JS : stable tant que le nœud vit, jamais réutilisée, périmée par un rechargement de la page ; rôle implicite et nom accessible simplifiés, iframes non parcourues, shadow roots ouverts traversés, valeur masquée pour les champs reconnus comme secrets par heuristique ou marqués `data-deveye-secret`). Texte par défaut, une ligne par élément : `- button \"Enregistrer\" [ref=w:save] [disabled]` (états `disabled`, `checked`/`unchecked`, `expanded`/`collapsed`, `selected`, `focused`, `offscreen`, `scrollable`, `ephemeral`, puis ` = \"valeur\"` ; jamais la valeur d'un widget secret) ; `focus:` désigne l'élément qui a le focus clavier ; `format: \"json\"` rend le même contenu avec les rects (aussi en `structuredContent`). Paramètres : `scope` (référence d'un sous-arbre), `max_depth`, `max_nodes` (défaut 400, plafond 2000, `truncated` si coupé), `interactive_only` (défaut true), `viewport_only` (défaut false). Les rects sont INDICATIFS : `act` remesure au moment de l'action. Sur une cible sans arbre d'éléments, répond `isError` `{ ok: false, reason: \"unsupported\", hint }` avec la marche à suivre (`target_info.elements` le dit d'avance).",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    pub(crate) async fn snapshot(
        &self,
        Parameters(p): Parameters<SnapshotParams>,
    ) -> Result<CallToolResult, McpError> {
        let format = SnapshotFormat::parse(p.format.as_deref())?;
        let query = p.query();
        let _serial = self.acquire_gate("snapshot").await;
        let snapshot = match self.elements.snapshot(&query).await {
            Ok(snapshot) => snapshot,
            Err(error) => return element_failure(error, query.scope.as_deref()),
        };
        Ok(match format {
            SnapshotFormat::Text => {
                CallToolResult::success(vec![ContentBlock::text(render_snapshot_text(&snapshot))])
            }
            SnapshotFormat::Json => structured_ok(&snapshot),
        })
    }

    #[tool(
        description = "[expérimental] Agit sur un élément désigné par sa RÉFÉRENCE (rendue par `snapshot`, hors promesse semver, ADR-0073) : `action` = `click` (`button` optionnel), `double_click`, `right_click`, `hover`, `type` (clique pour donner le focus puis saisit `text`), `scroll_into_view` (vérifie que l'élément est visible), `scroll` (molette `dx`/`dy` ancrée sur l'élément). La position est REMESURÉE au moment de l'action : seul le centre frais compte, jamais le rect d'un ancien instantané. Pas de relocalisation silencieuse : un élément disparu donne `{ ok: false, reason: \"stale_ref\", ref, hint, candidates? }` (relancer `snapshot` ; `candidates` = éléments de même rôle et de même nom), une référence inventée `unknown_ref`, un identifiant dupliqué `ambiguous_ref`, un élément recouvert `obscured` (`by`), hors vue `offscreen`, désactivé `disabled`. Réponse : `{ ok: true, action, ref, x, y }` (centre cliqué, px). Les gestes sont ceux de `click`, `double_click`, `hover`, `type_text`, `scroll` (mêmes cibles, mêmes limites ; `{ ok: true }` signifie « événements envoyés »). Un paramètre sans sens pour l'action (`text` hors `type`, `dx`/`dy` hors `scroll`, `button` hors `click`) est refusé en `invalid_params`.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    pub(crate) async fn act(
        &self,
        Parameters(p): Parameters<ActParams>,
    ) -> Result<CallToolResult, McpError> {
        let act = p.validate()?;
        let _serial = self.acquire_gate("act").await;
        let location = match self.elements.locate(&act.reference).await {
            Ok(location) => location,
            Err(error) => return element_failure(error, Some(&act.reference)),
        };
        let refused = |reason, detail: &str, hint: &str| {
            Ok(ToolError::new(reason, detail)
                .with_reference(&act.reference)
                .with_hint(hint)
                .into_result())
        };
        if location.disabled && act.action.needs_enabled() {
            return refused(
                ErrorReason::Disabled,
                "l'élément est désactivé : il n'accepte pas cette interaction",
                "attendre qu'il soit activé (`snapshot` rend `[disabled]`), ou agir autrement",
            );
        }
        if location.offscreen {
            return refused(
                ErrorReason::Offscreen,
                "l'élément est hors de la zone visible",
                "le faire apparaître (`act` `scroll` sur son conteneur, ou `scroll`), puis relancer `snapshot`",
            );
        }
        let (x, y) = location.center();
        if let Err(error) = self.ensure_in_viewport(&[(x, y)]).await {
            return refused(
                ErrorReason::Offscreen,
                &format!("centre remesuré hors viewport : {}", error.message),
                "relancer `snapshot` : la fenêtre a été redimensionnée ou l'élément déplacé",
            );
        }
        let position = Position::new(x, y);
        let commands = match act.action {
            ActAction::Click => vec![InteractionCommand::Click {
                position,
                button: act.button,
            }],
            ActAction::DoubleClick => vec![InteractionCommand::double_click(position)],
            ActAction::RightClick => vec![InteractionCommand::Click {
                position,
                button: MouseButton::Right,
            }],
            ActAction::Hover => vec![InteractionCommand::hover(position)],
            ActAction::Type => vec![
                InteractionCommand::click(position),
                InteractionCommand::type_text(act.text.clone().unwrap_or_default()),
            ],
            ActAction::Scroll => vec![InteractionCommand::scroll(position, act.dx, act.dy)],
            ActAction::ScrollIntoView => Vec::new(),
        };
        for command in commands {
            match self.interaction.dispatch(command).await {
                Ok(()) => {}
                Err(PortError::Interaction(detail)) => {
                    return Ok(ToolError::new(ErrorReason::InteractionFailed, detail)
                        .with_reference(&act.reference)
                        .into_result())
                }
                Err(other) => match unsupported_aware(Err::<(), _>(other)) {
                    Ok(()) => {}
                    Err(issue) => return issue,
                },
            }
        }
        Ok(json_success(&ActAck {
            action: act.action.name(),
            ok: true,
            reference: act.reference,
            x,
            y,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deveye_core::elements::{ElementLocation, ElementState};
    use deveye_ports::ElementResult;
    use std::sync::Mutex;

    #[test]
    fn text_rendering_is_compact_and_flags_states() {
        let snapshot = ElementSnapshot::new(
            0,
            "widgets",
            vec![ElementNode::new("w:root", "window")
                .with_name("Formulaire")
                .with_children(vec![
                    ElementNode::new("w:name", "textbox")
                        .with_name("Nom")
                        .with_value("al\nice")
                        .with_state(ElementState::new().focused(true)),
                    ElementNode::new("w:save", "button")
                        .with_name("Enregistrer")
                        .with_state(ElementState::new().disabled(true))
                        .with_offscreen(true)
                        .with_ephemeral(true)
                        .with_rect([1, 2, 3, 4]),
                    ElementNode::new("w:opt", "checkbox").with_state(
                        ElementState::new()
                            .checked(Some(false))
                            .expanded(Some(true)),
                    ),
                ])],
        )
        .with_focus(Some("w:name".into()));
        let text = render_snapshot_text(&snapshot);
        assert_eq!(
            text,
            "# widgets : 4 élément(s), génération 0\n\
             - window \"Formulaire\" [ref=w:root]\n\
             \x20 - textbox \"Nom\" [ref=w:name] [focused] = \"al ice\"\n\
             \x20 - button \"Enregistrer\" [ref=w:save] [disabled] [offscreen] [ephemeral]\n\
             \x20 - checkbox [ref=w:opt] [unchecked] [expanded]\n\
             focus: w:name\n"
        );
        assert!(!text.contains("[1, 2"), "le rect n'est pas dans le texte");
    }

    #[test]
    fn truncation_is_announced_in_the_header() {
        let snapshot = ElementSnapshot::new(3, "widgets", Vec::new()).with_truncated(true);
        let text = render_snapshot_text(&snapshot);
        assert!(
            text.contains("TRONQUÉ") && text.contains("génération 3"),
            "{text}"
        );
    }

    #[test]
    fn snapshot_params_default_to_the_documented_budget() {
        let query = SnapshotParams::default().query();
        assert_eq!(query.max_nodes, 400);
        assert!(query.interactive_only && !query.viewport_only && query.scope.is_none());
        let query = SnapshotParams {
            max_nodes: Some(1_000_000),
            scope: Some("  w:x ".into()),
            max_depth: Some(2),
            interactive_only: Some(false),
            viewport_only: Some(true),
            format: None,
        }
        .query();
        assert_eq!(query.max_nodes, SNAPSHOT_MAX_NODES_CAP);
        assert_eq!(query.scope.as_deref(), Some("w:x"));
        assert_eq!(query.max_depth, Some(2));
        assert!(!query.interactive_only && query.viewport_only);
    }

    #[test]
    fn snapshot_format_is_validated() {
        assert_eq!(SnapshotFormat::parse(None).ok(), Some(SnapshotFormat::Text));
        assert_eq!(
            SnapshotFormat::parse(Some("JSON")).ok(),
            Some(SnapshotFormat::Json)
        );
        let err = SnapshotFormat::parse(Some("yaml")).expect_err("format inconnu");
        assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
    }

    fn act(action: &str) -> ActParams {
        ActParams {
            reference: "w:save".into(),
            action: action.into(),
            text: None,
            dx: None,
            dy: None,
            button: None,
        }
    }

    fn invalid(params: ActParams) -> String {
        match params.validate() {
            Err(e) => {
                assert_eq!(e.code, rmcp::model::ErrorCode::INVALID_PARAMS);
                e.message.to_string()
            }
            Ok(_) => panic!("paramètres acceptés à tort"),
        }
    }

    #[test]
    fn act_validation_refuses_meaningless_parameters_before_any_io() {
        assert!(act("click").validate().is_ok());
        assert!(invalid(act("clique")).contains("action inconnue"));
        assert!(invalid(ActParams {
            reference: "  ".into(),
            ..act("click")
        })
        .contains("`ref` vide"));
        assert!(invalid(act("type")).contains("exige `text`"));
        assert!(invalid(ActParams {
            text: Some("x".into()),
            ..act("click")
        })
        .contains("`text`"));
        assert!(invalid(ActParams {
            dy: Some(5),
            ..act("click")
        })
        .contains("`dy`"));
        assert!(invalid(ActParams {
            button: Some("left".into()),
            ..act("hover")
        })
        .contains("`button`"));
        assert!(invalid(act("scroll")).contains("`dx` ou `dy`"));
        assert!(invalid(ActParams {
            button: Some("gauche".into()),
            ..act("click")
        })
        .contains("button invalide"));
        let ok = ActParams {
            dy: Some(-120),
            ..act("scroll")
        }
        .validate()
        .expect("scroll");
        assert_eq!((ok.dx, ok.dy), (0, -120));
        let ok = ActParams {
            button: Some("Right".into()),
            ..act("click")
        }
        .validate()
        .expect("clic droit");
        assert_eq!(ok.button, MouseButton::Right);
    }

    #[test]
    fn element_errors_map_to_typed_failures_with_hints() {
        let read = |error: ElementError, reference: Option<&str>| {
            let result = element_failure(error, reference).expect("résultat structuré");
            assert_eq!(result.is_error, Some(true));
            result.structured_content.expect("structuré")
        };
        let stale = read(
            ElementError::Stale {
                reference: "w:save".into(),
                candidates: vec!["w:save#2".into()],
            },
            None,
        );
        assert_eq!(stale["reason"], "stale_ref");
        assert_eq!(stale["ref"], "w:save");
        assert_eq!(stale["candidates"][0], "w:save#2");
        assert!(stale["hint"].as_str().unwrap().contains("snapshot"));
        let stale = read(
            ElementError::Stale {
                reference: "w:x".into(),
                candidates: Vec::new(),
            },
            None,
        );
        assert!(stale.get("candidates").is_none());
        assert_eq!(
            read(ElementError::UnknownRef("e9".into()), None)["reason"],
            "unknown_ref"
        );
        assert_eq!(
            read(
                ElementError::Ambiguous {
                    reference: "w:a".into(),
                    count: 2
                },
                None
            )["reason"],
            "ambiguous_ref"
        );
        let stale_tree = read(ElementError::StaleTree { age_ms: 42_000 }, Some("w:ok"));
        assert_eq!(stale_tree["reason"], "stale_tree");
        assert!(
            stale_tree["hint"].is_string()
                && stale_tree["detail"].as_str().unwrap().contains("42000")
        );
        let obscured = read(
            ElementError::Obscured {
                by: Some("w:modal".into()),
            },
            Some("w:ok"),
        );
        assert_eq!(
            (obscured["reason"].as_str(), obscured["by"].as_str()),
            (Some("obscured"), Some("w:modal"))
        );
        let unsupported = read(
            ElementError::Unsupported {
                hint: "exposer ui.widgets".into(),
            },
            None,
        );
        assert_eq!(unsupported["reason"], "unsupported");
        assert_eq!(unsupported["hint"], "exposer ui.widgets");
        // Une panne de liaison reste une erreur de protocole, pas un résultat sémantique.
        let outcome = element_failure(ElementError::Port(PortError::Network("coupé".into())), None);
        assert!(outcome.is_err());
        let outcome = element_failure(
            ElementError::Port(PortError::Unsupported("pas ici".into())),
            None,
        )
        .expect("unsupported est un résultat");
        assert_eq!(
            outcome.structured_content.expect("structuré")["reason"],
            "unsupported"
        );
    }

    // ------------------------------------------------------------------
    // Outils bout en bout contre un inspecteur et un récepteur espions.
    // ------------------------------------------------------------------

    struct Scripted {
        location: Mutex<ElementResult<ElementLocation>>,
        snapshot: Mutex<ElementResult<ElementSnapshot>>,
        located: Mutex<Vec<String>>,
    }

    impl Scripted {
        fn new(location: ElementResult<ElementLocation>) -> Arc<Self> {
            Arc::new(Self {
                location: Mutex::new(location),
                snapshot: Mutex::new(Ok(ElementSnapshot::new(
                    0,
                    "widgets",
                    vec![ElementNode::new("w:save", "button")
                        .with_name("Enregistrer")
                        .with_rect([1, 2, 3, 4])],
                ))),
                located: Mutex::new(Vec::new()),
            })
        }
    }

    fn copy<T: Clone>(result: &Result<T, ElementError>) -> Result<T, ElementError> {
        match result {
            Ok(v) => Ok(v.clone()),
            Err(ElementError::Stale {
                reference,
                candidates,
            }) => Err(ElementError::Stale {
                reference: reference.clone(),
                candidates: candidates.clone(),
            }),
            Err(ElementError::Unsupported { hint }) => {
                Err(ElementError::Unsupported { hint: hint.clone() })
            }
            Err(other) => Err(ElementError::UnknownRef(other.to_string())),
        }
    }

    #[async_trait::async_trait]
    impl ElementInspector for Scripted {
        async fn snapshot(&self, _query: &SnapshotQuery) -> ElementResult<ElementSnapshot> {
            copy(&self.snapshot.lock().unwrap())
        }

        async fn locate(&self, reference: &str) -> ElementResult<ElementLocation> {
            self.located.lock().unwrap().push(reference.to_string());
            copy(&self.location.lock().unwrap())
        }
    }

    struct Spy(Mutex<Vec<InteractionCommand>>);

    #[async_trait::async_trait]
    impl InteractionSink for Spy {
        async fn dispatch(&self, command: InteractionCommand) -> PortResult<()> {
            self.0.lock().unwrap().push(command);
            Ok(())
        }
    }

    /// Viewport assez grand pour que la garde de coordonnées laisse passer les centres.
    struct Wide;

    #[async_trait::async_trait]
    impl Renderer for Wide {
        async fn screenshot(&self) -> PortResult<deveye_core::Screenshot> {
            Err(PortError::Unsupported("sans capture".into()))
        }

        async fn viewport_size(&self) -> PortResult<deveye_core::Size> {
            Ok(deveye_core::Size::new(1_000, 800))
        }
    }

    fn rig(elements: Arc<Scripted>) -> (DeveyeMcpServer, Arc<Spy>) {
        let spy = Arc::new(Spy(Mutex::new(Vec::new())));
        let mut ports = crate::dispatch_tests::base_ports();
        ports.elements = elements;
        ports.interaction = spy.clone();
        ports.renderer = Arc::new(Wide);
        (DeveyeMcpServer::new(ports), spy)
    }

    fn json_of(result: &CallToolResult) -> serde_json::Value {
        serde_json::from_str(&result.content[0].as_text().expect("texte").text).expect("JSON")
    }

    fn act_call(reference: &str, action: &str) -> ActParams {
        ActParams {
            reference: reference.into(),
            action: action.into(),
            ..act(action)
        }
    }

    #[tokio::test]
    async fn act_click_hits_the_remeasured_center() {
        let elements = Scripted::new(Ok(ElementLocation::new("w:save", [200, 150, 80, 30])));
        let (server, spy) = rig(elements.clone());
        let result = server
            .act(Parameters(act_call("w:save", "click")))
            .await
            .expect("résultat");
        assert_ne!(result.is_error, Some(true));
        let body = json_of(&result);
        assert_eq!(body["ok"], true);
        assert_eq!(
            (body["x"].as_i64(), body["y"].as_i64()),
            (Some(240), Some(165))
        );
        assert_eq!(body["ref"], "w:save");
        assert_eq!(
            *spy.0.lock().unwrap(),
            vec![InteractionCommand::Click {
                position: Position::new(240, 165),
                button: MouseButton::Left
            }]
        );
        assert_eq!(*elements.located.lock().unwrap(), ["w:save"]);
    }

    #[tokio::test]
    async fn act_type_clicks_then_types_and_scroll_is_anchored() {
        let elements = Scripted::new(Ok(ElementLocation::new("w:name", [10, 20, 100, 20])));
        let (server, spy) = rig(elements);
        let mut params = act_call("w:name", "type");
        params.text = Some("alice".into());
        server.act(Parameters(params)).await.expect("type");
        let mut params = act_call("w:name", "scroll");
        params.dy = Some(240);
        server.act(Parameters(params)).await.expect("scroll");
        let calls = spy.0.lock().unwrap().clone();
        assert_eq!(
            calls,
            vec![
                InteractionCommand::click(Position::new(60, 30)),
                InteractionCommand::type_text("alice"),
                InteractionCommand::scroll(Position::new(60, 30), 0, 240),
            ]
        );
    }

    #[tokio::test]
    async fn act_refuses_disabled_and_offscreen_elements_without_emitting() {
        let disabled = Scripted::new(Ok(
            ElementLocation::new("w:a", [0, 0, 10, 10]).with_disabled(true)
        ));
        let (server, spy) = rig(disabled);
        let result = server
            .act(Parameters(act_call("w:a", "click")))
            .await
            .unwrap();
        assert_eq!(result.is_error, Some(true));
        assert_eq!(json_of(&result)["reason"], "disabled");
        // Un survol d'un élément désactivé reste permis (tooltips).
        server
            .act(Parameters(act_call("w:a", "hover")))
            .await
            .unwrap();
        assert_eq!(spy.0.lock().unwrap().len(), 1);

        let offscreen = Scripted::new(Ok(
            ElementLocation::new("w:b", [0, 0, 10, 10]).with_offscreen(true)
        ));
        let (server, spy) = rig(offscreen);
        let result = server
            .act(Parameters(act_call("w:b", "click")))
            .await
            .unwrap();
        assert_eq!(json_of(&result)["reason"], "offscreen");
        assert!(spy.0.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn act_on_a_stale_reference_reports_candidates_and_emits_nothing() {
        let elements = Scripted::new(Err(ElementError::Stale {
            reference: "w:save".into(),
            candidates: vec!["w:save#2".into()],
        }));
        let (server, spy) = rig(elements);
        let result = server
            .act(Parameters(act_call("w:save", "click")))
            .await
            .unwrap();
        assert_eq!(result.is_error, Some(true));
        let body = json_of(&result);
        assert_eq!(body["reason"], "stale_ref");
        assert!(!body["hint"].as_str().unwrap().is_empty());
        assert_eq!(body["candidates"][0], "w:save#2");
        assert!(spy.0.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn act_with_invalid_parameters_never_reaches_the_ports() {
        let elements = Scripted::new(Ok(ElementLocation::new("w:a", [0, 0, 10, 10])));
        let (server, spy) = rig(elements.clone());
        assert!(server
            .act(Parameters(act_call("w:a", "type")))
            .await
            .is_err());
        assert!(elements.located.lock().unwrap().is_empty());
        assert!(spy.0.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn snapshot_returns_compact_text_by_default_and_structured_json_on_demand() {
        let elements = Scripted::new(Ok(ElementLocation::new("w:a", [0, 0, 1, 1])));
        let (server, _) = rig(elements);
        let text = server
            .snapshot(Parameters(SnapshotParams::default()))
            .await
            .unwrap();
        assert!(text.structured_content.is_none());
        let body = text.content[0].as_text().unwrap().text.clone();
        assert!(
            body.contains("- button \"Enregistrer\" [ref=w:save]"),
            "{body}"
        );
        let params = SnapshotParams {
            format: Some("json".into()),
            ..Default::default()
        };
        let json = server.snapshot(Parameters(params)).await.unwrap();
        let structured = json.structured_content.clone().expect("structuredContent");
        assert_eq!(structured["nodes"][0]["ref"], "w:save");
        assert_eq!(
            structured["nodes"][0]["rect"],
            serde_json::json!([1, 2, 3, 4])
        );
        assert_eq!(json_of(&json), structured);
    }

    #[tokio::test]
    async fn snapshot_on_an_unsupported_target_carries_the_hint() {
        let elements = Scripted::new(Ok(ElementLocation::new("w:a", [0, 0, 1, 1])));
        *elements.snapshot.lock().unwrap() = Err(ElementError::Unsupported {
            hint: "exposer ui.widgets".into(),
        });
        let (server, _) = rig(elements);
        let result = server
            .snapshot(Parameters(SnapshotParams::default()))
            .await
            .unwrap();
        assert_eq!(result.is_error, Some(true));
        let body = json_of(&result);
        assert_eq!(
            (body["reason"].as_str(), body["hint"].as_str()),
            (Some("unsupported"), Some("exposer ui.widgets"))
        );
    }

    #[tokio::test]
    async fn observe_adds_the_elements_section_only_on_request() {
        let elements = Scripted::new(Ok(ElementLocation::new("w:a", [0, 0, 1, 1])));
        let (server, _) = rig(elements);
        let observe = |include: Option<Vec<String>>| {
            let params = crate::tools_observe::ObserveParams {
                include,
                exclude: Some(vec!["screenshot".into(), "console".into(), "logs".into()]),
                ..Default::default()
            };
            let server = server.clone();
            async move {
                let result = server.observe(Parameters(params)).await.expect("observe");
                json_of(&result)
            }
        };
        let default = observe(None).await;
        assert!(default["sections"].get("elements").is_none(), "{default}");
        assert!(
            !default["skipped"].to_string().contains("elements"),
            "{default}"
        );
        let asked = observe(Some(vec!["Elements".into()])).await;
        let section = &asked["sections"]["elements"];
        assert_eq!(section["available"], true, "{asked}");
        assert!(
            section["tree"].as_str().unwrap().contains("[ref=w:save]"),
            "{asked}"
        );
        assert_eq!(section["total_nodes"], 1);
    }

    #[tokio::test]
    async fn observe_degrades_the_elements_section_on_an_unsupported_target() {
        let elements = Scripted::new(Ok(ElementLocation::new("w:a", [0, 0, 1, 1])));
        *elements.snapshot.lock().unwrap() = Err(ElementError::Unsupported {
            hint: "exposer ui.widgets".into(),
        });
        let (server, _) = rig(elements);
        let params = crate::tools_observe::ObserveParams {
            include: Some(vec!["elements".into()]),
            ..Default::default()
        };
        let body = json_of(&server.observe(Parameters(params)).await.expect("observe"));
        let section = &body["sections"]["elements"];
        assert_eq!(section["available"], false);
        assert_eq!(section["reason"], "unsupported");
        assert!(section["detail"].as_str().unwrap().contains("ui.widgets"));
    }
}
