//! Dynamic workspace operations, modelled on
//! `XMonad.Actions.DynamicWorkspaces` and `XMonad.Actions.CopyWindow`:
//! creating workspaces by name, moving/copying windows to named workspaces,
//! renaming and removing workspaces, all driven by the [`crate::prompt`].

use crate::{
    Oxmonad,
    layout::Workspace,
    prompt::{
        Prompt,
        PromptKind,
        PromptOutcome,
    },
};

impl Oxmonad {
    /// Opens a workspace prompt of `kind`, completing over the current
    /// workspace names.
    pub fn open_workspace_prompt(&mut self, kind: PromptKind) {
        let options = self
            .workspaces
            .iter()
            .map(|workspace| workspace.name.clone())
            .collect();
        self.prompt = Some(Prompt::new(kind, options));
    }

    /// Feeds a key press into the open prompt, executing the prompt's action
    /// when it is submitted.
    pub fn handle_prompt_key(
        &mut self,
        keysym: smithay::input::keyboard::Keysym,
        ch: Option<char>,
    ) {
        let Some(mut prompt) = self.prompt.take() else {
            return;
        };
        match prompt.handle_key(keysym, ch) {
            PromptOutcome::Continue => self.prompt = Some(prompt),
            PromptOutcome::Cancel => {}
            PromptOutcome::Submit(value) => self.submit_prompt(prompt.kind, &value),
        }
    }

    /// Executes a submitted prompt value.
    fn submit_prompt(&mut self, kind: PromptKind, value: &str) {
        let name = value.trim();
        if name.is_empty() {
            return;
        }
        match kind {
            PromptKind::Select => {
                let target = self.find_or_create_workspace(name);
                self.view_workspace(target);
            }
            PromptKind::MoveTo => {
                let target = self.find_or_create_workspace(name);
                self.move_focused_to_workspace(target);
            }
            PromptKind::CopyTo => {
                let target = self.find_or_create_workspace(name);
                self.copy_focused_to_workspace(target);
            }
            PromptKind::Rename => self.rename_current_workspace(name.to_string()),
        }
    }

    /// Index of the workspace called `name`, creating an empty one at the end
    /// of the pool when no workspace has that name (`addWorkspace`).
    fn find_or_create_workspace(&mut self, name: &str) -> usize {
        if let Some(index) = self
            .workspaces
            .iter()
            .position(|workspace| workspace.name == name)
        {
            return index;
        }
        self.workspaces.push(Workspace::new(name.to_string()));
        self.workspaces.len() - 1
    }

    /// Copies the focused window to workspace `target`, keeping it on its
    /// current workspace(s) — `XMonad.Actions.CopyWindow.copy`.
    ///
    /// The window becomes the master of `target`. Copying is idempotent: a
    /// window appears at most once per workspace.
    pub fn copy_focused_to_workspace(&mut self, target: usize) {
        let Some(focused) = self.focused_window() else {
            return;
        };
        let Some(workspace) = self.workspaces.get_mut(target) else {
            return;
        };
        if workspace.windows.contains(&focused) {
            return;
        }
        workspace.windows.insert(0, focused);
        self.relayout();
    }

    /// Removes the current workspace (`removeWorkspace`, `mod+shift+⌫`).
    ///
    /// Its windows merge into the most recently hidden workspace (falling
    /// back to the first other workspace), which the active monitor then
    /// shows. Refuses to remove when that would leave fewer workspaces than
    /// monitors.
    pub fn remove_current_workspace(&mut self) {
        let Some(removed) = self.current_workspace() else {
            return;
        };
        if self.workspaces.len() <= self.monitors.len().max(1) {
            return;
        }

        // Prefer merging into the most recently hidden workspace, like
        // xmonad's removeWorkspace returning to the previous workspace.
        let target = self
            .workspace_history
            .iter()
            .rev()
            .copied()
            .find(|&index| index != removed && index < self.workspaces.len())
            .or_else(|| (0..self.workspaces.len()).find(|&index| index != removed));
        let Some(target) = target else {
            return;
        };

        let orphans = std::mem::take(&mut self.workspaces[removed].windows);
        // Removed windows join the target's stack ahead of its master,
        // preserving their order.
        for window in orphans.into_iter().rev() {
            if !self.workspaces[target].windows.contains(&window) {
                self.workspaces[target].windows.insert(0, window);
            }
        }
        self.workspaces.remove(removed);

        // Re-point every index that referenced the removed slot or anything
        // after it.
        let shift = |index: usize| if index > removed { index - 1 } else { index };
        let target = shift(target);
        self.workspace_history.retain(|&index| index != removed);
        for index in &mut self.workspace_history {
            *index = shift(*index);
        }
        for monitor in &mut self.monitors {
            monitor.workspace = shift(monitor.workspace);
        }
        // The active monitor was showing the removed workspace; give it the
        // merge target instead (stealing it if another monitor shows it).
        self.monitors[self.active_monitor].workspace = target;
        for (index, monitor) in self.monitors.iter_mut().enumerate() {
            if index != self.active_monitor && monitor.workspace == target {
                // The other monitor needs a workspace no one else shows.
                monitor.workspace = 0;
            }
        }
        self.ensure_distinct_monitor_workspaces();

        self.relayout();
        self.focus_workspace_master(target);
    }

    /// Reassigns duplicated monitor workspaces so no two monitors show the
    /// same workspace.
    fn ensure_distinct_monitor_workspaces(&mut self) {
        for index in 0..self.monitors.len() {
            let workspace = self.monitors[index].workspace;
            let taken = self.monitors[..index]
                .iter()
                .any(|monitor| monitor.workspace == workspace);
            if !taken {
                continue;
            }
            if let Some(free) = (0..self.workspaces.len()).find(|candidate| {
                !self
                    .monitors
                    .iter()
                    .enumerate()
                    .any(|(i, monitor)| i != index && monitor.workspace == *candidate)
                    && *candidate != workspace
            }) {
                self.monitors[index].workspace = free;
            }
        }
    }
}
