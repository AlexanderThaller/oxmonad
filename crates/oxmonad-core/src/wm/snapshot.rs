//! Snapshots of the window-management state, for restarting in place
//! (xmonad's `restart`, which serializes the `WindowSet` and resumes it).
//!
//! A [`Snapshot`] names windows by a stable string (river's window
//! identifier) instead of [`WindowId`]s, which only live as long as one
//! process. The new process restores it in three steps:
//!
//! 1. [`Wm::restore`] on a fresh `Wm` rebuilds the workspaces (tags, order,
//!    layout state) and the bar toggle and keeps the snapshot pending;
//! 2. [`Wm::manage_as`] puts every window whose identifier the snapshot knows
//!    back on its workspace(s), floating or fullscreen as before, instead of
//!    running the manage rules;
//! 3. [`Wm::finish_restore`], once the windows that existed at startup are
//!    managed, splits the outputs that were split (by output name) again,
//!    restores the window order and focus of each workspace and which workspace
//!    each screen shows (screens are matched by output name, then by order),
//!    and drops the snapshot: windows that did not come back are forgotten.
//!
//! Snapshots written before screens could be split have no screen regions;
//! they load as unsplit outputs.

use std::collections::{
    BTreeMap,
    BTreeSet,
};

use serde::{
    Deserialize,
    Deserializer,
    Serialize,
};

use super::{
    WindowInfo,
    Wm,
};
use crate::{
    geometry::{
        OutputId,
        Ratio,
        RationalRect,
        ScreenId,
        WindowId,
    },
    layout::LayoutRing,
    stack_set::{
        Stack,
        StackSet,
    },
};

/// The window-management state in a form that outlives the process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    /// [`Snapshot::VERSION`] when written.
    pub version: u32,
    /// Tag of the current workspace.
    pub current: String,
    /// Every workspace: current, then the other screens', then the hidden
    /// ones, most recently hidden first.
    pub workspaces: Vec<WorkspaceSnapshot>,
    /// The screens in screen order, their region of their output and the
    /// workspace each shows.
    pub screens: Vec<ScreenSnapshot>,
    /// Floating windows and their screen-relative geometry.
    pub floating: BTreeMap<String, RationalRect>,
    /// Fullscreen windows.
    pub fullscreen: BTreeSet<String>,
    /// Whether the bar was shown.
    pub bar_visible: bool,
}

impl Snapshot {
    /// The snapshot format version this build reads and writes.
    pub const VERSION: u32 = 1;
}

/// One workspace in a [`Snapshot`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceSnapshot {
    /// The tag.
    pub tag: String,
    /// The layout ring with each layout's state; `None` when it could not
    /// be read (e.g. saved by a version with different layouts), in which
    /// case the workspace gets the configured ring.
    #[serde(default, deserialize_with = "lenient_ring")]
    pub layout: Option<LayoutRing>,
    /// The windows in stack order (master first).
    pub windows: Vec<String>,
    /// The focused window.
    pub focus: Option<String>,
}

/// Reads a saved layout ring, or `None` if it doesn't parse, so a layout
/// change between versions only resets layouts instead of rejecting the
/// whole snapshot.
fn lenient_ring<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<LayoutRing>, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(value).ok())
}

/// One screen in a [`Snapshot`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenSnapshot {
    /// The output name (may be empty).
    pub name: String,
    /// The tag of the workspace it shows.
    pub workspace: String,
    /// The part of the output the screen covers (all of it unless the
    /// output was split; absent in older snapshots).
    #[serde(default = "full_region")]
    pub region: RationalRect,
}

fn full_region() -> RationalRect {
    RationalRect::FULL
}

/// Why [`Wm::restore`] refused a snapshot.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RestoreError {
    /// The snapshot was written by an incompatible version.
    #[error("snapshot version {0} is not supported (expected {expected})", expected = Snapshot::VERSION)]
    Version(u32),
    /// The snapshot is inconsistent.
    #[error("invalid snapshot: {0}")]
    Invalid(&'static str),
}

/// A snapshot being restored, and the windows that came back so far.
#[derive(Debug, Clone)]
pub(super) struct PendingRestore {
    snapshot: Snapshot,
    windows: BTreeMap<String, WindowId>,
}

