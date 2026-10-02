//! The `config.toml` schema.
//!
//! The embedded [`Config::DEFAULT_TOML`] (`default-config.toml`) is the
//! single source of defaults. A user file is merged over it
//! ([`Config::from_toml`]): tables merge recursively, other values
//! (including arrays such as `workspaces`, `layouts`, `rules` and
//! `scratchpads`) are replaced wholesale. The `keybindings` and
//! `pointer_bindings` tables are merged by *resolved* combination, so
//! `Super+j` in a user file overrides a default `Mod+j` when the modifier
//! is Super; the value `"none"` removes a binding. `Mod` always resolves
//! with the final (merged) `modifier`.

mod color;
mod keys;
mod merge;
#[cfg(test)]
mod tests;

use serde::{
    Deserialize,
    Deserializer,
};
use toml::Table;

pub use self::{
    color::{
        Color,
        ColorError,
    },
    keys::{
        KeyCombo,
        KeyError,
        Modifiers,
        PointerButton,
        PointerCombo,
    },
};
use crate::{
    action::{
        Action,
        ParseActionError,
    },
    geometry::{
        Ratio,
        RationalRect,
    },
    layout::{
        LayoutOrder,
        LayoutOrderError,
    },
    rules::Rule,
};

/// Errors from loading a config.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
    /// Invalid TOML or a value of the wrong type/shape (including invalid
    /// colors and unknown fields).
    #[error("{0}")]
    Toml(#[from] toml::de::Error),
    /// The `modifier` setting is invalid.
    #[error("invalid modifier: {0}")]
    InvalidModifier(KeyError),
    /// A key or pointer combination is invalid.
    #[error("invalid binding `{combo}`: {source}")]
    InvalidCombo {
        /// The combination as written.
        combo: String,
        /// Why it is invalid.
        source: KeyError,
    },
    /// A binding's action does not parse.
    #[error("invalid action for `{combo}`: {source}")]
    InvalidAction {
        /// The combination as written.
        combo: String,
        /// Why the action is invalid.
        source: ParseActionError,
    },
    /// A binding's value is not a string.
    #[error("binding `{0}` must be an action string")]
    InvalidBindingValue(String),
    /// A bindings section is not a table.
    #[error("`{0}` must be a table")]
    InvalidSection(String),
    /// `workspaces` is empty.
    #[error("`workspaces` must not be empty")]
    EmptyWorkspaces,
    /// A workspace tag is empty or duplicated.
    #[error("invalid workspace tag `{0}` (empty or duplicate)")]
    InvalidWorkspace(String),
    /// A scratchpad is invalid.
    #[error("invalid scratchpad `{name}`: {reason}")]
    InvalidScratchpad {
        /// The scratchpad's name.
        name: String,
        /// Why it is invalid.
        reason: &'static str,
    },
    /// `layouts` is empty or names an unknown or duplicate layout.
    #[error("invalid `layouts`: {0}")]
    InvalidLayouts(#[from] LayoutOrderError),
    /// A size (border width, bar/prompt/tab height, divider width) is
    /// negative.
    #[error("`{0}` must not be negative")]
    NegativeSize(&'static str),
}

/// Top or bottom edge of a screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BarPosition {
    /// Top edge.
    Top,
    /// Bottom edge.
    Bottom,
}

/// How glyph edges of oxmonad's own text are antialiased.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Antialias {
    /// One coverage value per pixel.
    #[default]
    Grayscale,
    /// Subpixel antialiasing for red, green, blue subpixel order.
    Rgb,
    /// Subpixel antialiasing for blue, green, red subpixel order.
    Bgr,
}

/// Text rendering settings shared by the bar, prompt and tab strip.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FontConfig {
    /// Glyph edge antialiasing; subpixel modes only apply over opaque
    /// backgrounds.
    pub antialias: Antialias,
}

/// Window border settings.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BorderConfig {
    /// Border width in logical pixels (`borderWidth`).
    pub width: i32,
    /// Border color of the focused window (`focusedBorderColor`).
    pub focused: Color,
    /// Border color of other windows (`normalBorderColor`).
    pub normal: Color,
    /// When windows go without a border (xmonad-contrib's `smartBorders`).
    pub smart: SmartBorders,
}

/// When windows lose their border because focus is obvious
/// (xmonad-contrib's `XMonad.Layout.NoBorders`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SmartBorders {
    /// Every window keeps its border (except under `noBorders Full`).
    Off,
    /// `smartBorders` (`lessBorders Never`): a tiled window alone on the
    /// only screen, and a floating window covering its whole screen.
    #[default]
    SingleScreen,
    /// `lessBorders Screen` plus full-screen floats: a tiled window alone
    /// on any screen, even with several screens.
    PerScreen,
}

