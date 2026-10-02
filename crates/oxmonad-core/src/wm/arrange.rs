//! Computing the [`Arrangement`]: the complete desired window state.

use std::collections::BTreeSet;

use super::Wm;
use crate::{
    config::{
        BarPosition,
        Color,
        SmartBorders,
    },
    geometry::{
        OutputId,
        Rect,
        ScreenId,
        WindowId,
    },
    stack_set::{
        Screen,
        Stack,
    },
};

/// Stacking layer of a placed window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Layer {
    /// Placed by the workspace layout.
    Tiled,
    /// Floating above the tiled windows.
    Floating,
    /// Fullscreen, covering the whole output above everything else
    /// (river's `fullscreen` on the placement's output).
    Fullscreen,
    /// Fullscreen on a virtual screen of a split output: covers exactly
    /// the screen's rectangle, stacked like [`Self::Fullscreen`]. river is
    /// not asked to make the window fullscreen (that would cover the whole
    /// output); the client is only told that it is.
    VirtualFullscreen,
}

impl Layer {
    /// Whether the window is fullscreen (on its output or its screen).
    #[must_use]
    pub const fn is_fullscreen(self) -> bool {
        matches!(self, Self::Fullscreen | Self::VirtualFullscreen)
    }
}

/// A window border drawn by the compositor outside the window content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Border {
    /// Width in logical pixels.
    pub width: i32,
    /// Color.
    pub color: Color,
}

/// Where and how one visible window is shown.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Placement {
    /// The window.
    pub window: WindowId,
    /// The screen it is shown on.
    pub screen: ScreenId,
    /// The output of that screen.
    pub output: OutputId,
    /// The content rectangle (what to propose and position). The border,
    /// if any, is drawn outside it, so content plus border fill the slot
    /// the layout computed (like xmonad's `tileWindow`).
    pub rect: Rect,
    /// Stacking layer.
    pub layer: Layer,
    /// The border to draw, `None` for borderless windows.
    pub border: Option<Border>,
}

/// Where keyboard focus goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Focus {
    /// This window.
    Window(WindowId),
    /// The workspace prompt.
    Prompt,
    /// The layout picker.
    LayoutPicker,
    /// The key binding help overlay.
    Help,
    /// Nothing (an empty workspace).
    None,
}

/// One tab of a tab strip.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Tab {
    /// The window the tab stands for.
    pub window: WindowId,
    /// The window title.
    pub title: String,
    /// Whether this is the focused window of the workspace.
    pub focused: bool,
    /// The tab's rectangle within the strip.
    pub rect: Rect,
}

/// The tab strip of a screen using the tabbed layout (view model for the
/// UI crate).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TabStrip {
    /// The whole strip.
    pub rect: Rect,
    /// The tabs in stack order.
    pub tabs: Vec<Tab>,
}

/// Per-screen shell surfaces.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ScreenArrangement {
    /// The screen.
    pub screen: ScreenId,
    /// The output of that screen.
    pub output: OutputId,
    /// The tab strip, if the screen's layout is tabbed and shows tabs.
    pub tab_strip: Option<TabStrip>,
}

/// The complete desired state, derived from the model by [`Wm::arrange`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Arrangement {
    /// Visible windows, bottom to top.
    pub windows: Vec<Placement>,
    /// Managed windows that must be hidden.
    pub hidden: Vec<WindowId>,
    /// Keyboard focus.
    pub focus: Focus,
    /// Per-screen tab strips, in screen order.
    pub screens: Vec<ScreenArrangement>,
    /// The bar's output and rectangle while it is shown. It spans the
    /// whole usable width of the output, across all its virtual screens.
    pub bar: Option<(OutputId, Rect)>,
    /// Where the prompt goes while it is open.
    pub prompt: Option<(ScreenId, Rect)>,
    /// While the layout picker is open: its screen and the area it is
    /// centred in (the screen's usable rectangle).
    pub layout_picker: Option<(ScreenId, Rect)>,
    /// While the help overlay is open: its screen (the current one) and the
    /// area it is centred in (the screen's usable rectangle).
    pub help: Option<(ScreenId, Rect)>,
}

impl Arrangement {
    /// The placement of `window`, if it is visible.
    #[must_use]
    pub fn placement(&self, window: WindowId) -> Option<&Placement> {
        self.windows.iter().find(|p| p.window == window)
    }
}

/// Placements collected per stacking layer.
#[derive(Debug, Default)]
struct Layers {
    tiled: Vec<Placement>,
    floating: Vec<Placement>,
    fullscreen: Vec<Placement>,
}

