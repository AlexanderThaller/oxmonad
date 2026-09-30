//! The [`Wm`] facade: the only interface the binary uses.
//!
//! `Wm` bundles the [`StackSet`], per-window information, the resolved
//! [`Config`], the bar toggle and the open prompt, layout picker or help
//! overlay. The binary
//! records events through its methods, feeds user intent through [`Wm::apply`]
//! and [`Wm::prompt_key`], executes the returned [`Effect`]s and renders
//! [`Wm::arrange`]'s [`Arrangement`] plus the view models.

mod apply;
mod arrange;
mod snapshot;
mod view;

#[cfg(test)]
mod tests;

use std::collections::{
    BTreeMap,
    BTreeSet,
};

pub use arrange::{
    Arrangement,
    Border,
    Focus,
    Layer,
    Placement,
    ScreenArrangement,
    Tab,
    TabStrip,
};
pub use snapshot::{
    RestoreError,
    ScreenSnapshot,
    Snapshot,
    WorkspaceSnapshot,
};
pub use view::{
    BarModel,
    HelpModel,
    LayoutPickerModel,
    LayoutPreview,
    PromptModel,
    WorkspaceState,
};

use crate::{
    action::Effect,
    config::Config,
    geometry::{
        OutputId,
        Ratio,
        RationalRect,
        Rect,
        Size,
        WindowId,
    },
    help::{
        Help,
        HelpOutcome,
    },
    layout::LayoutRing,
    layout_picker::{
        LayoutPicker,
        PickerOutcome,
    },
    prompt::{
        Prompt,
        PromptKey,
        PromptKind,
        PromptOutcome,
    },
    rules,
    stack_set::StackSet,
};

/// What core knows about a window besides its place in the stack set.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WindowInfo {
    /// The app id (X11 class under Xwayland).
    pub app_id: String,
    /// The window title.
    pub title: String,
    /// Whether the window has a parent (a transient/dialog); such windows
    /// float by default.
    pub has_parent: bool,
    /// The size the client asked for, used when the window floats.
    pub preferred_size: Option<Size>,
}

/// The window manager state; see the [module docs](self).
#[derive(Debug, Clone)]
pub struct Wm {
    stack_set: StackSet,
    windows: BTreeMap<WindowId, WindowInfo>,
    fullscreen: BTreeSet<WindowId>,
    config: Config,
    bar_visible: bool,
    prompt: Option<Prompt>,
    /// The open layout picker; never open together with the prompt.
    picker: Option<LayoutPicker>,
    /// The open key binding help; never open together with the prompt or
    /// the layout picker.
    help: Option<Help>,
    /// A snapshot being restored (see [`snapshot`]).
    restore: Option<snapshot::PendingRestore>,
}

impl Wm {
    /// A window manager with no screens and no windows.
    #[must_use]
    pub fn new(config: Config) -> Self {
        let stack_set = StackSet::new(&config.workspaces, layout_ring(&config));
        Self {
            stack_set,
            windows: BTreeMap::new(),
            fullscreen: BTreeSet::new(),
            bar_visible: config.bar.visible,
            config,
            prompt: None,
            picker: None,
            help: None,
            restore: None,
        }
    }

    /// Applies a reloaded configuration. Existing workspaces and windows
    /// are kept; configured workspaces that do not exist yet are added, new
    /// workspaces and `reset-layout` use the new layout settings, and the
    /// bar visibility is reset to the configured value.
    pub fn reconfigure(&mut self, config: Config) {
        self.stack_set.set_layout_template(layout_ring(&config));
        for tag in &config.workspaces {
            self.stack_set.add_workspace(tag);
        }
        self.bar_visible = config.bar.visible;
        self.config = config;
    }

    /// The active configuration.
    #[must_use]
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// The workspaces and screens.
    #[must_use]
    pub fn stack_set(&self) -> &StackSet {
        &self.stack_set
    }

    /// Whether the bar is currently shown.
    #[must_use]
    pub fn bar_visible(&self) -> bool {
        self.bar_visible
    }

    // ----------------------------------------------------------------
    // Screens

