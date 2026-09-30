//! The WM-drawn shell surfaces: the bar, the tab strips and the prompt.
//!
//! Their content is derived from core state in each manage sequence and
//! committed with `sync_next_commit`, so it changes in the same frame as
//! the windows.

use std::{
    collections::{
        BTreeMap,
        HashMap,
    },
    rc::Rc,
};

use oxmonad_core::{
    BarModel,
    PromptModel,
    Rect,
    TabStrip,
    WindowId,
    WorkspaceState as CoreWorkspaceState,
    config::{
        BarPosition,
        Color as CoreColor,
        Config,
    },
};
use oxmonad_ui::{
    BarStyle,
    BarView,
    BatteryState,
    BatteryStyle,
    Canvas,
    Color,
    Fonts,
    PpStyle,
    PromptPosition,
    PromptStyle,
    PromptView,
    Segment,
    Side,
    Tab,
    TabColors,
    TabStyle,
    TabsView,
    WorkspaceState,
    WorkspaceTag,
    format_battery,
    join_groups,
    layout_bar,
    pp_segments,
    prompt_size,
    render_bar,
    render_prompt,
    render_tabs,
    tab_at,
};
use river_protocols::window_management::{
    river_node_v1::RiverNodeV1,
    river_shell_surface_v1::RiverShellSurfaceV1,
    river_window_manager_v1::RiverWindowManagerV1,
};
use wayland_client::{
    QueueHandle,
    protocol::{
        wl_compositor::WlCompositor,
        wl_shm::WlShm,
        wl_surface::WlSurface,
    },
};

use crate::{
    plan::{
        Node,
        ShellPlacement,
    },
    shm::Buffers,
    state::State,
};

/// Points to logical pixels at 96 dpi.
fn points_to_px(points: f32) -> f32 {
    points * 96.0 / 72.0
}

fn color(c: CoreColor) -> Color {
    Color::rgba(c.r, c.g, c.b, c.a)
}

/// Loaded fonts by family; loading scans the system fonts, so each family
/// is loaded once.
#[derive(Debug, Default)]
pub struct FontCache(HashMap<String, Rc<Fonts>>);

impl FontCache {
    /// The fonts for `family`.
    pub fn get(&mut self, family: &str) -> Rc<Fonts> {
        self.0
            .entry(family.to_owned())
            .or_insert_with(|| {
                let fonts = Fonts::from_family(family);
                if fonts.family() != family {
                    tracing::info!(family, loaded = fonts.family(), "font fallback");
                }
                Rc::new(fonts)
            })
            .clone()
    }
}

/// Styles and fonts of every surface, derived from the config.
#[derive(Debug)]
pub struct Styles {
    bar: BarStyle,
    bar_fonts: Rc<Fonts>,
    pp: PpStyle,
    battery: BatteryStyle,
    prompt: PromptStyle,
    prompt_fonts: Rc<Fonts>,
    tabs: TabStyle,
    tab_fonts: Rc<Fonts>,
}

impl Styles {
    /// Styles for `config`.
    pub fn new(config: &Config, fonts: &mut FontCache) -> Self {
        let bar = &config.bar;
        let prompt = &config.prompt;
        let tabs = &config.tabs;
        Self {
            bar: BarStyle {
                bg: color(bar.colors.background),
                fg: color(bar.colors.foreground),
                font_size: points_to_px(bar.font_size),
                height: bar.height.unsigned_abs(),
                ..BarStyle::default()
            },
            bar_fonts: fonts.get(&bar.font_family),
            pp: PpStyle {
                current_fg: color(bar.colors.current),
                title_fg: color(bar.colors.title),
                // Core already shortened the title.
                title_max_chars: usize::MAX,
                ..PpStyle::default()
            },
            battery: BatteryStyle::default(),
            prompt: PromptStyle {
                bg: color(prompt.colors.background),
                fg: color(prompt.colors.foreground),
                bg_highlight: color(prompt.colors.highlight_background),
                fg_highlight: color(prompt.colors.highlight_foreground),
                border: color(prompt.colors.border),
                font_size: points_to_px(prompt.font_size),
                height: prompt.height.unsigned_abs(),
                position: match prompt.position {
                    BarPosition::Top => PromptPosition::Top,
                    BarPosition::Bottom => PromptPosition::Bottom,
                },
                ..PromptStyle::default()
            },
            prompt_fonts: fonts.get(&prompt.font_family),
            tabs: TabStyle {
                active: TabColors {
                    fill: color(tabs.active_color),
                    border: color(tabs.active_border_color),
                    text: color(tabs.active_text_color),
                },
                inactive: TabColors {
                    fill: color(tabs.inactive_color),
                    border: color(tabs.inactive_border_color),
                    text: color(tabs.inactive_text_color),
                },
                font_size: points_to_px(tabs.font_size),
                height: tabs.height.unsigned_abs(),
                ..TabStyle::default()
            },
            tab_fonts: fonts.get(&tabs.font_family),
        }
    }
}

