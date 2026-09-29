//! xmonad's `StackSet`: workspaces, the screens showing them, and the
//! floating layer.
//!
//! The structure follows `XMonad.StackSet` closely:
//!
//! * every [`Screen`] shows exactly one [`Workspace`]; one screen is the
//!   *current* one, the others are *visible*;
//! * the remaining workspaces are *hidden*, most recently hidden first (this
//!   order drives `toggle-ws` and which workspace a new screen gets);
//! * the floating map stores [`RationalRect`]s relative to the screen.
//!
//! One deviation: xmonad always has at least one screen. Before river has
//! announced any output (or after the last one disappeared) the stack set is
//! *headless*: a placeholder screen with an empty rectangle keeps the
//! current workspace, so every operation behaves uniformly. The placeholder
//! is never reported by [`StackSet::screens`].

mod dynamic;
mod screens;
mod stack;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

pub use stack::Stack;

use crate::{
    geometry::{
        RationalRect,
        Rect,
        ScreenId,
        WindowId,
    },
    layout::LayoutRing,
};

/// Tag of the hidden workspace named scratchpads live on
/// (`XMonad.Util.NamedScratchpad`'s `"NSP"`). It is excluded from the bar,
/// from `toggle-ws`, from prompt completion and from screen assignment.
pub const SCRATCHPAD_TAG: &str = "NSP";

/// A virtual desktop: a tag, a layout ring and an optional window stack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    /// The workspace name.
    pub tag: String,
    /// The layouts this workspace cycles through, each with its own state.
    pub layout: LayoutRing,
    /// The windows, or `None` when the workspace is empty.
    pub stack: Option<Stack<WindowId>>,
}

impl Workspace {
    /// An empty workspace.
    #[must_use]
    pub fn new(tag: impl Into<String>, layout: LayoutRing) -> Self {
        Self {
            tag: tag.into(),
            layout,
            stack: None,
        }
    }

    /// The windows in stack order (master first).
    #[must_use]
    pub fn windows(&self) -> Vec<WindowId> {
        self.stack
            .as_ref()
            .map(Stack::integrate)
            .unwrap_or_default()
    }

    /// Whether the workspace holds `window`.
    #[must_use]
    pub fn contains(&self, window: WindowId) -> bool {
        self.stack.as_ref().is_some_and(|s| s.contains(&window))
    }

    /// The focused window.
    #[must_use]
    pub fn focus(&self) -> Option<WindowId> {
        self.stack.as_ref().map(|s| s.focus)
    }

    /// Whether the workspace has no windows.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.stack.is_none()
    }

    /// xmonad's `insertUp` on this workspace: `window` becomes the focus,
    /// above the old focus. No-op if it is already here.
    pub fn insert_up(&mut self, window: WindowId) {
        match &mut self.stack {
            Some(stack) if stack.contains(&window) => {}
            Some(stack) => stack.insert_up(window),
            None => self.stack = Some(Stack::singleton(window)),
        }
    }

    /// Removes `window` from the workspace (xmonad's `filter (/= w)`).
    pub fn remove(&mut self, window: WindowId) {
        self.stack = self.stack.take().and_then(|s| s.filter(|&w| w != window));
    }
}

/// A physical screen (river output) and the workspace it shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Screen {
    /// The screen's identifier.
    pub id: ScreenId,
    /// The output name (e.g. `DP-1`), used to place the bar.
    pub name: String,
    /// The workspace shown on this screen.
    pub workspace: Workspace,
    /// The full output rectangle (fullscreen windows and floats use it).
    pub rect: Rect,
    /// The area left for tiling after layer-shell exclusive zones (the bar
    /// is subtracted separately by `Wm`).
    pub usable_rect: Rect,
}

/// Identifier of the headless placeholder screen.
const PLACEHOLDER: ScreenId = ScreenId(u64::MAX);

/// Workspaces, screens and floating windows; see the [module docs](self).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackSet {
    /// Screens sorted by id (xmonad's screen numbering order).
    screens: Vec<Screen>,
    /// Index into `screens` of the current screen.
    current: usize,
    /// Hidden workspaces, most recently hidden first.
    hidden: Vec<Workspace>,
    /// Floating windows and their screen-relative geometry.
    floating: BTreeMap<WindowId, RationalRect>,
    /// Whether `screens` holds only the placeholder.
    headless: bool,
    /// Layout ring given to newly created workspaces.
    layout_template: LayoutRing,
}

impl StackSet {
    /// A headless stack set with workspaces `tags` (the first one current)
    /// each using `layout`. An empty `tags` list yields a single `"1"`.
    #[must_use]
    pub fn new(tags: &[String], layout: LayoutRing) -> Self {
        let mut workspaces: Vec<Workspace> = tags
            .iter()
            .map(|tag| Workspace::new(tag.clone(), layout.clone()))
            .collect();
        if workspaces.is_empty() {
            workspaces.push(Workspace::new("1", layout.clone()));
        }
        let first = workspaces.remove(0);
        Self {
            screens: vec![Screen {
                id: PLACEHOLDER,
                name: String::new(),
                workspace: first,
                rect: Rect::default(),
                usable_rect: Rect::default(),
            }],
            current: 0,
            hidden: workspaces,
            floating: BTreeMap::new(),
            headless: true,
            layout_template: layout,
        }
    }