/// Virtual screen settings (`split-screen`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SplitConfig {
    /// Width in logical pixels of the divider line drawn between two
    /// virtual screens of a split output; `0` draws none.
    pub divider_width: i32,
    /// Color of the divider line.
    pub divider_color: Color,
    /// Empty space in logical pixels between the divider and the windows
    /// on either side, so their borders do not touch it.
    pub divider_padding: i32,
}

/// Status bar colors (xmobar + `myPP`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BarColors {
    /// Bar background.
    pub background: Color,
    /// Default text color.
    pub foreground: Color,
    /// The current workspace (`ppCurrent`).
    pub current: Color,
    /// Workspaces visible on other screens (`ppVisible`).
    pub visible: Color,
    /// Hidden workspaces with windows (`ppHidden`).
    pub hidden: Color,
    /// Hidden empty workspaces (`ppHiddenNoWindows`).
    pub hidden_empty: Color,
    /// The focused window title (`ppTitle`).
    pub title: Color,
    /// The layout name (`ppLayout`).
    pub layout: Color,
}

/// Status bar settings.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "mirrors the `[bar]` table's independent on/off keys"
)]
pub struct BarConfig {
    /// Whether the bar is shown at startup (`Mod+b` toggles it).
    pub visible: bool,
    /// Screen edge the bar sits on.
    pub position: BarPosition,
    /// Height in logical pixels.
    pub height: i32,
    /// Names of the outputs that get a bar, in priority order (the first
    /// connected one is the fallback primary bar). `None` (unset) puts a
    /// bar on every output; an empty list shows no bar. A plain string is
    /// accepted as a one-element list.
    #[serde(default, deserialize_with = "string_or_list")]
    pub output: Option<Vec<String>>,
    /// Name of the output whose bar shows the right-hand side (tray,
    /// battery, clock). When unset or without a bar, the first bar output
    /// is the primary one.
    #[serde(default)]
    pub primary: Option<String>,
    /// Font family.
    pub font_family: String,
    /// Font size in points.
    pub font_size: f32,
    /// Shorten the title to this many characters (xmonad's `shorten`);
    /// `0` disables shortening.
    #[serde(deserialize_with = "zero_is_none")]
    pub title_max_len: Option<usize>,
    /// Whether to show a system tray (`StatusNotifierItem` icons) left of
    /// the clock. Read at startup and on reload.
    pub tray: bool,
    /// Tray icon size in logical pixels.
    pub tray_icon_size: u32,
    /// Whether to draw a miniature of the current layout next to its name.
    pub layout_icon: bool,
    /// Whether hidden workspaces without windows are shown (in
    /// `colors.hidden_empty`); `myPP` drops them.
    pub show_empty_workspaces: bool,
    /// Colors.
    pub colors: BarColors,
}

/// Prompt colors (`XPConfig`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromptColors {
    /// Background (`bgColor`).
    pub background: Color,
    /// Text (`fgColor`).
    pub foreground: Color,
    /// Border (`borderColor`).
    pub border: Color,
    /// Highlighted completion background (`bgHLight`).
    pub highlight_background: Color,
    /// Highlighted completion text (`fgHLight`).
    pub highlight_foreground: Color,
}

/// Prompt settings (`XPConfig`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromptConfig {
    /// Screen edge the prompt sits on (`position`).
    pub position: BarPosition,
    /// Height in logical pixels.
    pub height: i32,
    /// Font family.
    pub font_family: String,
    /// Font size in points.
    pub font_size: f32,
    /// Colors.
    pub colors: PromptColors,
}

/// Tab strip theme of the tabbed layout (xmonad-contrib `Theme`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TabConfig {
    /// Tab strip height (`decoHeight`).
    pub height: i32,
    /// Font family.
    pub font_family: String,
    /// Font size in points.
    pub font_size: f32,
    /// Focused tab background (`activeColor`).
    pub active_color: Color,
    /// Other tabs' background (`inactiveColor`).
    pub inactive_color: Color,
    /// Focused tab text (`activeTextColor`).
    pub active_text_color: Color,
    /// Other tabs' text (`inactiveTextColor`).
    pub inactive_text_color: Color,
    /// Focused tab border (`activeBorderColor`).
    pub active_border_color: Color,
    /// Other tabs' border (`inactiveBorderColor`).
    pub inactive_border_color: Color,
}

