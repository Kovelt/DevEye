//! Contrat `deveye.ui/1` : arbre de widgets d'une app native, servi par la route
//! de commande `ui.widgets`.
//!
//! Une app sans DOM (egui, GTK, Godot, Uno…) décrit son interface par une liste
//! **plate** de widgets, chacun pointant vers son parent : bon marché à produire
//! pour une UI en mode immédiat, et convertible sans perte vers un arbre imbriqué.
//!
//! # Repère des rectangles
//!
//! `rect` vaut `[x, y, largeur, hauteur]` en pixels entiers, **dans le même repère
//! que les gestes de la cible** (origine au coin haut-gauche de la fenêtre) :
//! cliquer au centre d'un rect revient à cliquer le widget. `window.scale` est
//! informatif. Une app dont les coordonnées sont des points logiques (egui) les
//! multiplie par son facteur d'échelle avant de publier.
//!
//! Les coordonnées fractionnaires sont ramenées à la plus petite boîte entière qui
//! contient le rectangle réel ([`rect_from_f64`]) : un clic au centre reste donc
//! dans le widget, sans décalage d'un pixel à l'arrondi.
//!
//! # Évolution
//!
//! Une évolution mineure ajoute des champs optionnels (les lecteurs ignorent
//! l'inconnu) ; une rupture publie `deveye.ui/2`, lu en plus de `/1`. Tous les types
//! sont `#[non_exhaustive]` : on les construit par leurs constructeurs, jamais par
//! littéral. Le rôle est une chaîne ouverte : une valeur inconnue est conservée.
//!
//! Spécification et schéma JSON : `docs/schemas/ui-widgets-1.json`, ADR-0063.

use std::collections::{HashMap, HashSet};
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

/// Identifiant de version porté par chaque arbre.
pub const UI_SCHEMA: &str = "deveye.ui/1";

fn default_schema() -> String {
    UI_SCHEMA.to_string()
}

fn default_true() -> bool {
    true
}

fn is_true(value: &bool) -> bool {
    *value
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// Rôle d'un widget : chaîne ouverte, vocabulaire recommandé calqué sur ARIA.
///
/// Une valeur hors vocabulaire est conservée telle quelle (un agent la lit comme
/// du texte). Vocabulaire recommandé : `window`, `panel`, `button`, `checkbox`,
/// `radio`, `textbox`, `password`, `slider`, `knob`, `list`, `listitem`, `tab`,
/// `menu`, `menuitem`, `label`, `image`, `scrollarea` (constantes `UiRole::*`).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(transparent)]
pub struct UiRole(String);

impl UiRole {
    pub const WINDOW: &'static str = "window";
    pub const PANEL: &'static str = "panel";
    pub const BUTTON: &'static str = "button";
    pub const CHECKBOX: &'static str = "checkbox";
    pub const RADIO: &'static str = "radio";
    pub const TEXTBOX: &'static str = "textbox";
    /// Champ de saisie secret : sa valeur n'est jamais publiée ni servie.
    pub const PASSWORD: &'static str = "password";
    pub const SLIDER: &'static str = "slider";
    pub const KNOB: &'static str = "knob";
    pub const LIST: &'static str = "list";
    pub const LISTITEM: &'static str = "listitem";
    pub const TAB: &'static str = "tab";
    pub const MENU: &'static str = "menu";
    pub const MENUITEM: &'static str = "menuitem";
    pub const LABEL: &'static str = "label";
    pub const IMAGE: &'static str = "image";
    pub const SCROLLAREA: &'static str = "scrollarea";

    pub fn new(role: impl Into<String>) -> Self {
        Self(role.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for UiRole {
    fn from(role: &str) -> Self {
        Self::new(role)
    }
}

impl From<String> for UiRole {
    fn from(role: String) -> Self {
        Self(role)
    }
}

impl fmt::Display for UiRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Fenêtre observée : dimensions dans le repère des rects.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct UiWindow {
    pub width: u32,
    pub height: u32,
    /// Facteur d'échelle de l'app (points logiques vers pixels) : informatif.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<f64>,
}

impl UiWindow {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            scale: None,
        }
    }

    pub fn with_scale(mut self, scale: f64) -> Self {
        self.scale = Some(scale);
        self
    }
}

/// État d'un widget. Les champs absents prennent leur valeur par défaut
/// (`enabled` et `visible` à vrai, le reste à faux ou inconnu).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct UiState {
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub enabled: bool,
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub visible: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub focused: bool,
    /// Tri-état : absent si le widget n'est pas cochable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked: Option<bool>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub selected: bool,
    /// Absent si le widget n'est pas dépliable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expanded: Option<bool>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub readonly: bool,
    /// Valeur sensible : jamais publiée par le builder. Les lecteurs
    /// ([`UiTree::from_value`], [`UiTree::from_legacy`]) conservent la valeur reçue :
    /// le masquage à la lecture est à la charge de l'appelant
    /// ([`UiTree::mask_secret_values`]) ; le serveur DevEye l'applique en amont.
    #[serde(default, skip_serializing_if = "is_false")]
    pub secret: bool,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            enabled: true,
            visible: true,
            focused: false,
            checked: None,
            selected: false,
            expanded: None,
            readonly: false,
            secret: false,
        }
    }
}

