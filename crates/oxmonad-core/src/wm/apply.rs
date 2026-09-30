//! Executing [`Action`]s.

use super::Wm;
use crate::{
    action::{
        Action,
        Direction,
        Effect,
    },
    geometry::WindowId,
    layout::{
        LayoutContext,
        LayoutMessage,
    },
    stack_set::SCRATCHPAD_TAG,
};

impl Wm {
    /// Applies a user action, returning the effects the binary must
    /// perform.
    pub fn apply(&mut self, action: Action) -> Vec<Effect> {
        let ss = &mut self.stack_set;
        match action {
            Action::Spawn(command) => return vec![Effect::Spawn(command)],
            Action::SpawnTerminal => return vec![Effect::Spawn(self.config.terminal.clone())],
            Action::FocusNext => ss.focus_down(),
            Action::FocusPrev => ss.focus_up(),
            Action::FocusMaster => ss.focus_master(),
            Action::SwapNext => ss.swap_down(),
            Action::SwapPrev => ss.swap_up(),
            Action::SwapMaster => ss.swap_master(),
            Action::Sink => {
                if let Some(window) = ss.peek() {
                    ss.sink(window);
                }
            }
            Action::Close => return ss.peek().map(Effect::Close).into_iter().collect(),
            Action::NextLayout => ss.current_workspace_mut().layout.next(),
            Action::ResetLayout => ss.reset_layout(),
            Action::Shrink => self.send_message(LayoutMessage::Shrink),
            Action::Expand => self.send_message(LayoutMessage::Expand),
            Action::MirrorShrink => self.send_message(LayoutMessage::MirrorShrink),
            Action::MirrorExpand => self.send_message(LayoutMessage::MirrorExpand),
            Action::IncMaster(n) => self.send_message(LayoutMessage::IncMasterN(n)),
            Action::View(tag) => ss.greedy_view(&tag),
            Action::Shift(tag) => ss.shift(&tag),
            Action::Copy(tag) => ss.copy(&tag),
            Action::ToggleWs => ss.toggle_ws(&[SCRATCHPAD_TAG]),
            Action::Prompt(kind) => self.open_prompt(kind),
            Action::RemoveWorkspace => {
                ss.remove_current_workspace();
            }
            Action::FocusScreen(direction) => {
                if let Some(id) = ss.neighbour(direction) {
                    ss.focus_screen(id);
                }
            }
            Action::ShiftScreen(direction) => self.shift_to_screen(direction, false),
            Action::ShiftScreenFollow(direction) => self.shift_to_screen(direction, true),
            Action::SwapNextScreen => ss.swap_next_screen(),
            Action::SplitScreen(n) => ss.split_screen(n),
            Action::Rescreen => ss.rescreen(),
            Action::ToggleBar => self.bar_visible = !self.bar_visible,
            Action::Refresh => {}
            Action::Quit => return vec![Effect::Quit],
            Action::Reload => return vec![Effect::Reload],
            Action::Restart => return vec![Effect::Restart],
            Action::ToggleFullscreen => {
                if let Some(window) = ss.peek() {
                    let on = !self.fullscreen.contains(&window);
                    self.request_fullscreen(window, on);
                }
            }
            Action::ToggleScratchpad(name) => return self.toggle_scratchpad(&name),
            Action::FloatMove => {
                return self
                    .start_pointer_op()
                    .map(Effect::StartPointerMove)
                    .into_iter()
                    .collect();
            }
            Action::FloatResize => {
                return self
                    .start_pointer_op()
                    .map(Effect::StartPointerResize)
                    .into_iter()
                    .collect();
            }
        }
        Vec::new()
    }

    /// The tiled-stack context of the current workspace for layout
    /// messages.
    pub(super) fn layout_context(&self) -> LayoutContext {
        let workspace = self.stack_set.current_workspace();
        let Some(tiled) = workspace
            .stack
            .as_ref()
            .and_then(|stack| stack.filter(|&w| self.is_tiled(w)))
        else {
            return LayoutContext::default();
        };
        let focus_is_tiled = workspace.focus().is_some_and(|w| self.is_tiled(w));
        LayoutContext {
            focus: focus_is_tiled.then(|| tiled.focus_index()),
            count: tiled.len(),
        }
    }

    /// Whether `window` takes part in the tiling layout.
    pub(super) fn is_tiled(&self, window: WindowId) -> bool {
        !self.stack_set.is_floating(window) && !self.fullscreen.contains(&window)
    }

    fn send_message(&mut self, message: LayoutMessage) {
        let context = self.layout_context();
        self.stack_set
            .current_workspace_mut()
            .layout
            .handle(message, context);
    }

    /// `onNextNeighbour def W.shift` (and optionally follow the window).
    fn shift_to_screen(&mut self, direction: Direction, follow: bool) {
        let ss = &mut self.stack_set;
        let Some(id) = ss.neighbour(direction) else {
            return;
        };
        let Some(tag) = ss.screen(id).map(|s| s.workspace.tag.clone()) else {
            return;
        };
        ss.shift(&tag);
        if follow {
            ss.focus_screen(id);
        }
    }

    /// Floats the focused window where it currently is and raises it to
    /// the top of the stack (`mouseMoveWindow` + `shiftMaster`), returning
    /// it. Fullscreen windows are left alone.
    fn start_pointer_op(&mut self) -> Option<WindowId> {
        let window = self.stack_set.peek()?;
        if self.fullscreen.contains(&window) {
            return None;
        }
        if !self.stack_set.is_floating(window) {
            let placement = self
                .arrange()
                .windows
                .into_iter()
                .find(|p| p.window == window)?;
            self.float_at(window, placement.rect);
        }
        self.stack_set.shift_master();
        Some(window)
    }

    /// xmonad-contrib's `namedScratchpadAction`: hides the scratchpad if it
    /// is on the current workspace, summons it from wherever it is
    /// otherwise, or spawns it.
    fn toggle_scratchpad(&mut self, name: &str) -> Vec<Effect> {
        let Some(pad) = self
            .config
            .scratchpads
            .iter()
            .find(|pad| pad.name == name)
            .cloned()
        else {
            return Vec::new();
        };
        let matches = |wm: &Self, window: &WindowId| {
            wm.windows
                .get(window)
                .is_some_and(|info| info.app_id == pad.app_id)
        };
        let on_current = self
            .stack_set
            .current_workspace()
            .windows()
            .into_iter()
            .find(|w| matches(self, w));
        if let Some(window) = on_current {
            self.stack_set.add_workspace(SCRATCHPAD_TAG);
            self.stack_set.shift_win(SCRATCHPAD_TAG, window);
            return Vec::new();
        }
        let elsewhere = self
            .stack_set
            .all_windows()
            .into_iter()
            .find(|w| matches(self, w));
        match elsewhere {
            Some(window) => {
                let tag = self.stack_set.current_tag().to_owned();
                self.stack_set.shift_win(&tag, window);
                if !self.stack_set.is_floating(window) {
                    self.stack_set.float(window, pad.rect);
                }
                self.stack_set.focus_window(window);
                Vec::new()
            }
            None => vec![Effect::Spawn(pad.command)],
        }
    }
}