impl Wm {
    /// Captures the state; `id_of` names each window (windows it returns
    /// `None` for are left out).
    #[must_use]
    pub fn snapshot(&self, id_of: impl Fn(WindowId) -> Option<String>) -> Snapshot {
        let ss = &self.stack_set;
        let workspaces = ss
            .workspaces()
            .map(|w| WorkspaceSnapshot {
                tag: w.tag.clone(),
                layout: Some(w.layout.clone()),
                windows: w.windows().into_iter().filter_map(&id_of).collect(),
                focus: w.focus().and_then(&id_of),
            })
            .collect();
        Snapshot {
            version: Snapshot::VERSION,
            current: ss.current_tag().to_owned(),
            workspaces,
            screens: ss
                .screens()
                .iter()
                .map(|s| ScreenSnapshot {
                    name: s.name.clone(),
                    workspace: s.workspace.tag.clone(),
                    region: s.region,
                })
                .collect(),
            floating: ss
                .floating()
                .iter()
                .filter_map(|(&w, &rect)| Some((id_of(w)?, rect)))
                .collect(),
            fullscreen: self.fullscreen.iter().filter_map(|&w| id_of(w)).collect(),
            bar_visible: self.bar_visible,
        }
    }

    /// Starts restoring `snapshot` (see the [module docs](self)). Call it
    /// on a fresh `Wm`, before any screen or window is added.
    ///
    /// The snapshot's workspaces replace the configured ones (configured
    /// workspaces it lacks are added); each keeps its layout state if the
    /// configured layout ring still has the same layouts.
    pub fn restore(&mut self, snapshot: Snapshot) -> Result<(), RestoreError> {
        if snapshot.version != Snapshot::VERSION {
            return Err(RestoreError::Version(snapshot.version));
        }
        let tags: Vec<String> = snapshot.workspaces.iter().map(|w| w.tag.clone()).collect();
        if tags.iter().any(String::is_empty) {
            return Err(RestoreError::Invalid("empty workspace tag"));
        }
        if tags.iter().collect::<BTreeSet<_>>().len() != tags.len() {
            return Err(RestoreError::Invalid("duplicate workspace tag"));
        }
        // The current workspace first: it stays on the (placeholder)
        // current screen.
        let Some(current) = tags.iter().position(|t| *t == snapshot.current) else {
            return Err(RestoreError::Invalid("unknown current workspace"));
        };
        let mut order = tags.clone();
        let current = order.remove(current);
        order.insert(0, current);

        let template = self.stack_set.layout_template().clone();
        let mut stack_set = StackSet::new(&order, template.clone());
        for saved in &snapshot.workspaces {
            if let Some(workspace) = stack_set.workspace_mut(&saved.tag)
                && let Some(layout) = &saved.layout
            {
                workspace.layout = template.restore(layout);
            }
        }
        for tag in &self.config.workspaces {
            stack_set.add_workspace(tag);
        }
        stack_set.order_hidden(&order);
        self.stack_set = stack_set;
        self.windows.clear();
        self.fullscreen.clear();
        self.prompt = None;
        self.bar_visible = snapshot.bar_visible;
        self.restore = Some(PendingRestore {
            snapshot,
            windows: BTreeMap::new(),
        });
        Ok(())
    }

    /// Whether a snapshot is being restored.
    #[must_use]
    pub fn is_restoring(&self) -> bool {
        self.restore.is_some()
    }

    /// [`Self::manage`] for a window river names `identifier`: while a
    /// snapshot is being restored, a window it knows goes back to its
    /// workspaces, floating or fullscreen as before, without running the
    /// manage rules.
    pub fn manage_as(&mut self, id: WindowId, identifier: Option<&str>, info: WindowInfo) {
        if !self.windows.contains_key(&id)
            && let Some(identifier) = identifier
            && self.restore_window(id, identifier)
        {
            self.windows.insert(id, info);
            return;
        }
        self.manage(id, info);
    }

    /// Puts a window back where the pending snapshot had it; returns
    /// whether the snapshot knew it.
    fn restore_window(&mut self, id: WindowId, identifier: &str) -> bool {
        let Some(pending) = &mut self.restore else {
            return false;
        };
        if pending.windows.contains_key(identifier) {
            return false;
        }
        let snapshot = &pending.snapshot;
        let tags: Vec<&str> = snapshot
            .workspaces
            .iter()
            .filter(|w| w.windows.iter().any(|i| i == identifier))
            .map(|w| w.tag.as_str())
            .filter(|tag| self.stack_set.has_tag(tag))
            .collect();
        if tags.is_empty() {
            return false;
        }
        for tag in tags {
            self.stack_set.insert_up_on(tag, id);
        }
        if let Some(&rect) = snapshot.floating.get(identifier) {
            self.stack_set.float(id, rect);
        }
        if snapshot.fullscreen.contains(identifier) {
            self.fullscreen.insert(id);
        }
        pending.windows.insert(identifier.to_owned(), id);
        true
    }