    /// A new output appeared (or an existing one changed name/geometry);
    /// see [`StackSet::add_output`].
    pub fn add_output(&mut self, id: OutputId, name: &str, rect: Rect) {
        self.stack_set.add_output(id, name, rect);
    }

    /// An output disappeared, with all its screens.
    pub fn remove_output(&mut self, id: OutputId) {
        self.stack_set.remove_output(id);
    }

    /// An output's geometry changed (a split output keeps its
    /// proportions).
    pub fn set_output_rect(&mut self, id: OutputId, rect: Rect) {
        self.stack_set.set_output_rect(id, rect);
    }

    /// The area of an output not covered by exclusive layer-shell surfaces
    /// changed.
    pub fn set_usable_rect(&mut self, id: OutputId, rect: Rect) {
        self.stack_set.set_usable_rect(id, rect);
    }

    // ----------------------------------------------------------------
    // Windows

    /// Manages a new window, running the manage rules (xmonad's `manage`).
    ///
    /// The window is inserted above the focus of the current workspace (or
    /// of the rule's workspace). Transients float centred on their screen
    /// unless a rule says otherwise; windows matching a scratchpad's app id
    /// float at the scratchpad's rectangle. Managing a known window just
    /// updates its info.
    pub fn manage(&mut self, id: WindowId, info: WindowInfo) {
        if self.windows.contains_key(&id) {
            self.update_info(id, info);
            return;
        }
        let decision = rules::decide(
            &self.config.rules,
            &info.app_id,
            &info.title,
            info.has_parent,
        );
        let scratchpad = self
            .config
            .scratchpads
            .iter()
            .find(|pad| pad.app_id == info.app_id)
            .map(|pad| pad.rect);

        let target = match &decision.workspace {
            Some(tag) => {
                self.stack_set.add_workspace(tag);
                tag.clone()
            }
            None => self.stack_set.current_tag().to_owned(),
        };
        self.stack_set.insert_up_on(&target, id);

        if let Some(rect) = scratchpad {
            self.stack_set.float(id, rect);
        } else if decision.float {
            let rect = self.centered_float_rect(&target, info.preferred_size);
            self.stack_set.float(id, rect);
        }
        if decision.swap_down
            && let Some(workspace) = self.stack_set.workspace_mut(&target)
            && let Some(stack) = &mut workspace.stack
            && stack.focus == id
        {
            stack.swap_down();
        }
        self.windows.insert(id, info);
    }

    /// Forgets a window (it was closed or unmapped for good).
    pub fn unmanage(&mut self, id: WindowId) {
        self.stack_set.delete(id);
        self.windows.remove(&id);
        self.fullscreen.remove(&id);
    }

    /// Updates a window's app id/title. Manage rules are not re-run.
    pub fn update_info(&mut self, id: WindowId, info: WindowInfo) {
        if let Some(existing) = self.windows.get_mut(&id) {
            *existing = info;
        }
    }

    /// What core knows about a managed window.
    #[must_use]
    pub fn window_info(&self, id: WindowId) -> Option<&WindowInfo> {
        self.windows.get(&id)
    }

    /// Whether `id` is managed.
    #[must_use]
    pub fn is_managed(&self, id: WindowId) -> bool {
        self.windows.contains_key(&id)
    }

    /// Whether `id` is fullscreen.
    #[must_use]
    pub fn is_fullscreen(&self, id: WindowId) -> bool {
        self.fullscreen.contains(&id)
    }

    /// A client asked to enter (`true`) or leave (`false`) fullscreen.
    pub fn request_fullscreen(&mut self, id: WindowId, fullscreen: bool) {
        if !self.windows.contains_key(&id) {
            return;
        }
        if fullscreen {
            self.fullscreen.insert(id);
        } else {
            self.fullscreen.remove(&id);
        }
    }

    /// Focuses a window after a click or pointer focus. A window on another
    /// screen makes that screen current.
    pub fn focus_window(&mut self, id: WindowId) {
        if self.windows.contains_key(&id) {
            self.stack_set.focus_window(id);
        }
    }

