//! Screen management and viewing workspaces.

use super::{
    Output,
    PLACEHOLDER,
    PLACEHOLDER_OUTPUT,
    SCRATCHPAD_TAG,
    Screen,
    StackSet,
    Workspace,
};
use crate::{
    action::Direction,
    geometry::{
        OutputId,
        Ratio,
        RationalRect,
        Rect,
        ScreenId,
        region_rect,
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

    /// The outputs, sorted by id (none while headless).
    #[must_use]
    pub fn outputs(&self) -> &[Output] {
        &self.outputs
    }

    /// The output with `id`.
    #[must_use]
    pub fn output(&self, id: OutputId) -> Option<&Output> {
        self.outputs.iter().find(|o| o.id == id)
    }

    /// Whether output `id` is split into several virtual screens.
    #[must_use]
    pub fn is_split(&self, id: OutputId) -> bool {
        self.screens().iter().filter(|s| s.output == id).count() > 1
    }

    /// Adds an output with one screen covering it, or updates its name and
    /// rectangle if `id` is known (a split output keeps its split, with the
    /// same proportions).
    ///
    /// The new screen is `ScreenId(id.0)` and shows the most recently
    /// hidden workspace (never the scratchpad workspace); if none is
    /// hidden, a fresh workspace is created, tagged with the smallest
    /// positive integer not yet in use. The first output after being
    /// headless keeps the current workspace and becomes current.
    pub fn add_output(&mut self, id: OutputId, name: &str, rect: Rect) {
        if let Some(output) = self.outputs.iter_mut().find(|o| o.id == id) {
            name.clone_into(&mut output.name);
            output.rect = rect;
            output.usable = rect;
            self.refresh_output(id);
            return;
        }
        self.outputs.push(Output {
            id,
            name: name.to_owned(),
            rect,
            usable: rect,
        });
        self.outputs.sort_by_key(|o| o.id);
        let screen = self.fresh_screen_id(ScreenId(id.0));
        if self.headless {
            let placeholder = &mut self.screens[0];
            placeholder.id = screen;
            placeholder.output = id;
            placeholder.region = RationalRect::FULL;
            self.headless = false;
            self.current = 0;
        } else {
            let workspace = self.take_workspace();
            self.screens.push(Screen {
                id: screen,
                output: id,
                name: String::new(),
                workspace,
                region: RationalRect::FULL,
                rect: Rect::default(),
                usable_rect: Rect::default(),
            });
            self.sort_screens();
        }
        self.refresh_output(id);
    }

    /// Removes an output and its screens. Their workspaces become the most
    /// recently hidden ones (in screen order); if the current screen was
    /// among them, the first screen in physical order becomes current.
    /// Removing the last output makes the stack set headless, keeping the
    /// current workspace current.
    pub fn remove_output(&mut self, id: OutputId) {
        let Some(position) = self.outputs.iter().position(|o| o.id == id) else {
            return;
        };
        self.outputs.remove(position);
        let current_id = self.screens[self.current].id;
        let was_current = self.screens[self.current].output == id;
        if self.outputs.is_empty() {
            let mut screens = std::mem::take(&mut self.screens);
            let mut placeholder = screens.remove(self.current);
            self.hide_all(screens.into_iter().map(|s| s.workspace).collect());
            placeholder.id = PLACEHOLDER;
            placeholder.output = PLACEHOLDER_OUTPUT;
            placeholder.name.clear();
            placeholder.region = RationalRect::FULL;
            placeholder.rect = Rect::default();
            placeholder.usable_rect = Rect::default();
            self.screens = vec![placeholder];
            self.current = 0;
            self.headless = true;
            return;
        }
        let (removed, kept): (Vec<Screen>, Vec<Screen>) = std::mem::take(&mut self.screens)
            .into_iter()
            .partition(|s| s.output == id);
        self.screens = kept;
        self.hide_all(removed.into_iter().map(|s| s.workspace).collect());
        self.current = if was_current {
            let first = self.physical_order()[0];
            self.index_of(first).unwrap_or(0)
        } else {
            self.index_of(current_id).unwrap_or(0)
        };
    }

    /// Updates an output's rectangle (also resetting its usable area to
    /// the whole output). Its screens keep their proportions.
    pub fn set_output_rect(&mut self, id: OutputId, rect: Rect) {
        if let Some(output) = self.outputs.iter_mut().find(|o| o.id == id) {
            output.rect = rect;
            output.usable = rect;
            self.refresh_output(id);
        }
    }

    /// Updates the area of an output not covered by exclusive layer-shell
    /// surfaces; each of its screens may tile the part of it they cover.
    pub fn set_usable_rect(&mut self, id: OutputId, rect: Rect) {
        if let Some(output) = self.outputs.iter_mut().find(|o| o.id == id) {
            output.usable = rect;
            self.refresh_output(id);
        }
    }

    /// `XMonad.Layout.LayoutScreens.layoutSplitScreen n (TwoPane ½ ½)`:
    /// splits the current screen into `n` equal columns (for `n = 2`
    /// exactly `TwoPane 0.5 0.5`'s halves). The current screen keeps its
    /// workspace and the leftmost column and stays current; the new
    /// screens to its right show the most recently hidden workspaces in
    /// order (never the scratchpad workspace), or fresh ones when none are
    /// left. Does nothing while headless or for `n < 2`.
    pub fn split_screen(&mut self, n: usize) {
        let Ok(parts) = i64::try_from(n) else {
            return;
        };
        if self.headless || parts < 2 {
            return;
        }
        let region = self.screens[self.current].region;
        let width = region.w * Ratio::new(1, parts);
        let regions: Vec<RationalRect> = (0..parts)
            .map(|i| {
                RationalRect::new(
                    region.x + width * Ratio::from_int(i),
                    region.y,
                    width,
                    region.h,
                )
            })
            .collect();
        self.divide_screen(self.current, &regions);
    }

    /// Gives screen `index` the first of `regions` (fractions of its
    /// output) and adds a screen for each of the others, showing the most
    /// recently hidden workspaces (or fresh ones).
    pub(crate) fn divide_screen(&mut self, index: usize, regions: &[RationalRect]) {
        let Some((&first, rest)) = regions.split_first() else {
            return;
        };
        let output = self.screens[index].output;
        self.screens[index].region = first;
        for &region in rest {
            let workspace = self.take_workspace();
            let id = self.fresh_screen_id(ScreenId(self.next_virtual));
            self.screens.push(Screen {
                id,
                output,
                name: String::new(),
                workspace,
                region,
                rect: Rect::default(),
                usable_rect: Rect::default(),
            });
        }
        self.sort_screens();
        self.refresh_output(output);
    }

    /// Undoes every split, like `XMonad.Actions.PhysicalScreens.rescreen`
    /// once the physical screens are detected again: each output gets one
    /// screen covering it. On each output the current screen, else the
    /// first one in screen order, survives with its workspace (unsplit
    /// outputs keep theirs); the workspaces of the other screens become
    /// the most recently hidden ones, in screen order (xmonad's `map
    /// workspace visible ++ hidden`). No window is lost and the current
    /// workspace stays current.
    pub fn rescreen(&mut self) {
        if self.headless {
            return;
        }
        let current_id = self.screens[self.current].id;
        let current_output = self.screens[self.current].output;
        let mut kept: Vec<Screen> = Vec::new();
        let mut removed = Vec::new();
        for screen in std::mem::take(&mut self.screens) {
            let survives = if screen.output == current_output {
                screen.id == current_id
            } else {
                !kept.iter().any(|s| s.output == screen.output)
            };
            if survives {
                kept.push(screen);
            } else {
                removed.push(screen.workspace);
            }
        }
        for screen in &mut kept {
            screen.region = RationalRect::FULL;
        }
        // Back to the output's own number, unless that is taken.
        for index in 0..kept.len() {
            let own = ScreenId(kept[index].output.0);
            if !kept.iter().any(|s| s.id == own) {
                kept[index].id = own;
            }
        }
        let current_output_screen = kept
            .iter()
            .position(|s| s.output == current_output)
            .unwrap_or(0);
        self.current = current_output_screen;
        self.screens = kept;
        self.hide_all(removed);
        self.sort_screens();
        for id in self.outputs.iter().map(|o| o.id).collect::<Vec<_>>() {
            self.refresh_output(id);
        }
    }

    /// Puts `workspaces` in front of the hidden ones, keeping their order.
    fn hide_all(&mut self, workspaces: Vec<Workspace>) {
        self.hidden.splice(0..0, workspaces);
    }

    /// The workspace a new screen shows: the most recently hidden one
    /// (never the scratchpad workspace), else a fresh one.
    fn take_workspace(&mut self) -> Workspace {
        match self.hidden.iter().position(|w| w.tag != SCRATCHPAD_TAG) {
            Some(index) => self.hidden.remove(index),
            None => Workspace::new(self.fresh_tag(), self.layout_template.clone()),
        }
    }

    /// `preferred` if no screen uses it, else a fresh virtual screen id.
    fn fresh_screen_id(&mut self, preferred: ScreenId) -> ScreenId {
        let used = |ss: &Self, id: ScreenId| ss.screens.iter().any(|s| s.id == id);
        if !used(self, preferred) {
            if preferred.0 >= self.next_virtual {
                self.next_virtual = preferred.0 + 1;
            }
            return preferred;
        }
        loop {
            let id = ScreenId(self.next_virtual);
            self.next_virtual += 1;
            if !used(self, id) {
                return id;
            }
        }
    }

    /// Restores screen order (by output, then region), keeping the current
    /// screen.
    fn sort_screens(&mut self) {
        let current_id = self.screens[self.current].id;
        self.screens
            .sort_by_key(|s| (s.output, s.region.y, s.region.x, s.id));
        self.current = self.index_of(current_id).unwrap_or(0);
    }

    /// Recomputes the name and rectangles of output `id`'s screens.
    fn refresh_output(&mut self, id: OutputId) {
        let Some(output) = self.outputs.iter().find(|o| o.id == id) else {
            return;
        };
        for screen in self.screens.iter_mut().filter(|s| s.output == id) {
            output.name.clone_into(&mut screen.name);
            screen.rect = region_rect(screen.region, output.rect);
            screen.usable_rect = screen.rect.intersection(output.usable);
        }
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

    /// Shows workspace `tag` on screen `id` without changing the current
    /// screen: a workspace visible elsewhere swaps places with the one on
    /// `id`, a hidden one replaces it (which becomes the most recently
    /// hidden).
    pub fn show_on(&mut self, id: ScreenId, tag: &str) {
        let Some(index) = self.screens().iter().position(|s| s.id == id) else {
            return;
        };
        if self.screens[index].workspace.tag == tag {
            return;
        }
        if let Some(other) = self.screens.iter().position(|s| s.workspace.tag == tag) {
            let (a, b) = pair_mut(&mut self.screens, index, other);
            std::mem::swap(&mut a.workspace, &mut b.workspace);
        } else if let Some(hidden) = self.hidden.iter().position(|w| w.tag == tag) {
            let incoming = self.hidden.remove(hidden);
            let outgoing = std::mem::replace(&mut self.screens[index].workspace, incoming);
            self.hidden.insert(0, outgoing);
        }
    }

    /// Swaps the current screen's workspace with the next screen's in
    /// screen order (`swapNextScreen`, i.e. `greedyView` of the next
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