impl Wm {
    /// The output the bar is shown on, if it is visible: the output named
    /// by `bar.output`, else the first output. Whether it is visible
    /// follows the workspace on that output (see [`Wm::bar_visible_on`]):
    /// the current screen's if it is on the output, else the output's
    /// first screen's (a split output has several).
    #[must_use]
    pub fn bar_output(&self) -> Option<OutputId> {
        let outputs = self.stack_set.outputs();
        let output = self
            .config
            .bar
            .output
            .as_ref()
            .and_then(|name| outputs.iter().find(|o| &o.name == name))
            .or_else(|| outputs.first())?
            .id;
        let current = self
            .stack_set
            .current_screen()
            .filter(|s| s.output == output);
        let screen =
            current.or_else(|| self.stack_set.screens().iter().find(|s| s.output == output));
        let tag = screen.map_or_else(
            || self.stack_set.current_workspace().tag.as_str(),
            |s| s.workspace.tag.as_str(),
        );
        self.bar_visible_on(tag).then_some(output)
    }

    /// The bar's output and rectangle, if it is shown: a strip along the
    /// configured edge of the output's usable area (like xmobar's strut,
    /// it spans every virtual screen of a split output).
    #[must_use]
    pub fn bar_rect(&self) -> Option<(OutputId, Rect)> {
        let output = self.stack_set.output(self.bar_output()?)?;
        let area = output.usable;
        let height = self.config.bar.height.clamp(0, area.h);
        let rect = match self.config.bar.position {
            BarPosition::Top => Rect::new(area.x, area.y, area.w, height),
            BarPosition::Bottom => Rect::new(area.x, area.y + area.h - height, area.w, height),
        };
        Some((output.id, rect))
    }

    /// The area of `screen` the layout tiles into: the usable rectangle
    /// minus the bar, if the bar is on the screen's output.
    #[must_use]
    pub fn tiling_area(&self, screen: ScreenId) -> Option<Rect> {
        let screen = self.stack_set.screen(screen)?;
        let area = screen.usable_rect;
        let Some((_, bar)) = self
            .bar_rect()
            .filter(|&(output, _)| output == screen.output)
        else {
            return Some(area);
        };
        let (top, bottom) = (area.y, area.y + area.h);
        let (top, bottom) = match self.config.bar.position {
            BarPosition::Top => (top.max(bar.y + bar.h), bottom),
            BarPosition::Bottom => (top, bottom.min(bar.y)),
        };
        Some(Rect::new(area.x, top, area.w, (bottom - top).max(0)))
    }

    /// Computes the complete desired state.
    ///
    /// A window that is on several visible workspaces (a copy) is shown
    /// only once: on the current screen if it is there, else on the first
    /// such screen in screen order.
    #[must_use]
    pub fn arrange(&self) -> Arrangement {
        // The current screen claims its windows first.
        let all = self.stack_set.screens();
        let current = self.stack_set.current_screen().map(|s| s.id);
        let mut order: Vec<usize> = (0..all.len()).collect();
        order.sort_by_key(|&i| (Some(all[i].id) != current, i));

        let mut claimed = BTreeSet::new();
        let mut layers = Layers::default();
        let mut screens = Vec::new();
        for index in order {
            let screen = &all[index];
            let stack = screen
                .workspace
                .stack
                .as_ref()
                .and_then(|s| s.filter(|w| !claimed.contains(w)));
            let tab_strip = stack.and_then(|stack| {
                claimed.extend(stack.iter().copied());
                self.arrange_screen(screen, &stack, &mut layers)
            });
            screens.push((
                index,
                ScreenArrangement {
                    screen: screen.id,
                    output: screen.output,
                    tab_strip,
                },
            ));
        }
        screens.sort_by_key(|&(index, _)| index);
        let screens = screens.into_iter().map(|(_, s)| s).collect();

        let mut windows = layers.tiled;
        windows.append(&mut layers.floating);
        windows.append(&mut layers.fullscreen);
        let placed: BTreeSet<WindowId> = windows.iter().map(|p| p.window).collect();
        let hidden = self
            .windows
            .keys()
            .copied()
            .filter(|w| !placed.contains(w))
            .collect();
        let focus = if self.prompt.is_some() {
            Focus::Prompt
        } else if self.picker.is_some() {
            Focus::LayoutPicker
        } else if self.help.is_some() {
            Focus::Help
        } else {
            self.stack_set.peek().map_or(Focus::None, Focus::Window)
        };
        Arrangement {
            windows,
            hidden,
            focus,
            screens,
            bar: self.bar_rect(),
            prompt: self.prompt_rect(),
            layout_picker: self.layout_picker_area(),
            help: self.help_area(),
        }
    }