/// The bar's status modules.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Status {
    /// The formatted clock.
    pub clock: String,
    /// The battery, if there is one.
    pub battery: Option<BatteryState>,
}

/// The bar view for `model` and `status`.
#[must_use]
pub fn bar_view(model: &BarModel, status: &Status, styles: &Styles) -> BarView {
    let tags: Vec<WorkspaceTag> = model
        .workspaces
        .iter()
        .map(|(name, state)| {
            let (state, has_windows) = match state {
                CoreWorkspaceState::Current => (WorkspaceState::Current, true),
                CoreWorkspaceState::Visible => (WorkspaceState::Visible, true),
                CoreWorkspaceState::HiddenNonEmpty => (WorkspaceState::Hidden, true),
                CoreWorkspaceState::HiddenEmpty => (WorkspaceState::Hidden, false),
            };
            WorkspaceTag {
                name: name.clone(),
                state,
                has_windows,
                urgent: false,
            }
        })
        .collect();
    let battery = status
        .battery
        .map(|state| format_battery(state, &styles.battery))
        .unwrap_or_default();
    let clock = if status.clock.is_empty() {
        Vec::new()
    } else {
        vec![Segment::new(&status.clock)]
    };
    BarView {
        segments_left: pp_segments(&tags, &model.layout, &model.title, &styles.pp),
        segments_right: join_groups([battery, clock], &styles.bar.separator),
    }
}

/// The prompt view for `model`.
#[must_use]
pub fn prompt_view(model: &PromptModel) -> PromptView {
    PromptView {
        label: model.label.clone(),
        input: model.input.clone(),
        cursor: model.cursor,
        completions: model.completions.clone(),
        selected: model.selected,
    }
}

/// What one surface shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Content {
    /// The status bar.
    Bar(BarView),
    /// A tab strip; the windows are in tab order.
    Tabs(TabsView, Vec<WindowId>),
    /// The prompt, drawn on a screen of the given logical height.
    Prompt(PromptView, u32),
}

/// A surface oxmonad wants to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wanted {
    /// Which surface.
    pub node: Node,
    /// Where it goes (for the prompt: the input line's strip).
    pub rect: Rect,
    /// Its buffer scale.
    pub scale: u32,
    /// What it shows.
    pub content: Content,
}

/// The tab strip surface content for a core tab strip.
#[must_use]
pub fn tabs_content(strip: &TabStrip) -> Content {
    let view = TabsView {
        tabs: strip
            .tabs
            .iter()
            .map(|tab| Tab {
                title: tab.title.clone(),
                focused: tab.focused,
                urgent: false,
            })
            .collect(),
    };
    Content::Tabs(view, strip.tabs.iter().map(|tab| tab.window).collect())
}

/// One live shell surface.
#[derive(Debug)]
struct Surface {
    base: WlSurface,
    shell: RiverShellSurfaceV1,
    node: RiverNodeV1,
    buffers: Buffers,
    /// What is on screen, and where.
    drawn: Option<Wanted>,
}

impl Surface {
    fn destroy(mut self) {
        self.buffers.destroy();
        self.node.destroy();
        self.shell.destroy();
        self.base.destroy();
    }
}

/// Protocol objects needed to create shell surfaces.
#[derive(Debug)]
pub struct ShellGlobals<'a> {
    /// The window manager.
    pub wm: &'a RiverWindowManagerV1,
    /// The compositor.
    pub compositor: &'a WlCompositor,
    /// Shared memory.
    pub shm: &'a WlShm,
    /// The queue.
    pub qh: &'a QueueHandle<State>,
}

/// Every shell surface.
#[derive(Debug, Default)]
pub struct Shells {
    surfaces: BTreeMap<Node, Surface>,
}

impl Shells {
    /// Makes the surfaces match `wanted`: destroys unwanted ones, creates
    /// missing ones and redraws changed ones. Must run during a manage or
    /// render sequence (redraws use `sync_next_commit`).
    pub fn sync(&mut self, wanted: Vec<Wanted>, styles: &Styles, globals: &ShellGlobals<'_>) {
        let keep: Vec<Node> = wanted.iter().map(|w| w.node).collect();
        let gone: Vec<Node> = self
            .surfaces
            .keys()
            .copied()
            .filter(|node| !keep.contains(node))
            .collect();
        for node in gone {
            if let Some(surface) = self.surfaces.remove(&node) {
                surface.destroy();
            }
        }
        for want in wanted {
            let surface = self.surfaces.entry(want.node).or_insert_with(|| {
                let surface = globals.compositor.create_surface(globals.qh, ());
                let shell = globals
                    .wm
                    .get_shell_surface(&surface, globals.qh, want.node);
                let node = shell.get_node(globals.qh, ());
                Surface {
                    base: surface,
                    shell,
                    node,
                    buffers: Buffers::default(),
                    drawn: None,
                }
            });
            if surface.drawn.as_ref() == Some(&want) {
                continue;
            }
            let canvas = draw(&want, styles);
            match surface.buffers.upload(globals.shm, &canvas, globals.qh) {
                Ok(buffer) => {
                    surface.shell.sync_next_commit();
                    surface
                        .base
                        .set_buffer_scale(i32::try_from(want.scale).unwrap_or(1));
                    surface.base.attach(Some(&buffer), 0, 0);
                    surface.base.damage_buffer(0, 0, i32::MAX, i32::MAX);
                    surface.base.commit();
                    surface.drawn = Some(want);
                }
                Err(err) => tracing::warn!(%err, "cannot allocate a shell surface buffer"),
            }
        }
    }