impl UiState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub fn with_visible(mut self, visible: bool) -> Self {
        self.visible = visible;
        self
    }

    pub fn with_focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }

    pub fn with_checked(mut self, checked: bool) -> Self {
        self.checked = Some(checked);
        self
    }

    pub fn with_selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn with_expanded(mut self, expanded: bool) -> Self {
        self.expanded = Some(expanded);
        self
    }

    pub fn with_readonly(mut self, readonly: bool) -> Self {
        self.readonly = readonly;
        self
    }

    pub fn with_secret(mut self, secret: bool) -> Self {
        self.secret = secret;
        self
    }
}

/// Position de défilement d'un conteneur, dans l'unité de l'app.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct UiScroll {
    pub x: f64,
    pub y: f64,
    pub max_x: f64,
    pub max_y: f64,
}

impl UiScroll {
    pub fn new(x: f64, y: f64, max_x: f64, max_y: f64) -> Self {
        Self { x, y, max_x, max_y }
    }
}

/// Un widget de l'interface.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct UiWidget {
    /// Identifiant stable fourni par l'app (unique dans l'arbre).
    pub id: String,
    /// Identifiant du parent ; absent pour une racine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    pub role: UiRole,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Valeur courante (texte d'un champ, position d'un curseur…). Jamais
    /// publiée pour un widget secret.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    /// `[x, y, largeur, hauteur]` : voir la documentation du module.
    pub rect: [i32; 4],
    #[serde(default)]
    pub state: UiState,
    /// Actions acceptées : `click`, `double_click`, `type`, `toggle`, `scroll`,
    /// `set_value`…
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scroll: Option<UiScroll>,
    /// Vrai si le widget est partiellement rogné par un conteneur défilant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clipped: Option<bool>,
    /// Vrai si `id` a été synthétisé par le lecteur du format legacy : il change
    /// avec l'ordre des widgets et ne doit pas servir de référence durable.
    #[serde(default, skip_serializing_if = "is_false")]
    pub id_ephemeral: bool,
}

impl UiWidget {
    /// Widget sans libellé ni valeur, de rect nul et d'état par défaut.
    pub fn new(id: impl Into<String>, role: impl Into<UiRole>) -> Self {
        Self {
            id: id.into(),
            parent: None,
            role: role.into(),
            label: None,
            value: None,
            rect: [0; 4],
            state: UiState::default(),
            actions: Vec::new(),
            scroll: None,
            clipped: None,
            id_ephemeral: false,
        }
    }

    pub fn with_parent(mut self, parent: impl Into<String>) -> Self {
        self.parent = Some(parent.into());
        self
    }

    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn with_value(mut self, value: impl Into<Value>) -> Self {
        self.value = Some(value.into());
        self
    }

    pub fn with_rect(mut self, rect: [i32; 4]) -> Self {
        self.rect = rect;
        self
    }

    /// Rect à partir de coordonnées fractionnaires, arrondi selon
    /// [`rect_from_f64`] ; ignoré si les valeurs ne sont pas exploitables.
    pub fn with_rect_f64(mut self, x: f64, y: f64, width: f64, height: f64) -> Self {
        if let Some(rect) = rect_from_f64(x, y, width, height) {
            self.rect = rect;
        }
        self
    }

    pub fn with_state(mut self, state: UiState) -> Self {
        self.state = state;
        self
    }

    /// Marque le widget secret : sa valeur sera omise à la construction de l'arbre.
    pub fn with_secret(mut self, secret: bool) -> Self {
        self.state.secret = secret;
        self
    }