    /// The layout ring new workspaces start with.
    #[must_use]
    pub fn layout_template(&self) -> &LayoutRing {
        &self.layout_template
    }

    /// Replaces the layout ring new workspaces (and `reset-layout`) use.
    pub fn set_layout_template(&mut self, layout: LayoutRing) {
        self.layout_template = layout;
    }

    // ----------------------------------------------------------------
    // Queries

    /// Whether no real screen exists.
    #[must_use]
    pub fn is_headless(&self) -> bool {
        self.headless
    }

    /// The real screens, sorted by id.
    #[must_use]
    pub fn screens(&self) -> &[Screen] {
        if self.headless { &[] } else { &self.screens }
    }

    /// The screen with `id`.
    #[must_use]
    pub fn screen(&self, id: ScreenId) -> Option<&Screen> {
        self.screens().iter().find(|s| s.id == id)
    }

    /// The current screen, `None` while headless.
    #[must_use]
    pub fn current_screen(&self) -> Option<&Screen> {
        if self.headless {
            None
        } else {
            self.screens.get(self.current)
        }
    }

    /// The workspace on the current screen.
    #[must_use]
    pub fn current_workspace(&self) -> &Workspace {
        &self.screens[self.current].workspace
    }

    /// The workspace on the current screen, mutably.
    pub fn current_workspace_mut(&mut self) -> &mut Workspace {
        &mut self.screens[self.current].workspace
    }

    /// Tag of the current workspace.
    #[must_use]
    pub fn current_tag(&self) -> &str {
        &self.current_workspace().tag
    }

    /// The hidden workspaces, most recently hidden first.
    #[must_use]
    pub fn hidden(&self) -> &[Workspace] {
        &self.hidden
    }

    /// Every workspace: current, then the other screens', then hidden
    /// (xmonad's `workspaces` order).
    pub fn workspaces(&self) -> impl Iterator<Item = &Workspace> {
        let current = &self.screens[self.current].workspace;
        std::iter::once(current)
            .chain(
                self.screens
                    .iter()
                    .enumerate()
                    .filter(move |(i, _)| *i != self.current)
                    .map(|(_, s)| &s.workspace),
            )
            .chain(self.hidden.iter())
    }

    fn workspaces_mut(&mut self) -> impl Iterator<Item = &mut Workspace> {
        self.screens
            .iter_mut()
            .map(|s| &mut s.workspace)
            .chain(self.hidden.iter_mut())
    }

    /// The workspace tagged `tag`.
    #[must_use]
    pub fn workspace(&self, tag: &str) -> Option<&Workspace> {
        self.workspaces().find(|w| w.tag == tag)
    }

    /// The workspace tagged `tag`, mutably.
    pub fn workspace_mut(&mut self, tag: &str) -> Option<&mut Workspace> {
        self.workspaces_mut().find(|w| w.tag == tag)
    }

    /// Whether a workspace tagged `tag` exists.
    #[must_use]
    pub fn has_tag(&self, tag: &str) -> bool {
        self.workspace(tag).is_some()
    }

    /// The screen showing workspace `tag`, if it is visible.
    #[must_use]
    pub fn screen_of_tag(&self, tag: &str) -> Option<ScreenId> {
        self.screens()
            .iter()
            .find(|s| s.workspace.tag == tag)
            .map(|s| s.id)
    }

    /// The focused window of the current workspace (xmonad's `peek`).
    #[must_use]
    pub fn peek(&self) -> Option<WindowId> {
        self.current_workspace().focus()
    }

    /// Whether `window` is on any workspace.
    #[must_use]
    pub fn member(&self, window: WindowId) -> bool {
        self.workspaces().any(|w| w.contains(window))
    }

    /// Tag of the first workspace holding `window` (xmonad's `findTag`).
    #[must_use]
    pub fn find_tag(&self, window: WindowId) -> Option<&str> {
        self.workspaces()
            .find(|w| w.contains(window))
            .map(|w| w.tag.as_str())
    }

    /// Every managed window, each once, in workspace order.
    #[must_use]
    pub fn all_windows(&self) -> Vec<WindowId> {
        let mut out = Vec::new();
        for window in self.workspaces().flat_map(Workspace::windows) {
            if !out.contains(&window) {
                out.push(window);
            }
        }
        out
    }

    // ----------------------------------------------------------------
    // Floating layer

    /// The floating windows and their screen-relative geometry.
    #[must_use]
    pub fn floating(&self) -> &BTreeMap<WindowId, RationalRect> {
        &self.floating
    }