/// A keyboard binding.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KeyBinding {
    /// The key combination.
    pub combo: KeyCombo,
    /// What it does.
    pub action: Action,
}

/// A pointer binding.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PointerBinding {
    /// The button combination.
    pub combo: PointerCombo,
    /// What it does.
    pub action: Action,
}

/// A named scratchpad (xmonad-contrib `NamedScratchpad`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ScratchpadConfig {
    /// Name used by `toggle-scratchpad <name>`.
    pub name: String,
    /// Command spawned when no matching window exists.
    pub command: String,
    /// App id identifying the scratchpad's window.
    pub app_id: String,
    /// Floating geometry relative to the screen.
    pub rect: RationalRect,
}

/// The fully resolved configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    /// The modifier `Mod` stands for.
    pub modifier: Modifiers,
    /// Terminal command (`spawn-terminal`).
    pub terminal: String,
    /// Focus the window the pointer moves onto (`focusFollowsMouse`).
    pub focus_follows_mouse: bool,
    /// Initial workspace tags, in order.
    pub workspaces: Vec<String>,
    /// The layout ring of every workspace, in ring order; the first one is
    /// a workspace's default layout.
    pub layouts: LayoutOrder,
    /// Window borders.
    pub border: BorderConfig,
    /// Text rendering.
    pub font: FontConfig,
    /// Status bar.
    pub bar: BarConfig,
    /// Workspace prompt.
    pub prompt: PromptConfig,
    /// Tab strip.
    pub tabs: TabConfig,
    /// Virtual screens.
    pub split: SplitConfig,
    /// Keyboard bindings, sorted by combination.
    pub keybindings: Vec<KeyBinding>,
    /// Pointer bindings, sorted by combination.
    pub pointer_bindings: Vec<PointerBinding>,
    /// Manage rules, first match wins.
    pub rules: Vec<Rule>,
    /// Named scratchpads.
    pub scratchpads: Vec<ScratchpadConfig>,
}

/// The merged document minus the binding tables.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    modifier: String,
    terminal: String,
    focus_follows_mouse: bool,
    workspaces: Vec<String>,
    layouts: Vec<String>,
    border: BorderConfig,
    font: FontConfig,
    bar: BarConfig,
    prompt: PromptConfig,
    tabs: TabConfig,
    split: SplitConfig,
    #[serde(default)]
    rules: Vec<Rule>,
    #[serde(default)]
    scratchpads: Vec<RawScratchpad>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawScratchpad {
    name: String,
    command: String,
    app_id: String,
    rect: [f64; 4],
}

impl Config {
    /// The embedded default configuration.
    pub const DEFAULT_TOML: &str = include_str!("../../default-config.toml");

    /// The default configuration.
    #[must_use]
    pub fn defaults() -> Self {
        Self::from_toml("").expect("the embedded default config is valid")
    }

    /// Loads a user config (TOML) merged over the defaults.
    pub fn from_toml(user: &str) -> Result<Self, ConfigError> {
        let mut doc: Table = toml::from_str(Self::DEFAULT_TOML)?;
        let mut user: Table = toml::from_str(user)?;
        let default_keys = merge::take_bindings(&mut doc, "keybindings")?;
        let default_pointer = merge::take_bindings(&mut doc, "pointer_bindings")?;
        let user_keys = merge::take_bindings(&mut user, "keybindings")?;
        let user_pointer = merge::take_bindings(&mut user, "pointer_bindings")?;
        merge::deep_merge(&mut doc, user);
        let raw: RawConfig = toml::Value::Table(doc).try_into()?;

        let modifier = Modifiers::parse(&raw.modifier).map_err(ConfigError::InvalidModifier)?;
        let keybindings = merge::resolve_bindings(
            default_keys,
            user_keys,
            |combo, action| Ok((parse_key(combo, modifier)?, parse_action(combo, action)?)),
            |combo| parse_key(combo, modifier),
        )?
        .into_iter()
        .map(|(combo, action)| KeyBinding { combo, action })
        .collect();
        let pointer_bindings = merge::resolve_bindings(
            default_pointer,
            user_pointer,
            |combo, action| {
                Ok((
                    parse_pointer(combo, modifier)?,
                    parse_action(combo, action)?,
                ))
            },
            |combo| parse_pointer(combo, modifier),
        )?
        .into_iter()
        .map(|(combo, action)| PointerBinding { combo, action })
        .collect();

        validate_workspaces(&raw.workspaces)?;
        for (name, value) in [
            ("border.width", raw.border.width),
            ("bar.height", raw.bar.height),
            ("prompt.height", raw.prompt.height),
            ("tabs.height", raw.tabs.height),
            ("split.divider_width", raw.split.divider_width),
            ("split.divider_padding", raw.split.divider_padding),
        ] {
            if value < 0 {
                return Err(ConfigError::NegativeSize(name));
            }
        }
        let scratchpads = convert_scratchpads(raw.scratchpads)?;
        let layouts = LayoutOrder::parse(&raw.layouts)?;

        Ok(Self {
            modifier,
            terminal: raw.terminal,
            focus_follows_mouse: raw.focus_follows_mouse,
            workspaces: raw.workspaces,
            layouts,
            border: raw.border,
            font: raw.font,
            bar: raw.bar,
            prompt: raw.prompt,
            tabs: raw.tabs,
            split: raw.split,
            keybindings,
            pointer_bindings,
            rules: raw.rules,
            scratchpads,
        })
    }

