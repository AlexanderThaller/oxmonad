//! Computing the [`Arrangement`]: the complete desired window state.

use std::collections::BTreeSet;

use super::Wm;
use crate::{
    config::{
        BarPosition,
        Color,
    },
    geometry::{
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
    /// Fullscreen, covering the whole output above everything else.
    Fullscreen,
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
    /// Where the bar goes, if it is shown on this screen.
    pub bar: Option<Rect>,
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
    /// Per-screen bar and tab strip, in screen-id order.
    pub screens: Vec<ScreenArrangement>,
    /// Where the prompt goes while it is open.
    pub prompt: Option<(ScreenId, Rect)>,
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
    /// The screen the bar is shown on, if it is visible: the screen named
    /// by `bar.output`, else the first screen.
    #[must_use]
    pub fn bar_screen(&self) -> Option<ScreenId> {
        if !self.bar_visible {
            return None;
        }
        let screens = self.stack_set.screens();
        self.config
            .bar
            .output
            .as_ref()
            .and_then(|name| screens.iter().find(|s| &s.name == name))
            .or_else(|| screens.first())
            .map(|s| s.id)
    }

    /// The bar rectangle on `screen`, if the bar is shown there.
    fn bar_rect(&self, screen: &Screen) -> Option<Rect> {
        if self.bar_screen() != Some(screen.id) {
            return None;
        }
        let area = screen.usable_rect;
        let height = self.config.bar.height.clamp(0, area.h);
        Some(match self.config.bar.position {
            BarPosition::Top => Rect::new(area.x, area.y, area.w, height),
            BarPosition::Bottom => Rect::new(area.x, area.y + area.h - height, area.w, height),
        })
    }

    /// The area of `screen` the layout tiles into: the usable rectangle
    /// minus the bar.
    #[must_use]
    pub fn tiling_area(&self, screen: ScreenId) -> Option<Rect> {
        let screen = self.stack_set.screen(screen)?;
        let area = screen.usable_rect;
        Some(match (self.bar_rect(screen), self.config.bar.position) {
            (None, _) => area,
            (Some(bar), BarPosition::Top) => {
                Rect::new(area.x, area.y + bar.h, area.w, area.h - bar.h)
            }
            (Some(bar), BarPosition::Bottom) => Rect::new(area.x, area.y, area.w, area.h - bar.h),
        })
    }

    /// Computes the complete desired state.
    ///
    /// A window that is on several visible workspaces (a copy) is shown
    /// only once: on the current screen if it is there, else on the first
    /// such screen in screen-id order.
    #[must_use]
    pub fn arrange(&self) -> Arrangement {
        // The current screen claims its windows first.
        let mut order: Vec<&Screen> = self.stack_set.screens().iter().collect();
        let current = self.stack_set.current_screen().map(|s| s.id);
        order.sort_by_key(|s| (Some(s.id) != current, s.id));

        let mut claimed = BTreeSet::new();
        let mut layers = Layers::default();
        let mut screens = Vec::new();
        for screen in order {
            let stack = screen
                .workspace
                .stack
                .as_ref()
                .and_then(|s| s.filter(|w| !claimed.contains(w)));
            let tab_strip = stack.and_then(|stack| {
                claimed.extend(stack.iter().copied());
                self.arrange_screen(screen, &stack, &mut layers)
            });
            screens.push(ScreenArrangement {
                screen: screen.id,
                bar: self.bar_rect(screen),
                tab_strip,
            });
        }
        screens.sort_by_key(|s| s.screen);

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
        } else {
            self.stack_set.peek().map_or(Focus::None, Focus::Window)
        };
        Arrangement {
            windows,
            hidden,
            focus,
            screens,
            prompt: self.prompt_rect(),
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
            for (index, slot) in output.rects {
                let window = windows[index];
                let border = self.border(window).filter(|_| layout.has_borders());
                layers.tiled.push(Placement {
                    window,
                    screen: screen.id,
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
            layers.floating.push(Placement {
                window,
                screen: screen.id,
                rect: border.map_or(frame, |b| frame.inset(b.width)),
                layer: Layer::Floating,
                border,
            });
        }

        for &window in stack.iter().filter(|w| self.fullscreen.contains(w)) {
            layers.fullscreen.push(Placement {
                window,
                screen: screen.id,
                rect: screen.rect,
                layer: Layer::Fullscreen,
                border: None,
            });
        }
        tab_strip
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

    /// The title of a managed window (empty if unknown).
    pub(super) fn title(&self, window: WindowId) -> String {
        self.windows
            .get(&window)
            .map(|info| info.title.clone())
            .unwrap_or_default()
    }
}