    /// Makes the screen containing the point `(x, y)` current, like xmonad
    /// switching screens when the pointer crosses onto another screen's
    /// root window. Does nothing while the prompt, the layout picker or the
    /// help overlay is open, when the point is on no screen, or when that
    /// screen is already current (so it never undoes keyboard focus changes
    /// on the same screen).
    pub fn focus_screen_at(&mut self, x: i32, y: i32) {
        if self.grabs_keyboard() {
            return;
        }
        let target = self
            .stack_set
            .screens()
            .iter()
            .find(|s| s.rect.contains(x, y))
            .map(|s| s.id);
        let current = self.stack_set.current_screen().map(|s| s.id);
        if let Some(id) = target.filter(|&id| Some(id) != current) {
            self.stack_set.focus_screen(id);
        }
    }

    /// Floats a window with its content at `rect` (absolute coordinates),
    /// e.g. after a client-initiated move. If `rect`'s centre lies on a
    /// screen showing another workspace, the window moves to that
    /// workspace (like xmonad's `float`) and is focused there.
    pub fn float_at(&mut self, id: WindowId, rect: Rect) {
        if !self.windows.contains_key(&id) {
            return;
        }
        let frame = rect.outset(self.config.border.width.max(0));
        let (cx, cy) = frame.center();
        let screen = self
            .stack_set
            .screens()
            .iter()
            .find(|s| s.rect.contains(cx, cy))
            .or_else(|| self.stack_set.current_screen())
            .map(|s| (s.rect, s.workspace.tag.clone()));
        let Some((screen_rect, tag)) = screen else {
            return;
        };
        let visible_elsewhere = self
            .stack_set
            .screens()
            .iter()
            .any(|s| s.workspace.contains(id));
        let on_target = self
            .stack_set
            .workspace(&tag)
            .is_some_and(|w| w.contains(id));
        if visible_elsewhere && !on_target {
            self.stack_set.shift_win(&tag, id);
            self.stack_set.focus_window(id);
        }
        self.stack_set
            .float(id, RationalRect::relative_to(frame, screen_rect));
    }

    /// Updates the geometry of an already floating window (during an
    /// interactive move/resize); `rect` is the content rectangle. Ignored
    /// for tiled windows.
    pub fn set_float_rect(&mut self, id: WindowId, rect: Rect) {
        if self.stack_set.is_floating(id) {
            self.float_at(id, rect);
        }
    }

    // ----------------------------------------------------------------
    // Prompt

    /// Whether a prompt is open (keyboard input should go to
    /// [`Self::prompt_key`]).
    #[must_use]
    pub fn is_prompt_open(&self) -> bool {
        self.prompt.is_some()
    }

    /// Whether the prompt, the layout picker or the help overlay is open
    /// and takes the keyboard (keyboard input should go to
    /// [`Self::prompt_key`]).
    #[must_use]
    pub fn grabs_keyboard(&self) -> bool {
        self.prompt.is_some() || self.picker.is_some() || self.help.is_some()
    }

    /// Opens a workspace prompt completing over the workspace tags,
    /// closing the layout picker and the help overlay.
    fn open_prompt(&mut self, kind: PromptKind) {
        let options = self.bar_tags();
        self.picker = None;
        self.help = None;
        self.prompt = Some(Prompt::new(kind, options));
    }

    /// Feeds a key into the open prompt (executing it on `Return`), layout
    /// picker or help overlay.
    pub fn prompt_key(&mut self, key: PromptKey) -> Vec<Effect> {
        if let Some(help) = &mut self.help {
            if help.handle_key(key) == HelpOutcome::Close {
                self.help = None;
            }
            return Vec::new();
        }
        if let Some(picker) = &mut self.picker {
            match picker.handle_key(key) {
                PickerOutcome::Continue => {}
                PickerOutcome::Cancel => self.picker = None,
                PickerOutcome::Apply(index) => self.pick_layout(index),
            }
            return Vec::new();
        }
        let Some(prompt) = &mut self.prompt else {
            return Vec::new();
        };
        let kind = prompt.kind();
        match prompt.handle_key(key) {
            PromptOutcome::Continue => {}
            PromptOutcome::Cancel => self.prompt = None,
            PromptOutcome::Submit(tag) => {
                self.prompt = None;
                self.submit_prompt(kind, &tag);
            }
        }
        Vec::new()
    }

