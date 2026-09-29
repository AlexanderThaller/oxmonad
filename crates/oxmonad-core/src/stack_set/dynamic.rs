//! Dynamic workspaces (`XMonad.Actions.DynamicWorkspaces`) and workspace
//! history (`XMonad.Actions.CycleWS.toggleWS'`).

use super::{
    SCRATCHPAD_TAG,
    Stack,
    StackSet,
    Workspace,
};
use crate::geometry::WindowId;

impl StackSet {
    /// Adds an empty hidden workspace tagged `tag` unless one exists
    /// (`addHiddenWorkspace`: it goes to the front of the hidden list).
    /// Returns whether a workspace was created.
    pub fn add_workspace(&mut self, tag: &str) -> bool {
        if tag.is_empty() || self.has_tag(tag) {
            return false;
        }
        let workspace = Workspace::new(tag, self.layout_template.clone());
        self.hidden.insert(0, workspace);
        true
    }

    /// Renames the current workspace (`renameWorkspaceByName`).
    ///
    /// If a hidden workspace already carries `tag`, it is removed and its
    /// windows merge into the current one first (xmonad's
    /// `removeWorkspace'`). Renaming to an empty tag, or to the tag of a
    /// workspace visible on another screen, is refused. Returns whether the
    /// rename happened.
    pub fn rename_current(&mut self, tag: &str) -> bool {
        if tag.is_empty() || self.current_tag() == tag || self.screen_of_tag(tag).is_some() {
            return false;
        }
        self.merge_hidden_into_current(tag);
        tag.clone_into(&mut self.current_workspace_mut().tag);
        true
    }

    /// Removes the current workspace (`removeWorkspace`): the most recently
    /// hidden workspace (never the scratchpad one) is viewed on the current
    /// screen and the removed workspace's windows merge into it, taking the
    /// focus. Does nothing if there is no such hidden workspace to fall
    /// back to. Returns whether a workspace was removed.
    ///
    /// Only the current screen changes; workspaces shown on other screens
    /// stay where they are.
    pub fn remove_current_workspace(&mut self) -> bool {
        let Some(target) = self
            .hidden
            .iter()
            .find(|w| w.tag != SCRATCHPAD_TAG)
            .map(|w| w.tag.clone())
        else {
            return false;
        };
        let removed = self.current_tag().to_owned();
        self.view(&target);
        self.merge_hidden_into_current(&removed);
        true
    }

    /// xmonad's `removeWorkspace'`: removes hidden workspace `tag` and
    /// melds its windows (first, taking the focus) with the current
    /// workspace's.
    fn merge_hidden_into_current(&mut self, tag: &str) {
        let Some(index) = self.hidden.iter().position(|w| w.tag == tag) else {
            return;
        };
        let removed = self.hidden.remove(index);
        let current = self.current_workspace_mut();
        let merged = match (removed.stack, current.stack.take()) {
            (None, None) => None,
            (Some(stack), None) | (None, Some(stack)) => Some(stack),
            (Some(first), Some(second)) => {
                let mut windows: Vec<WindowId> = first.integrate();
                for window in second.iter() {
                    if !windows.contains(window) {
                        windows.push(*window);
                    }
                }
                Stack::from_list(windows)
            }
        };
        current.stack = merged;
    }

    /// Views the most recently hidden workspace whose tag is not in `skip`
    /// (`toggleWS'`). The scratchpad workspace is always skipped.
    pub fn toggle_ws(&mut self, skip: &[&str]) {
        if let Some(tag) = self
            .hidden
            .iter()
            .find(|w| w.tag != SCRATCHPAD_TAG && !skip.contains(&w.tag.as_str()))
            .map(|w| w.tag.clone())
        {
            self.view(&tag);
        }
    }

    /// Restores the current workspace's layout ring to the template
    /// (`setLayout (layoutHook conf)`).
    pub fn reset_layout(&mut self) {
        let template = self.layout_template.clone();
        self.current_workspace_mut().layout = template;
    }
}
