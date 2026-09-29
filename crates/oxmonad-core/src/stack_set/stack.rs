//! The [`Stack`] zipper: a non-empty list with one focused element.

/// A non-empty list with a focused element (xmonad's `Stack`).
///
/// Unlike xmonad, both halves are stored in screen order: [`Self::up`]
/// runs from the master down to the element directly above the focus,
/// [`Self::down`] from the element directly below the focus to the bottom.
/// The whole list in order is `up ++ [focus] ++ down`
/// ([`Self::integrate`]).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Stack<T> {
    /// The focused element.
    pub focus: T,
    /// Elements above the focus, master first.
    pub up: Vec<T>,
    /// Elements below the focus, nearest first.
    pub down: Vec<T>,
}

impl<T: Clone + PartialEq> Stack<T> {
    /// A stack holding only `focus`.
    #[must_use]
    pub fn singleton(focus: T) -> Self {
        Self {
            focus,
            up: Vec::new(),
            down: Vec::new(),
        }
    }

    /// xmonad's `differentiate`: the list as a stack focused on its first
    /// element, or `None` for an empty list.
    #[must_use]
    pub fn from_list(mut items: Vec<T>) -> Option<Self> {
        if items.is_empty() {
            return None;
        }
        let focus = items.remove(0);
        Some(Self {
            focus,
            up: Vec::new(),
            down: items,
        })
    }

    /// All elements in order (xmonad's `integrate`).
    #[must_use]
    pub fn integrate(&self) -> Vec<T> {
        self.iter().cloned().collect()
    }

    /// Iterates over all elements in order.
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &T> {
        self.up
            .iter()
            .chain(std::iter::once(&self.focus))
            .chain(self.down.iter())
    }

    /// Number of elements.
    #[must_use]
    pub fn len(&self) -> usize {
        self.up.len() + 1 + self.down.len()
    }