    /// Completes a restore (see the [module docs](self)): window order and
    /// focus per workspace, the workspace on each screen, the current
    /// workspace and the hidden order. Windows the snapshot knew that did
    /// not come back are forgotten. Does nothing unless restoring.
    pub fn finish_restore(&mut self) {
        let Some(PendingRestore { snapshot, windows }) = self.restore.take() else {
            return;
        };
        let ss = &mut self.stack_set;
        restore_splits(ss, &snapshot.screens);
        for saved in &snapshot.workspaces {
            let Some(workspace) = ss.workspace_mut(&saved.tag) else {
                continue;
            };
            let current = workspace.windows();
            let mut order: Vec<WindowId> = saved
                .windows
                .iter()
                .filter_map(|i| windows.get(i).copied())
                .filter(|w| current.contains(w))
                .collect();
            // Windows managed normally meanwhile keep their order, below.
            let others: Vec<WindowId> = current
                .iter()
                .copied()
                .filter(|w| !order.contains(w))
                .collect();
            order.extend(others);
            let focus = saved
                .focus
                .as_ref()
                .and_then(|i| windows.get(i).copied())
                .or_else(|| workspace.focus());
            workspace.stack = Stack::from_list(order).map(|mut stack| {
                if let Some(focus) = focus {
                    stack.focus_on(&focus);
                }
                stack
            });
        }

        for (screen, tag) in match_screens(ss, &snapshot.screens) {
            if ss.has_tag(&tag) {
                ss.show_on(screen, &tag);
            }
        }
        match ss.screen_of_tag(&snapshot.current) {
            Some(screen) => ss.focus_screen(screen),
            None => ss.view(&snapshot.current),
        }
        let order: Vec<String> = snapshot.workspaces.iter().map(|w| w.tag.clone()).collect();
        ss.order_hidden(&order);
    }
}

/// Splits each unsplit output the way the snapshot had it split, matching
/// outputs by name. Splits whose regions do not lie within the output are
/// ignored.
fn restore_splits(ss: &mut StackSet, saved: &[ScreenSnapshot]) {
    let outputs: Vec<(OutputId, String)> = ss
        .outputs()
        .iter()
        .map(|o| (o.id, o.name.clone()))
        .filter(|(_, name)| !name.is_empty())
        .collect();
    for (output, name) in outputs {
        let regions: Vec<RationalRect> = saved
            .iter()
            .filter(|s| s.name == name)
            .map(|s| s.region)
            .collect();
        if regions.len() < 2 || !regions.iter().all(|&r| within_unit(r)) {
            continue;
        }
        let on_output: Vec<usize> = ss
            .screens()
            .iter()
            .enumerate()
            .filter(|(_, s)| s.output == output)
            .map(|(i, _)| i)
            .collect();
        if let [index] = on_output[..] {
            ss.divide_screen(index, &regions);
        }
    }
}

/// Whether `r` is a non-empty region inside the unit square.
fn within_unit(r: RationalRect) -> bool {
    let (zero, one) = (Ratio::ZERO, Ratio::ONE);
    r.x >= zero && r.y >= zero && r.w > zero && r.h > zero && r.x + r.w <= one && r.y + r.h <= one
}

/// Pairs today's screens with the snapshot's: by output name first, the
/// rest in order. Returns `(screen, tag it should show)`.
fn match_screens(ss: &StackSet, saved: &[ScreenSnapshot]) -> Vec<(ScreenId, String)> {
    let screens: Vec<(ScreenId, &str)> = ss
        .screens()
        .iter()
        .map(|s| (s.id, s.name.as_str()))
        .collect();
    let mut taken = vec![false; saved.len()];
    let mut matched: Vec<Option<usize>> = screens
        .iter()
        .map(|&(_, name)| {
            if name.is_empty() {
                return None;
            }
            let index = saved
                .iter()
                .enumerate()
                .position(|(i, s)| !taken[i] && s.name == name)?;
            taken[index] = true;
            Some(index)
        })
        .collect();
    for slot in &mut matched {
        if slot.is_none() {
            *slot = taken.iter().position(|t| !t);
            if let Some(index) = *slot {
                taken[index] = true;
            }
        }
    }
    screens
        .iter()
        .zip(matched)
        .filter_map(|(&(id, _), index)| Some((id, saved[index?].workspace.clone())))
        .collect()
}