    pub fn with_actions<I, S>(mut self, actions: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.actions = actions.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_scroll(mut self, scroll: UiScroll) -> Self {
        self.scroll = Some(scroll);
        self
    }

    pub fn with_clipped(mut self, clipped: bool) -> Self {
        self.clipped = Some(clipped);
        self
    }

    /// Vrai si la valeur ne doit jamais être exposée : `state.secret` ou rôle
    /// `password` (insensible à la casse et aux espaces de bord, comme le
    /// masquage du serveur).
    pub fn is_secret(&self) -> bool {
        self.state.secret
            || self
                .role
                .as_str()
                .trim()
                .eq_ignore_ascii_case(UiRole::PASSWORD)
    }

    /// Centre du rect, où cliquer pour viser le widget.
    pub fn center(&self) -> (i32, i32) {
        let [x, y, w, h] = self.rect;
        (x.saturating_add(w / 2), y.saturating_add(h / 2))
    }
}

/// JSON Schema (brouillon 2020-12) de [`UiTree`], publié dans
/// `docs/schemas/ui-widgets-1.json`. Disponible avec la feature `schema`.
///
/// Le fichier versionné est comparé à cette sortie par un test, après
/// normalisation (les valeurs JSON, pas l'ordre des clés ni la mise en forme).
#[cfg(feature = "schema")]
pub fn json_schema() -> Value {
    let mut schema = serde_json::to_value(schemars::schema_for!(UiTree)).unwrap_or(Value::Null);
    if let Some(object) = schema.as_object_mut() {
        object.insert(
            "$id".to_string(),
            Value::String(
                "https://github.com/Morgan-Voltz/DevEye/docs/schemas/ui-widgets-1.json".to_string(),
            ),
        );
        object.insert(
            "title".to_string(),
            Value::String("DevEye deveye.ui/1 : réponse de la route ui.widgets".to_string()),
        );
    }
    schema
}

/// Arrondit un rectangle fractionnaire vers la plus petite boîte entière qui le
/// contient : `x` et `y` au plancher, bords droit et bas au plafond, la taille
/// étant la différence. Rend `None` si une valeur n'est pas finie ou si la taille
/// est négative.
pub fn rect_from_f64(x: f64, y: f64, width: f64, height: f64) -> Option<[i32; 4]> {
    if ![x, y, width, height].iter().all(|v| v.is_finite()) || width < 0.0 || height < 0.0 {
        return None;
    }
    // `as` sature aux bornes de i32 : une coordonnée aberrante ne panique pas.
    let left = x.floor() as i32;
    let top = y.floor() as i32;
    let right = (x + width).ceil() as i32;
    let bottom = (y + height).ceil() as i32;
    Some([
        left,
        top,
        right.saturating_sub(left),
        bottom.saturating_sub(top),
    ])
}

/// Arbre de widgets d'une app à un instant.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct UiTree {
    /// Toujours [`UI_SCHEMA`] pour les arbres produits par cette version.
    #[serde(default = "default_schema")]
    pub schema: String,
    /// Compteur croissant propre à l'app, incrémenté à chaque publication.
    #[serde(default)]
    pub generation: u64,
    pub window: UiWindow,
    /// Identifiant du widget qui a le focus clavier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus: Option<String>,
    /// Instant de publication (ms depuis l'epoch Unix) : sert à repérer un arbre
    /// périmé (UI gelée).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published_at_ms: Option<u64>,
    #[serde(default)]
    pub widgets: Vec<UiWidget>,
}

impl UiTree {
    /// Arbre vide pour la fenêtre donnée.
    pub fn new(window: UiWindow) -> Self {
        Self {
            schema: default_schema(),
            generation: 0,
            window,
            focus: None,
            published_at_ms: None,
            widgets: Vec::new(),
        }
    }

    pub fn builder(width: u32, height: u32) -> UiTreeBuilder {
        UiTreeBuilder::new(width, height)
    }

    pub fn widget(&self, id: &str) -> Option<&UiWidget> {
        self.widgets.iter().find(|w| w.id == id)
    }

    /// Supprime la valeur de tout widget secret.
    pub fn strip_secret_values(&mut self) {
        for widget in self.widgets.iter_mut().filter(|w| w.is_secret()) {
            widget.value = None;
        }
    }