    /// Places the windows of `stack` (the part of `screen`'s workspace not
    /// shown elsewhere) into `layers`, returning the screen's tab strip.
    fn arrange_screen(
        &self,
        screen: &Screen,
        stack: &Stack<WindowId>,
        layers: &mut Layers,
    ) -> Option<TabStrip> {
        // Tiled windows, laid out by the workspace's layout.
        let mut tab_strip = None;
        if let Some(tiled) = stack.filter(|&w| self.is_tiled(w)) {
            let windows = tiled.integrate();
            let area = self.tiling_area(screen.id).unwrap_or(screen.usable_rect);
            let layout = &screen.workspace.layout;
            let output = layout.arrange(area, windows.len(), tiled.focus_index());
            let lone = output.rects.len() == 1 && self.lone_tiled_unbordered();
            for (index, slot) in output.rects {
                let window = windows[index];
                let border = self
                    .border(window)
                    .filter(|_| layout.has_borders() && !lone);
                layers.tiled.push(Placement {
                    window,
                    screen: screen.id,
                    output: screen.output,
                    rect: border.map_or(slot, |b| slot.inset(b.width)),
                    layer: Layer::Tiled,
                    border,
                });
            }
            tab_strip = output.tabs.map(|tabs| TabStrip {
                rect: tabs.strip,
                tabs: windows
                    .iter()
                    .zip(tabs.tabs)
                    .map(|(&window, rect)| Tab {
                        window,
                        title: self.title(window),
                        focused: window == tiled.focus,
                        rect,
                    })
                    .collect(),
            });
        }

        // Floating windows: bottom to top is reverse stack order, with the
        // focused window on top.
        let mut floats: Vec<WindowId> = stack
            .iter()
            .rev()
            .copied()
            .filter(|w| self.stack_set.is_floating(*w) && !self.fullscreen.contains(w))
            .collect();
        if let Some(pos) = floats.iter().position(|&w| w == stack.focus) {
            let top = floats.remove(pos);
            floats.push(top);
        }
        for window in floats {
            let Some(rational) = self.stack_set.floating().get(&window) else {
                continue;
            };
            let frame = rational.scale(screen.rect);
            let border = self.border(window);
            let rect = border.map_or(frame, |b| frame.inset(b.width));
            // A float whose window covers its whole screen (a video player)
            // keeps its size but loses the border, like smartBorders.
            let covers_screen = rect.intersection(screen.rect) == screen.rect;
            let border =
                border.filter(|_| !covers_screen || self.config.border.smart == SmartBorders::Off);
            layers.floating.push(Placement {
                window,
                screen: screen.id,
                output: screen.output,
                rect,
                layer: Layer::Floating,
                border,
            });
        }

        let layer = if self.stack_set.is_split(screen.output) {
            Layer::VirtualFullscreen
        } else {
            Layer::Fullscreen
        };
        for &window in stack.iter().filter(|w| self.fullscreen.contains(w)) {
            layers.fullscreen.push(Placement {
                window,
                screen: screen.id,
                output: screen.output,
                rect: screen.rect,
                layer,
                border: None,
            });
        }
        tab_strip
    }

    /// Whether a tiled window that its screen's layout places alone goes
    /// without a border (`[border] smart`). With `single-screen` this needs
    /// a single screen, as xmonad-contrib's `smartBorders` counts every
    /// screen, empty ones included.
    fn lone_tiled_unbordered(&self) -> bool {
        match self.config.border.smart {
            SmartBorders::Off => false,
            SmartBorders::SingleScreen => self.stack_set.screens().len() == 1,
            SmartBorders::PerScreen => true,
        }
    }

    /// The border of `window`: focused or normal color, `None` when the
    /// configured width is zero.
    fn border(&self, window: WindowId) -> Option<Border> {
        let width = self.config.border.width;
        (width > 0).then(|| Border {
            width,
            color: if self.stack_set.peek() == Some(window) {
                self.config.border.focused
            } else {
                self.config.border.normal
            },
        })
    }

    /// The prompt's screen and rectangle while it is open: a strip along
    /// the configured edge of the current screen's usable area.
    fn prompt_rect(&self) -> Option<(ScreenId, Rect)> {
        self.prompt.as_ref()?;
        let screen = self.stack_set.current_screen()?;
        let area = screen.usable_rect;
        let height = self.config.prompt.height.clamp(0, area.h);
        let rect = match self.config.prompt.position {
            BarPosition::Top => Rect::new(area.x, area.y, area.w, height),
            BarPosition::Bottom => Rect::new(area.x, area.y + area.h - height, area.w, height),
        };
        Some((screen.id, rect))
    }

    /// The layout picker's screen and area while it is open: the usable
    /// rectangle of the screen showing the picker's workspace, else of the
    /// current screen.
    fn layout_picker_area(&self) -> Option<(ScreenId, Rect)> {
        let picker = self.picker.as_ref()?;
        let screen = self
            .stack_set
            .screen_of_tag(picker.tag())
            .and_then(|id| self.stack_set.screen(id))
            .or_else(|| self.stack_set.current_screen())?;
        Some((screen.id, screen.usable_rect))
    }

    /// The help overlay's screen and area while it is open: the current
    /// screen's usable rectangle.
    fn help_area(&self) -> Option<(ScreenId, Rect)> {
        self.help.as_ref()?;
        let screen = self.stack_set.current_screen()?;
        Some((screen.id, screen.usable_rect))
    }

    /// The title of a managed window (empty if unknown).
    pub(super) fn title(&self, window: WindowId) -> String {
        self.windows
            .get(&window)
            .map(|info| info.title.clone())
            .unwrap_or_default()
    }
}
