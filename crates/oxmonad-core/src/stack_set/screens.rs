//! Screen management and viewing workspaces.

use super::{
    PLACEHOLDER,
    SCRATCHPAD_TAG,
    Screen,
    StackSet,
    Workspace,
};
use crate::{
    action::Direction,
    geometry::{
        Rect,
        ScreenId,
    },
};

impl StackSet {
    /// xmonad's `view`: shows workspace `tag`. A workspace visible on
    /// another screen makes that screen current; a hidden one replaces the
    /// current screen's workspace, which becomes the most recently hidden.
    pub fn view(&mut self, tag: &str) {
        if self.current_tag() == tag {
            return;
        }
        if let Some(index) = self.screens.iter().position(|s| s.workspace.tag == tag) {
            self.current = index;
        } else if let Some(index) = self.hidden.iter().position(|w| w.tag == tag) {
            let incoming = self.hidden.remove(index);
            let outgoing = std::mem::replace(self.current_workspace_mut(), incoming);
            self.hidden.insert(0, outgoing);
        }
    }

    /// xmonad's `greedyView`: like [`Self::view`], but a workspace visible
    /// on another screen is swapped onto the current screen instead of
    /// focusing that screen.
    pub fn greedy_view(&mut self, tag: &str) {
        if let Some(index) = self.screens.iter().position(|s| s.workspace.tag == tag)
            && index != self.current
        {
            let (a, b) = pair_mut(&mut self.screens, self.current, index);
            std::mem::swap(&mut a.workspace, &mut b.workspace);
        } else {
            self.view(tag);
        }
    }

    /// Adds a screen, or updates its name and rectangle if `id` is known.
    ///
    /// A new screen shows the most recently hidden workspace (never the
    /// scratchpad workspace); if none is hidden, a fresh workspace is
    /// created, tagged with the smallest positive integer not yet in use.
    /// The first screen after being headless keeps the current workspace
    /// and becomes current.
    pub fn add_screen(&mut self, id: ScreenId, name: &str, rect: Rect) {
        if let Some(screen) = self.screens().iter().position(|s| s.id == id) {
            let screen = &mut self.screens[screen];
            name.clone_into(&mut screen.name);
            screen.rect = rect;
            screen.usable_rect = rect;
            return;
        }
        if self.headless {
            let placeholder = &mut self.screens[0];
            placeholder.id = id;
            name.clone_into(&mut placeholder.name);
            placeholder.rect = rect;
            placeholder.usable_rect = rect;
            self.headless = false;
            self.current = 0;
            return;
        }
        let workspace = match self.hidden.iter().position(|w| w.tag != SCRATCHPAD_TAG) {
            Some(index) => self.hidden.remove(index),
            None => Workspace::new(self.fresh_tag(), self.layout_template.clone()),
        };
        let current_id = self.screens[self.current].id;
        self.screens.push(Screen {
            id,
            name: name.to_owned(),
            workspace,
            rect,
            usable_rect: rect,
        });
        self.screens.sort_by_key(|s| s.id);
        self.current = self.index_of(current_id).unwrap_or(0);
    }

    /// Removes a screen. Its workspace becomes the most recently hidden
    /// one; if it was current, the first screen in physical order becomes
    /// current. Removing the last screen makes the stack set headless,
    /// keeping its workspace current.
    pub fn remove_screen(&mut self, id: ScreenId) {
        let Some(index) = self.screens().iter().position(|s| s.id == id) else {
            return;
        };
        if self.screens.len() == 1 {
            let placeholder = &mut self.screens[0];
            placeholder.id = PLACEHOLDER;
            placeholder.name.clear();
            placeholder.rect = Rect::default();
            placeholder.usable_rect = Rect::default();
            self.headless = true;
            return;
        }
        let was_current = index == self.current;
        let current_id = self.screens[self.current].id;
        let removed = self.screens.remove(index);
        self.hidden.insert(0, removed.workspace);
        self.current = if was_current {
            let first = self.physical_order()[0];
            self.index_of(first).unwrap_or(0)
        } else {
            self.index_of(current_id).unwrap_or(0)
        };
    }

    /// Updates a screen's output rectangle (also resetting its usable
    /// rectangle to the full output).
    pub fn set_screen_rect(&mut self, id: ScreenId, rect: Rect) {
        if let Some(screen) = self.screen_mut(id) {
            screen.rect = rect;
            screen.usable_rect = rect;
        }
    }

    /// Updates the area of a screen available for tiling.
    pub fn set_usable_rect(&mut self, id: ScreenId, rect: Rect) {
        if let Some(screen) = self.screen_mut(id) {
            screen.usable_rect = rect;
        }
    }

    fn screen_mut(&mut self, id: ScreenId) -> Option<&mut Screen> {
        if self.headless {
            return None;
        }
        self.screens.iter_mut().find(|s| s.id == id)
    }

    fn index_of(&self, id: ScreenId) -> Option<usize> {
        self.screens.iter().position(|s| s.id == id)
    }

    /// Makes screen `id` current.
    pub fn focus_screen(&mut self, id: ScreenId) {
        if let Some(index) = self.screens().iter().position(|s| s.id == id) {
            self.current = index;
        }
    }

    /// Screen ids in physical order: top-to-bottom, then left-to-right
    /// (`XMonad.Actions.PhysicalScreens`' default comparator).
    #[must_use]
    pub fn physical_order(&self) -> Vec<ScreenId> {
        let mut screens: Vec<&Screen> = self.screens().iter().collect();
        screens.sort_by_key(|s| (s.rect.y, s.rect.x, s.id));
        screens.into_iter().map(|s| s.id).collect()
    }

    /// The physical neighbour of the current screen in `direction`,
    /// wrapping (`getNeighbour`).
    #[must_use]
    pub fn neighbour(&self, direction: Direction) -> Option<ScreenId> {
        let order = self.physical_order();
        let current = self.current_screen()?.id;
        let position = order.iter().position(|&id| id == current).unwrap_or(0);
        let next = match direction {
            Direction::Next => (position + 1) % order.len(),
            Direction::Prev => (position + order.len() - 1) % order.len(),
        };
        order.get(next).copied()
    }

    /// Swaps the current screen's workspace with the next screen's in
    /// screen-id order (`swapNextScreen`, i.e. `greedyView` of the next
    /// screen's workspace).
    pub fn swap_next_screen(&mut self) {
        if self.screens().len() < 2 {
            return;
        }
        let next = (self.current + 1) % self.screens.len();
        let tag = self.screens[next].workspace.tag.clone();
        self.greedy_view(&tag);
    }

    /// The smallest positive integer not used as a tag.
    pub(super) fn fresh_tag(&self) -> String {
        let mut n: u64 = 1;
        while self.has_tag(&n.to_string()) {
            n += 1;
        }
        n.to_string()
    }
}

/// Two distinct mutable elements of a slice.
fn pair_mut<T>(items: &mut [T], a: usize, b: usize) -> (&mut T, &mut T) {
    assert_ne!(a, b, "pair_mut needs distinct indices");
    if a < b {
        let (left, right) = items.split_at_mut(b);
        (&mut left[a], &mut right[0])
    } else {
        let (left, right) = items.split_at_mut(a);
        (&mut right[0], &mut left[b])
    }
}