    /// Remplace par `placeholder` la valeur présente de tout widget secret (l'absence
    /// de valeur reste visible : on ne prétend pas qu'il y a un secret à cacher).
    pub fn mask_secret_values(&mut self, placeholder: &str) {
        for widget in self.widgets.iter_mut().filter(|w| w.is_secret()) {
            if widget.value.is_some() {
                widget.value = Some(Value::String(placeholder.to_string()));
            }
        }
    }

    /// Contrôle les invariants de l'arbre. Une liste vide signifie un arbre sain ;
    /// voir [`UiIssue::is_error`] pour distinguer erreurs et avertissements.
    pub fn validate(&self) -> Vec<UiIssue> {
        let mut issues = Vec::new();
        let issue = |kind, widget: Option<&str>, message: String| UiIssue {
            kind,
            widget: widget.map(str::to_string),
            message,
        };

        if self.schema != UI_SCHEMA {
            issues.push(issue(
                UiIssueKind::UnknownSchema,
                None,
                format!("schéma `{}` au lieu de `{UI_SCHEMA}`", self.schema),
            ));
        }

        let mut seen: HashSet<&str> = HashSet::new();
        let mut reported: HashSet<&str> = HashSet::new();
        for widget in &self.widgets {
            if widget.id.is_empty() {
                issues.push(issue(
                    UiIssueKind::EmptyId,
                    None,
                    "widget sans identifiant".to_string(),
                ));
            } else if !seen.insert(widget.id.as_str()) && reported.insert(widget.id.as_str()) {
                issues.push(issue(
                    UiIssueKind::DuplicateId,
                    Some(&widget.id),
                    format!("identifiant `{}` dupliqué", widget.id),
                ));
            }
        }

        let mut parents: HashMap<&str, Option<&str>> = HashMap::new();
        for widget in &self.widgets {
            parents
                .entry(widget.id.as_str())
                .or_insert(widget.parent.as_deref());
        }
        for widget in &self.widgets {
            if let Some(parent) = widget.parent.as_deref() {
                if !parents.contains_key(parent) {
                    issues.push(issue(
                        UiIssueKind::UnknownParent,
                        Some(&widget.id),
                        format!("parent `{parent}` de `{}` introuvable", widget.id),
                    ));
                }
            }
        }

        // Un widget est « dans un cycle » si remonter ses parents le ramène à lui ;
        // les widgets qui mènent seulement à un cycle ne sont pas signalés.
        for widget in &self.widgets {
            let mut current = widget.parent.as_deref();
            let mut steps = 0;
            while let Some(id) = current {
                if id == widget.id {
                    issues.push(issue(
                        UiIssueKind::ParentCycle,
                        Some(&widget.id),
                        format!("`{}` est son propre ancêtre", widget.id),
                    ));
                    break;
                }
                steps += 1;
                if steps > self.widgets.len() {
                    break;
                }
                current = parents.get(id).copied().flatten();
            }
        }

        if let Some(focus) = self.focus.as_deref() {
            if !parents.contains_key(focus) {
                issues.push(issue(
                    UiIssueKind::UnknownFocus,
                    Some(focus),
                    format!("le focus désigne `{focus}`, absent de l'arbre"),
                ));
            }
        }

        let window_known = self.window.width > 0 && self.window.height > 0;
        for widget in &self.widgets {
            let [x, y, w, h] = widget.rect;
            if w < 0 || h < 0 {
                issues.push(issue(
                    UiIssueKind::NegativeSize,
                    Some(&widget.id),
                    format!("rect de `{}` de taille négative", widget.id),
                ));
                continue;
            }
            let skipped = !widget.state.visible || widget.clipped == Some(true);
            if window_known && !skipped && w > 0 && h > 0 {
                let (right, bottom) = (i64::from(x) + i64::from(w), i64::from(y) + i64::from(h));
                let outside = right <= 0
                    || bottom <= 0
                    || i64::from(x) >= i64::from(self.window.width)
                    || i64::from(y) >= i64::from(self.window.height);
                if outside {
                    issues.push(issue(
                        UiIssueKind::OutsideWindow,
                        Some(&widget.id),
                        format!("rect de `{}` entièrement hors de la fenêtre", widget.id),
                    ));
                }
            }
        }
        issues
    }