    fn submit_prompt(&mut self, kind: PromptKind, tag: &str) {
        let ss = &mut self.stack_set;
        match kind {
            PromptKind::ViewWorkspace => {
                ss.add_workspace(tag);
                ss.greedy_view(tag);
            }
            PromptKind::MoveToWorkspace => {
                ss.add_workspace(tag);
                ss.shift(tag);
            }
            PromptKind::CopyToWorkspace => {
                ss.add_workspace(tag);
                ss.copy(tag);
            }
            PromptKind::RenameWorkspace => {
                ss.rename_current(tag);
            }
        }
    }

    // ----------------------------------------------------------------
    // Layout picker

    /// Whether the layout picker is open.
    #[must_use]
    pub fn is_layout_picker_open(&self) -> bool {
        self.picker.is_some()
    }

    /// `pick-layout`: closes the picker if it is open, otherwise opens it
    /// for the current workspace (closing the prompt and the help overlay).
    fn toggle_layout_picker(&mut self) {
        if self.picker.take().is_some() {
            return;
        }
        let workspace = self.stack_set.current_workspace();
        let picker = LayoutPicker::new(
            workspace.tag.clone(),
            workspace.layout.len(),
            workspace.layout.index(),
        );
        self.prompt = None;
        self.help = None;
        self.picker = Some(picker);
    }

    /// Highlights the layout at ring index `index` in the open picker
    /// without applying it (the entry under the pointer).
    pub fn hover_layout(&mut self, index: usize) {
        if let Some(picker) = self.picker.as_mut() {
            picker.select(index);
        }
    }

    /// Switches the picker's workspace to the layout at ring index `index`
    /// (e.g. a clicked entry), keeping every layout's state, and closes the
    /// picker. Does nothing while the picker is closed.
    pub fn pick_layout(&mut self, index: usize) {
        let Some(picker) = self.picker.take() else {
            return;
        };
        if let Some(workspace) = self.stack_set.workspace_mut(picker.tag()) {
            workspace.layout.select(index);
        }
    }

    // ----------------------------------------------------------------
    // Help overlay

    /// Whether the key binding help overlay is open.
    #[must_use]
    pub fn is_help_open(&self) -> bool {
        self.help.is_some()
    }

    /// `help`: closes the overlay if it is open, otherwise opens it
    /// scrolled to the top (closing the prompt and the layout picker).
    fn toggle_help(&mut self) {
        if self.help.take().is_some() {
            return;
        }
        self.prompt = None;
        self.picker = None;
        self.help = Some(Help::new());
    }

    /// Closes the help overlay (e.g. after a click on it).
    pub fn close_help(&mut self) {
        self.help = None;
    }

    /// Tells the open help overlay that `page` rows fit on screen and its
    /// content scrolls by at most `max_scroll` rows (the renderer knows the
    /// font metrics); clamps the scroll offset.
    pub fn set_help_viewport(&mut self, page: usize, max_scroll: usize) {
        if let Some(help) = self.help.as_mut() {
            help.set_viewport(page, max_scroll);
        }
    }

    // ----------------------------------------------------------------
    // Helpers

    /// A floating rectangle of `size` (default: half the screen) centred
    /// on the screen showing `tag` (or the current screen).
    fn centered_float_rect(&self, tag: &str, size: Option<Size>) -> RationalRect {
        let half = Ratio::new(1, 2);
        let quarter = Ratio::new(1, 4);
        let default = RationalRect::new(quarter, quarter, half, half);
        let screen = self
            .stack_set
            .screen_of_tag(tag)
            .and_then(|id| self.stack_set.screen(id))
            .or_else(|| self.stack_set.current_screen());
        match (screen, size) {
            (Some(screen), Some(size)) if size.w > 0 && size.h > 0 => {
                let border = self.config.border.width.max(0);
                let frame = Size::new(size.w + 2 * border, size.h + 2 * border);
                RationalRect::relative_to(screen.rect.centered(frame), screen.rect)
            }
            _ => default,
        }
    }
}

/// The layout ring the configuration asks for.
fn layout_ring(config: &Config) -> LayoutRing {
    LayoutRing::standard(config.tabs.height)
}