    /// The action bound to `combo`, if any.
    #[must_use]
    pub fn key_action(&self, combo: &KeyCombo) -> Option<&Action> {
        self.keybindings
            .iter()
            .find(|binding| &binding.combo == combo)
            .map(|binding| &binding.action)
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::defaults()
    }
}

fn parse_key(combo: &str, modifier: Modifiers) -> Result<KeyCombo, ConfigError> {
    KeyCombo::parse(combo, modifier).map_err(|source| ConfigError::InvalidCombo {
        combo: combo.to_owned(),
        source,
    })
}

fn parse_pointer(combo: &str, modifier: Modifiers) -> Result<PointerCombo, ConfigError> {
    PointerCombo::parse(combo, modifier).map_err(|source| ConfigError::InvalidCombo {
        combo: combo.to_owned(),
        source,
    })
}

fn parse_action(combo: &str, action: &str) -> Result<Action, ConfigError> {
    action.parse().map_err(|source| ConfigError::InvalidAction {
        combo: combo.to_owned(),
        source,
    })
}

fn validate_workspaces(workspaces: &[String]) -> Result<(), ConfigError> {
    if workspaces.is_empty() {
        return Err(ConfigError::EmptyWorkspaces);
    }
    for (i, tag) in workspaces.iter().enumerate() {
        if tag.trim().is_empty() || workspaces[..i].contains(tag) {
            return Err(ConfigError::InvalidWorkspace(tag.clone()));
        }
    }
    Ok(())
}

fn convert_scratchpads(raw: Vec<RawScratchpad>) -> Result<Vec<ScratchpadConfig>, ConfigError> {
    let mut out: Vec<ScratchpadConfig> = Vec::with_capacity(raw.len());
    for pad in raw {
        let invalid = |reason| ConfigError::InvalidScratchpad {
            name: pad.name.clone(),
            reason,
        };
        if pad.name.trim().is_empty() {
            return Err(invalid("empty name"));
        }
        if out.iter().any(|other| other.name == pad.name) {
            return Err(invalid("duplicate name"));
        }
        if pad.rect.iter().any(|v| !(0.0..=1.0).contains(v)) {
            return Err(invalid("rect fractions must lie in 0..=1"));
        }
        let [x, y, w, h] = pad.rect.map(Ratio::from_f64);
        if w == Ratio::ZERO || h == Ratio::ZERO {
            return Err(invalid("rect width and height must be positive"));
        }
        out.push(ScratchpadConfig {
            rect: RationalRect::new(x, y, w, h),
            name: pad.name,
            command: pad.command,
            app_id: pad.app_id,
        });
    }
    Ok(out)
}

fn zero_is_none<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<usize>, D::Error> {
    Ok(Some(usize::deserialize(deserializer)?).filter(|&n| n > 0))
}

/// Deserializes `bar.output`: a list of output names, or a single name as
/// a one-element list.
fn string_or_list<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Vec<String>>, D::Error> {
    /// Either accepted form.
    #[derive(Deserialize)]
    #[serde(untagged, expecting = "an output name or a list of output names")]
    enum OneOrMany {
        /// `output = "DP-1"`.
        One(String),
        /// `output = ["DP-2", "DP-1"]`.
        Many(Vec<String>),
    }
    Ok(Some(match OneOrMany::deserialize(deserializer)? {
        OneOrMany::One(name) => vec![name],
        OneOrMany::Many(names) => names,
    }))
}