    /// Lit la réponse de la route `ui.widgets`, au format `deveye.ui/1` ou au format
    /// legacy d'avant le schéma (voir [`UiTree::from_legacy`]).
    pub fn from_value(value: &Value) -> Result<Self, UiReadError> {
        if let Some(schema) = value.get("schema") {
            let schema = schema
                .as_str()
                .ok_or_else(|| UiReadError::Invalid("`schema` n'est pas une chaîne".into()))?;
            if schema != UI_SCHEMA {
                return Err(UiReadError::UnsupportedSchema(schema.to_string()));
            }
            return serde_json::from_value(value.clone())
                .map_err(|e| UiReadError::Invalid(e.to_string()));
        }
        Self::from_legacy(value)
    }

    /// Lecteur tolérant du format sans champ `schema`, tel que les apps l'ont publié
    /// avant `deveye.ui/1` : soit un objet `{ window?, widgets: [...] }`, soit
    /// directement la liste des widgets.
    ///
    /// Par widget : `kind` (ou `role`) donne le rôle, `text` (ou `label`) le libellé,
    /// `rect` est un tableau `[x, y, l, h]` ou un objet `{ x, y, width, height }`.
    /// Un widget sans `id` reçoit `role[index]`, marqué [`UiWidget::id_ephemeral`].
    /// Les champs inconnus sont ignorés.
    pub fn from_legacy(value: &Value) -> Result<Self, UiReadError> {
        let (window, widgets) = match value {
            Value::Array(items) => (UiWindow::new(0, 0), items),
            Value::Object(map) => {
                let widgets = map
                    .get("widgets")
                    .and_then(Value::as_array)
                    .ok_or_else(|| {
                        UiReadError::Invalid(
                            "ni `schema` ni liste `widgets` dans la réponse".into(),
                        )
                    })?;
                (legacy_window(map.get("window")), widgets)
            }
            _ => return Err(UiReadError::NotAnObject),
        };
        let mut tree = UiTree::new(window);
        for (index, item) in widgets.iter().enumerate() {
            tree.widgets.push(legacy_widget(index, item)?);
        }
        Ok(tree)
    }
}

fn legacy_window(value: Option<&Value>) -> UiWindow {
    let dimension = |key: &str| {
        value
            .and_then(|w| w.get(key))
            .and_then(Value::as_f64)
            .filter(|v| v.is_finite() && *v > 0.0)
            .map_or(0, |v| v.round() as u32)
    };
    UiWindow::new(dimension("width"), dimension("height"))
}

fn legacy_widget(index: usize, item: &Value) -> Result<UiWidget, UiReadError> {
    let map = item
        .as_object()
        .ok_or_else(|| UiReadError::Invalid(format!("widget {index} : objet attendu")))?;
    let text = |keys: &[&str]| {
        keys.iter()
            .find_map(|key| map.get(*key).and_then(Value::as_str))
            .map(str::to_string)
    };
    let role = text(&["kind", "role"]).unwrap_or_else(|| "unknown".to_string());
    let explicit_id = match map.get("id") {
        Some(Value::String(id)) if !id.is_empty() => Some(id.clone()),
        Some(Value::Number(id)) => Some(id.to_string()),
        _ => None,
    };
    let id_ephemeral = explicit_id.is_none();
    let id = explicit_id.unwrap_or_else(|| format!("{role}[{index}]"));
    let rect = match map.get("rect") {
        None | Some(Value::Null) => [0; 4],
        Some(raw) => legacy_rect(raw)
            .ok_or_else(|| UiReadError::Invalid(format!("widget {index} : `rect` illisible")))?,
    };
    let mut widget = UiWidget::new(id, role).with_rect(rect);
    widget.id_ephemeral = id_ephemeral;
    widget.label = text(&["label", "text"]);
    widget.parent = map
        .get("parent")
        .and_then(Value::as_str)
        .map(str::to_string);
    widget.value = map.get("value").filter(|v| !v.is_null()).cloned();
    widget.state.secret = ["secret"]
        .iter()
        .filter_map(|key| map.get(*key).and_then(Value::as_bool))
        .chain(
            map.get("state")
                .and_then(|state| state.get("secret"))
                .and_then(Value::as_bool),
        )
        .any(|flag| flag);
    Ok(widget)
}