    /// Whether `window` floats.
    #[must_use]
    pub fn is_floating(&self, window: WindowId) -> bool {
        self.floating.contains_key(&window)
    }

    /// Floats `window` at `rect` (xmonad's `float`).
    pub fn float(&mut self, window: WindowId, rect: RationalRect) {
        self.floating.insert(window, rect);
    }

    /// Returns `window` to the tiling (xmonad's `sink`).
    pub fn sink(&mut self, window: WindowId) {
        self.floating.remove(&window);
    }

    // ----------------------------------------------------------------
    // Stack operations on the current workspace

    fn modify_stack(&mut self, f: impl FnOnce(&mut Stack<WindowId>)) {
        if let Some(stack) = &mut self.current_workspace_mut().stack {
            f(stack);
        }
    }

    /// Focus the previous window, wrapping (`focusUp`).
    pub fn focus_up(&mut self) {
        self.modify_stack(Stack::focus_up);
    }

    /// Focus the next window, wrapping (`focusDown`).
    pub fn focus_down(&mut self) {
        self.modify_stack(Stack::focus_down);
    }

    /// Focus the master window (`focusMaster`).
    pub fn focus_master(&mut self) {
        self.modify_stack(Stack::focus_master);
    }

    /// Swap the focused window with the previous one (`swapUp`).
    pub fn swap_up(&mut self) {
        self.modify_stack(Stack::swap_up);
    }

    /// Swap the focused window with the next one (`swapDown`).
    pub fn swap_down(&mut self) {
        self.modify_stack(Stack::swap_down);
    }

    /// Swap the focused window with the master (`swapMaster`).
    pub fn swap_master(&mut self) {
        self.modify_stack(Stack::swap_master);
    }

    /// Move the focused window to the top of the stack (`shiftMaster`).
    pub fn shift_master(&mut self) {
        self.modify_stack(Stack::shift_master);
    }

    /// Inserts a new window above the focus of the current workspace
    /// (xmonad's `insertUp`). No-op if the window is already managed.
    pub fn insert_up(&mut self, window: WindowId) {
        if !self.member(window) {
            self.current_workspace_mut().insert_up(window);
        }
    }

    /// Removes `window` from every workspace and the floating layer
    /// (xmonad's `delete`).
    pub fn delete(&mut self, window: WindowId) {
        for workspace in self.workspaces_mut() {
            workspace.remove(window);
        }
        self.sink(window);
    }

    // ----------------------------------------------------------------
    // Moving windows between workspaces

    /// Moves the focused window to workspace `tag` (xmonad's `shift`).
    pub fn shift(&mut self, tag: &str) {
        if let Some(window) = self.peek() {
            let from = self.current_tag().to_owned();
            self.shift_from(&from, tag, window);
        }
    }

    /// Moves `window` from the first workspace holding it to `tag`, where
    /// it becomes the focus (xmonad's `shiftWin`).
    ///
    /// Unlike xmonad, a window that has copies elsewhere is still inserted
    /// into `tag` (xmonad's `insertUp` would silently drop it).
    pub fn shift_win(&mut self, tag: &str, window: WindowId) {
        if let Some(from) = self.find_tag(window).map(str::to_owned) {
            self.shift_from(&from, tag, window);
        }
    }

    fn shift_from(&mut self, from: &str, to: &str, window: WindowId) {
        if from == to || !self.has_tag(to) {
            return;
        }
        if let Some(ws) = self.workspace_mut(from) {
            ws.remove(window);
        }
        if let Some(ws) = self.workspace_mut(to) {
            ws.insert_up(window);
        }
    }

    /// Copies the focused window to workspace `tag`, where it becomes the
    /// focus (`XMonad.Actions.CopyWindow.copy`). No-op if it is already
    /// there.
    pub fn copy(&mut self, tag: &str) {
        if let Some(window) = self.peek()
            && let Some(ws) = self.workspace_mut(tag)
        {
            ws.insert_up(window);
        }
    }

    /// Inserts `window` into workspace `tag` above its focus.
    pub fn insert_up_on(&mut self, tag: &str, window: WindowId) {
        if let Some(ws) = self.workspace_mut(tag) {
            ws.insert_up(window);
        }
    }

    /// Focuses `window` (xmonad's `focusWindow`): on the current workspace
    /// it just takes the focus; on a workspace visible on another screen
    /// that screen becomes current; on a hidden workspace that workspace is
    /// viewed on the current screen.
    pub fn focus_window(&mut self, window: WindowId) {
        if self.current_workspace().contains(window) {
            self.modify_stack(|s| {
                s.focus_on(&window);
            });
            return;
        }
        if let Some(index) = self
            .screens()
            .iter()
            .position(|s| s.workspace.contains(window))
        {
            self.current = index;
        } else if let Some(tag) = self.find_tag(window).map(str::to_owned) {
            self.view(&tag);
        } else {
            return;
        }
        self.modify_stack(|s| {
            s.focus_on(&window);
        });
    }
}