    /// Destroys every surface.
    pub fn destroy(&mut self) {
        for (_, surface) in std::mem::take(&mut self.surfaces) {
            surface.destroy();
        }
    }

    /// Where each drawn surface goes.
    #[must_use]
    pub fn placements(&self, styles: &Styles) -> Vec<ShellPlacement> {
        self.surfaces
            .values()
            .filter_map(|s| s.drawn.as_ref())
            .map(|want| ShellPlacement {
                node: want.node,
                rect: surface_rect(want, styles),
            })
            .collect()
    }

    /// The river shell surface of `node`, if it exists.
    #[must_use]
    pub fn shell_surface(&self, node: Node) -> Option<&RiverShellSurfaceV1> {
        self.surfaces.get(&node).map(|s| &s.shell)
    }

    /// The node of a live shell surface.
    #[must_use]
    pub fn node_proxy(&self, node: Node) -> Option<&RiverNodeV1> {
        self.surfaces.get(&node).map(|s| &s.node)
    }

    /// The clickable spans of `node` along its top edge as global
    /// `(x_start, x_end, click)` (for `dump-state`).
    #[must_use]
    pub fn click_targets(&self, node: Node, styles: &Styles) -> Vec<(i32, i32, Click)> {
        let Some(want) = self.surfaces.get(&node).and_then(|s| s.drawn.as_ref()) else {
            return Vec::new();
        };
        let rect = surface_rect(want, styles);
        let mut targets: Vec<(i32, i32, Click)> = Vec::new();
        for x in rect.x..rect.x + rect.w {
            let Some(click) = self.click(node, x, rect.y, styles) else {
                continue;
            };
            match targets.last_mut() {
                Some((_, end, last)) if *last == click && *end == x => *end = x + 1,
                _ => targets.push((x, x + 1, click)),
            }
        }
        targets
    }

    /// What a click at global `(x, y)` on shell surface `node` means.
    #[must_use]
    pub fn click(&self, node: Node, x: i32, y: i32, styles: &Styles) -> Option<Click> {
        let want = self.surfaces.get(&node)?.drawn.as_ref()?;
        let rect = surface_rect(want, styles);
        if !rect.contains(x, y) {
            return None;
        }
        let local_x = f64::from(x - rect.x);
        match &want.content {
            Content::Bar(view) => {
                let layout = layout_bar(
                    view,
                    &styles.bar,
                    &styles.bar_fonts,
                    rect.w.unsigned_abs(),
                    want.scale,
                );
                let id = layout.segment_at(local_x)?;
                (id.side == Side::Left)
                    .then(|| view.segment(id)?.action.clone())
                    .flatten()
                    .map(Click::Workspace)
            }
            Content::Tabs(view, windows) => {
                let index = tab_at(view, rect.w.unsigned_abs(), local_x)?;
                windows.get(index).copied().map(Click::Window)
            }
            Content::Prompt(..) => None,
        }
    }
}

/// What a click on a shell surface asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Click {
    /// View this workspace (a bar workspace tag).
    Workspace(String),
    /// Focus this window (a tab).
    Window(WindowId),
}

/// The logical rectangle a surface covers.
fn surface_rect(want: &Wanted, styles: &Styles) -> Rect {
    match &want.content {
        Content::Prompt(view, max_height) => {
            let (_, height) = prompt_size(
                view,
                &styles.prompt,
                &styles.prompt_fonts,
                want.rect.w.unsigned_abs(),
                *max_height,
            );
            let height = i32::try_from(height).unwrap_or(want.rect.h);
            let y = match styles.prompt.position {
                PromptPosition::Top => want.rect.y,
                PromptPosition::Bottom => want.rect.y + want.rect.h - height,
            };
            Rect::new(want.rect.x, y, want.rect.w, height)
        }
        Content::Bar(_) | Content::Tabs(..) => want.rect,
    }
}

/// Renders a surface's content.
fn draw(want: &Wanted, styles: &Styles) -> Canvas {
    let width = want.rect.w.max(1).unsigned_abs();
    match &want.content {
        Content::Bar(view) => render_bar(view, &styles.bar, &styles.bar_fonts, width, want.scale),
        Content::Tabs(view, _) => {
            let style = TabStyle {
                height: want.rect.h.max(1).unsigned_abs(),
                ..styles.tabs.clone()
            };
            render_tabs(view, &style, &styles.tab_fonts, width, want.scale)
        }
        Content::Prompt(view, max_height) => render_prompt(
            view,
            &styles.prompt,
            &styles.prompt_fonts,
            width,
            *max_height,
            want.scale,
        ),
    }
}