fn legacy_rect(raw: &Value) -> Option<[i32; 4]> {
    let numbers: Vec<f64> = match raw {
        // Un `null` isolé vaut 0 : une app legacy en cours de mise en page ne doit pas
        // rendre tout l'arbre illisible.
        Value::Array(items) if items.len() == 4 => items
            .iter()
            .map(|item| match item {
                Value::Null => Some(0.0),
                other => other.as_f64(),
            })
            .collect::<Option<_>>()?,
        Value::Object(map) => {
            let get = |keys: &[&str]| {
                keys.iter()
                    .find_map(|k| map.get(*k).and_then(Value::as_f64))
            };
            vec![
                get(&["x"])?,
                get(&["y"])?,
                get(&["width", "w"])?,
                get(&["height", "h"])?,
            ]
        }
        _ => return None,
    };
    let [x, y, width, height] = [numbers[0], numbers[1], numbers[2], numbers[3]];
    if width < 0.0 || height < 0.0 {
        // Taille négative conservée telle quelle : `validate` la relève (`NegativeSize`)
        // au lieu de faire échouer la lecture.
        if ![x, y, width, height].iter().all(|v| v.is_finite()) {
            return None;
        }
        return Some([x as i32, y as i32, width as i32, height as i32]);
    }
    rect_from_f64(x, y, width, height)
}

/// Construit un [`UiTree`] ; `build` omet la valeur des widgets secrets.
#[derive(Clone, Debug)]
pub struct UiTreeBuilder {
    tree: UiTree,
}

impl UiTreeBuilder {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            tree: UiTree::new(UiWindow::new(width, height)),
        }
    }

    pub fn scale(mut self, scale: f64) -> Self {
        self.tree.window.scale = Some(scale);
        self
    }

    pub fn generation(mut self, generation: u64) -> Self {
        self.tree.generation = generation;
        self
    }

    pub fn focus(mut self, id: impl Into<String>) -> Self {
        self.tree.focus = Some(id.into());
        self
    }

    pub fn published_at_ms(mut self, millis: u64) -> Self {
        self.tree.published_at_ms = Some(millis);
        self
    }

    pub fn widget(mut self, widget: UiWidget) -> Self {
        self.tree.widgets.push(widget);
        self
    }

    pub fn widgets(mut self, widgets: impl IntoIterator<Item = UiWidget>) -> Self {
        self.tree.widgets.extend(widgets);
        self
    }

    pub fn build(mut self) -> UiTree {
        self.tree.strip_secret_values();
        self.tree
    }
}

/// Famille d'un écart relevé par [`UiTree::validate`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum UiIssueKind {
    UnknownSchema,
    EmptyId,
    DuplicateId,
    UnknownParent,
    ParentCycle,
    UnknownFocus,
    NegativeSize,
    /// Avertissement : un widget hors fenêtre peut être légitime (contenu défilant).
    OutsideWindow,
}

/// Un écart relevé par [`UiTree::validate`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct UiIssue {
    pub kind: UiIssueKind,
    /// Widget concerné, si l'écart en désigne un.
    pub widget: Option<String>,
    pub message: String,
}

impl UiIssue {
    /// Vrai pour un écart bloquant, faux pour un simple avertissement.
    pub fn is_error(&self) -> bool {
        !matches!(self.kind, UiIssueKind::OutsideWindow)
    }
}

impl fmt::Display for UiIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