    /// Always `false`: a stack is never empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        false
    }

    /// Index of the focused element in [`Self::integrate`].
    #[must_use]
    pub fn focus_index(&self) -> usize {
        self.up.len()
    }

    /// Whether `item` is in the stack.
    #[must_use]
    pub fn contains(&self, item: &T) -> bool {
        self.iter().any(|x| x == item)
    }

    /// xmonad's `focusUp'`: focus the previous element, wrapping to the
    /// bottom.
    pub fn focus_up(&mut self) {
        if let Some(above) = self.up.pop() {
            let old = std::mem::replace(&mut self.focus, above);
            self.down.insert(0, old);
        } else if let Some(bottom) = self.down.pop() {
            let old = std::mem::replace(&mut self.focus, bottom);
            self.up = std::iter::once(old).chain(self.down.drain(..)).collect();
        }
    }

    /// xmonad's `focusDown'`: focus the next element, wrapping to the top.
    pub fn focus_down(&mut self) {
        self.reverse();
        self.focus_up();
        self.reverse();
    }

    /// xmonad's `swapUp'`: move the focused element up, wrapping to the
    /// bottom.
    pub fn swap_up(&mut self) {
        if let Some(above) = self.up.pop() {
            self.down.insert(0, above);
        } else {
            self.up = std::mem::take(&mut self.down);
        }
    }

    /// xmonad's `swapDown`: move the focused element down, wrapping to the
    /// top.
    pub fn swap_down(&mut self) {
        self.reverse();
        self.swap_up();
        self.reverse();
    }

    /// xmonad's `swapMaster`: exchange the focused element with the master,
    /// keeping focus on it.
    pub fn swap_master(&mut self) {
        if self.up.is_empty() {
            return;
        }
        let master = self.up.remove(0);
        let mut down = std::mem::take(&mut self.up);
        down.push(master);
        down.append(&mut self.down);
        self.down = down;
    }

    /// xmonad's `shiftMaster`: move the focused element to the top,
    /// keeping the order of the others.
    pub fn shift_master(&mut self) {
        let mut down = std::mem::take(&mut self.up);
        down.append(&mut self.down);
        self.down = down;
    }

    /// xmonad's `focusMaster`: focus the first element.
    pub fn focus_master(&mut self) {
        if self.up.is_empty() {
            return;
        }
        let master = self.up.remove(0);
        let old = std::mem::replace(&mut self.focus, master);
        let mut down = std::mem::take(&mut self.up);
        down.push(old);
        down.append(&mut self.down);
        self.down = down;
    }

    /// Moves the focus onto `item` if present; returns whether it was found.
    pub fn focus_on(&mut self, item: &T) -> bool {
        if !self.contains(item) {
            return false;
        }
        while self.focus != *item {
            self.focus_up();
        }
        true
    }

    /// xmonad's `insertUp` on a stack: `item` becomes the focus, directly
    /// above the old focus.
    pub fn insert_up(&mut self, item: T) {
        let old = std::mem::replace(&mut self.focus, item);
        self.down.insert(0, old);
    }

    /// xmonad's `filter`: keeps the elements satisfying `keep`. The focus
    /// moves down if it is removed, else up; `None` if nothing is left.
    #[must_use]
    pub fn filter(&self, mut keep: impl FnMut(&T) -> bool) -> Option<Self> {
        let up: Vec<T> = self.up.iter().filter(|x| keep(x)).cloned().collect();
        let mut rest = std::iter::once(&self.focus)
            .chain(self.down.iter())
            .filter(|x| keep(x))
            .cloned();
        if let Some(focus) = rest.next() {
            return Some(Self {
                focus,
                up,
                down: rest.collect(),
            });
        }
        let mut up = up;
        let focus = up.pop()?;
        Some(Self {
            focus,
            up,
            down: Vec::new(),
        })
    }

    fn reverse(&mut self) {
        std::mem::swap(&mut self.up, &mut self.down);
        self.up.reverse();
        self.down.reverse();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a stack of `items` focused at `focus`.
    fn stack(items: &[u32], focus: usize) -> Stack<u32> {
        Stack {
            focus: items[focus],
            up: items[..focus].to_vec(),
            down: items[focus + 1..].to_vec(),
        }
    }

    fn state(s: &Stack<u32>) -> (Vec<u32>, u32) {
        (s.integrate(), s.focus)
    }

    #[test]
    fn focus_up_and_down_wrap() {
        let mut s = stack(&[1, 2, 3], 0);
        s.focus_up();
        assert_eq!(state(&s), (vec![1, 2, 3], 3));
        s.focus_down();
        assert_eq!(state(&s), (vec![1, 2, 3], 1));
        s.focus_down();
        assert_eq!(state(&s), (vec![1, 2, 3], 2));
        s.focus_up();
        assert_eq!(state(&s), (vec![1, 2, 3], 1));
    }

    #[test]
    fn swaps_wrap_like_xmonad() {
        let mut s = stack(&[1, 2, 3], 1);
        s.swap_up();
        assert_eq!(state(&s), (vec![2, 1, 3], 2));
        s.swap_up();
        // At the top, swapUp moves the focus to the bottom.
        assert_eq!(state(&s), (vec![1, 3, 2], 2));
        s.swap_down();
        // At the bottom, swapDown moves it to the top.
        assert_eq!(state(&s), (vec![2, 1, 3], 2));
        s.swap_down();
        assert_eq!(state(&s), (vec![1, 2, 3], 2));
    }

    #[test]
    fn master_operations() {
        let mut s = stack(&[1, 2, 3, 4], 3);
        s.swap_master();
        assert_eq!(state(&s), (vec![4, 2, 3, 1], 4));
        let mut s = stack(&[1, 2, 3, 4], 2);
        s.shift_master();
        assert_eq!(state(&s), (vec![3, 1, 2, 4], 3));
        let mut s = stack(&[1, 2, 3, 4], 2);
        s.focus_master();
        assert_eq!(state(&s), (vec![1, 2, 3, 4], 1));
        let mut s = stack(&[1, 2], 0);
        s.swap_master();
        assert_eq!(state(&s), (vec![1, 2], 1));
    }

    #[test]
    fn insert_up_goes_above_focus() {
        let mut s = stack(&[1, 2, 3], 1);
        s.insert_up(9);
        assert_eq!(state(&s), (vec![1, 9, 2, 3], 9));
    }

    #[test]
    fn filter_moves_focus_down_then_up() {
        let s = stack(&[1, 2, 3], 1);
        let f = s.filter(|&x| x != 2).expect("non-empty");
        assert_eq!(state(&f), (vec![1, 3], 3));
        let s = stack(&[1, 2, 3], 2);
        let f = s.filter(|&x| x != 3).expect("non-empty");
        assert_eq!(state(&f), (vec![1, 2], 2));
        assert!(stack(&[1], 0).filter(|_| false).is_none());
    }

    #[test]
    fn focus_on_and_from_list() {
        let mut s = Stack::from_list(vec![1, 2, 3]).expect("non-empty");
        assert_eq!(state(&s), (vec![1, 2, 3], 1));
        assert!(s.focus_on(&3));
        assert_eq!(state(&s), (vec![1, 2, 3], 3));
        assert!(!s.focus_on(&7));
        assert_eq!(s.focus_index(), 2);
        assert!(Stack::<u32>::from_list(Vec::new()).is_none());
    }
}