/// Échec de lecture de la réponse de `ui.widgets`.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum UiReadError {
    #[error("la réponse de ui.widgets n'est ni un objet ni une liste")]
    NotAnObject,
    #[error("schéma `{0}` non pris en charge (attendu `deveye.ui/1`)")]
    UnsupportedSchema(String),
    #[error("arbre ui.widgets invalide : {0}")]
    Invalid(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample() -> UiTree {
        UiTree::builder(800, 600)
            .scale(2.0)
            .generation(7)
            .focus("name")
            .widget(UiWidget::new("root", UiRole::WINDOW).with_rect([0, 0, 800, 600]))
            .widget(
                UiWidget::new("ok", UiRole::BUTTON)
                    .with_parent("root")
                    .with_label("Valider")
                    .with_rect([40, 120, 96, 28])
                    .with_actions(["click"]),
            )
            .widget(
                UiWidget::new("name", UiRole::TEXTBOX)
                    .with_parent("root")
                    .with_value("Alice")
                    .with_rect([40, 60, 200, 28]),
            )
            .build()
    }

    #[test]
    fn rect_rounding_contains_the_real_rectangle() {
        assert_eq!(rect_from_f64(0.9, 1.0, 1.0, 2.5), Some([0, 1, 2, 3]));
        assert_eq!(rect_from_f64(10.0, 20.0, 5.0, 5.0), Some([10, 20, 5, 5]));
        assert_eq!(rect_from_f64(-0.5, 0.0, 1.0, 1.0), Some([-1, 0, 2, 1]));
        assert_eq!(rect_from_f64(f64::NAN, 0.0, 1.0, 1.0), None);
        assert_eq!(rect_from_f64(0.0, 0.0, -1.0, 1.0), None);
        assert!(rect_from_f64(0.0, 0.0, 1e30, 1.0).is_some());
    }

    #[test]
    fn tree_roundtrips_through_json() {
        let tree = sample();
        let value = serde_json::to_value(&tree).unwrap();
        assert_eq!(value["schema"], UI_SCHEMA);
        assert_eq!(UiTree::from_value(&value).unwrap(), tree);
    }

    #[test]
    fn default_state_is_omitted_from_the_wire() {
        let value = serde_json::to_value(sample()).unwrap();
        assert_eq!(value["widgets"][1].get("state"), Some(&json!({})));
        assert!(value["widgets"][1].get("id_ephemeral").is_none());
    }

    #[test]
    fn unknown_fields_and_roles_are_tolerated() {
        let value = json!({
            "schema": "deveye.ui/1",
            "window": { "width": 10, "height": 10, "future": 1 },
            "widgets": [{ "id": "a", "role": "gauge", "rect": [0, 0, 1, 1], "extra": true }],
            "added_in_1_3": []
        });
        let tree = UiTree::from_value(&value).unwrap();
        assert_eq!(tree.widgets[0].role.as_str(), "gauge");
        assert!(tree.widgets[0].state.enabled);
    }

    #[test]
    fn other_schema_versions_are_refused_by_name() {
        let err = UiTree::from_value(&json!({ "schema": "deveye.ui/2" })).unwrap_err();
        assert_eq!(err, UiReadError::UnsupportedSchema("deveye.ui/2".into()));
        assert!(UiTree::from_value(&json!("x")).is_err());
        assert!(UiTree::from_value(&json!({ "foo": 1 })).is_err());
    }

    #[test]
    fn legacy_object_format_is_converted() {
        let value = json!({
            "window": { "width": 640, "height": 480 },
            "widgets": [
                { "id": "ok", "kind": "button", "label": "OK", "rect": [1.5, 2, 10, 4] },
                { "kind": "label", "text": "Titre", "rect": { "x": 0, "y": 0, "width": 5, "height": 5 } }
            ]
        });
        let tree = UiTree::from_value(&value).unwrap();
        assert_eq!(tree.schema, UI_SCHEMA);
        assert_eq!(tree.window.width, 640);
        assert_eq!(tree.widgets[0].rect, [1, 2, 11, 4]);
        assert!(!tree.widgets[0].id_ephemeral);
        assert_eq!(tree.widgets[1].id, "label[1]");
        assert!(tree.widgets[1].id_ephemeral);
        assert_eq!(tree.widgets[1].label.as_deref(), Some("Titre"));
        assert!(tree.validate().iter().all(|i| !i.is_error()));
    }

    #[test]
    fn legacy_root_array_is_converted() {
        let value = json!([{ "id": "valider", "kind": "button", "text": "Valider",
            "rect": { "x": 40, "y": 120, "width": 96, "height": 28 } }]);
        let tree = UiTree::from_value(&value).unwrap();
        assert_eq!(tree.window, UiWindow::new(0, 0));
        assert_eq!(tree.widgets[0].rect, [40, 120, 96, 28]);
        assert_eq!(tree.widgets[0].center(), (88, 134));
        assert!(tree.validate().is_empty());
    }

    #[test]
    fn legacy_garbage_rect_is_an_error() {
        let value = json!([{ "id": "a", "kind": "button", "rect": "oops" }]);
        assert!(matches!(
            UiTree::from_value(&value),
            Err(UiReadError::Invalid(_))
        ));
    }

    #[test]
    fn legacy_negative_or_null_rect_is_kept_and_flagged_by_validate() {
        let value = json!([
            { "id": "a", "kind": "button", "rect": [0, 0, 10, -1] },
            { "id": "b", "kind": "button", "rect": [1, null, 10, 10] }
        ]);
        let tree = UiTree::from_value(&value).expect("lecture tolérante");
        assert_eq!(tree.widgets[0].rect, [0, 0, 10, -1]);
        assert_eq!(tree.widgets[1].rect, [1, 0, 10, 10]);
        assert!(tree
            .validate()
            .iter()
            .any(|i| i.kind == UiIssueKind::NegativeSize));
    }

    #[test]
    fn builder_omits_secret_values() {
        let tree = UiTree::builder(100, 100)
            .widget(
                UiWidget::new("pin", UiRole::TEXTBOX)
                    .with_value("1234")
                    .with_secret(true),
            )
            .widget(UiWidget::new("pw", UiRole::PASSWORD).with_value("hunter2"))
            .widget(UiWidget::new("user", UiRole::TEXTBOX).with_value("alice"))
            .build();
        assert_eq!(tree.widgets[0].value, None);
        assert_eq!(tree.widgets[1].value, None);
        assert_eq!(tree.widgets[2].value, Some(json!("alice")));
    }

    #[test]
    fn secret_role_is_case_and_space_insensitive_and_legacy_flag_is_read() {
        for role in ["password", "Password", "PASSWORD", " password "] {
            assert!(UiWidget::new("a", role).is_secret(), "{role}");
        }
        assert!(!UiWidget::new("a", "textbox").is_secret());
        let tree = UiTree::from_value(&json!([
            { "kind": "textbox", "secret": true, "value": "x" },
            { "kind": "textbox", "state": { "secret": true }, "value": "y" },
            { "kind": "textbox", "value": "z" }
        ]))
        .expect("arbre legacy lisible");
        let flags: Vec<bool> = tree.widgets.iter().map(UiWidget::is_secret).collect();
        assert_eq!(flags, [true, true, false]);
    }

    #[test]
    fn mask_secret_values_replaces_only_present_values() {
        let mut tree = UiTree::new(UiWindow::new(10, 10));
        tree.widgets
            .push(UiWidget::new("pw", UiRole::PASSWORD).with_value("hunter2"));
        tree.widgets.push(UiWidget::new("empty", UiRole::PASSWORD));
        tree.mask_secret_values("<secret>");
        assert_eq!(tree.widgets[0].value, Some(json!("<secret>")));
        assert_eq!(tree.widgets[1].value, None);
    }

    #[test]
    fn validate_reports_each_class_of_error() {
        let mut tree = UiTree::new(UiWindow::new(100, 100));
        tree.focus = Some("ghost".into());
        tree.widgets = vec![
            UiWidget::new("a", "panel").with_rect([0, 0, 10, 10]),
            UiWidget::new("a", "panel").with_rect([0, 0, 10, 10]),
            UiWidget::new("", "panel"),
            UiWidget::new("b", "panel").with_parent("zzz"),
            UiWidget::new("c", "panel").with_parent("d"),
            UiWidget::new("d", "panel").with_parent("c"),
            UiWidget::new("e", "panel").with_parent("e"),
            UiWidget::new("f", "panel").with_rect([0, 0, -1, 4]),
            UiWidget::new("g", "panel").with_rect([500, 500, 10, 10]),
        ];
        let kinds: Vec<UiIssueKind> = tree.validate().iter().map(|i| i.kind).collect();
        for kind in [
            UiIssueKind::DuplicateId,
            UiIssueKind::EmptyId,
            UiIssueKind::UnknownParent,
            UiIssueKind::ParentCycle,
            UiIssueKind::UnknownFocus,
            UiIssueKind::NegativeSize,
            UiIssueKind::OutsideWindow,
        ] {
            assert!(kinds.contains(&kind), "{kind:?} non détecté : {kinds:?}");
        }
        assert_eq!(
            kinds
                .iter()
                .filter(|k| **k == UiIssueKind::ParentCycle)
                .count(),
            3,
            "c, d et e sont chacun dans un cycle"
        );
        let outside = tree
            .validate()
            .into_iter()
            .find(|i| i.kind == UiIssueKind::OutsideWindow)
            .unwrap();
        assert!(!outside.is_error());
    }

    #[test]
    fn hidden_or_clipped_widgets_are_not_flagged_outside() {
        let mut tree = UiTree::new(UiWindow::new(100, 100));
        tree.widgets = vec![
            UiWidget::new("a", "panel")
                .with_rect([500, 0, 10, 10])
                .with_clipped(true),
            UiWidget::new("b", "panel")
                .with_rect([500, 0, 10, 10])
                .with_state(UiState::new().with_visible(false)),
        ];
        assert!(tree.validate().is_empty());
    }

    #[test]
    fn sample_tree_is_valid() {
        assert!(sample().validate().is_empty());
    }
}
