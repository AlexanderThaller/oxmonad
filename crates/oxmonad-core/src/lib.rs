//! Pure window-management model of oxmonad.
//!
//! This crate is xmonad's brain without the X11: the [`stack_set`] of
//! workspaces and screens, the [`layout`] ring, manage [`rules`], the
//! workspace [`prompt`], the [`layout_picker`], the key binding [`help`],
//! [`action`]s and the TOML [`config`] schema, all behind the [`Wm`] facade.
//! It performs no I/O, owns no threads and knows nothing about Wayland; the
//! `oxmonad` binary feeds it events and turns its [`Arrangement`] and
//! [`Effect`]s into river protocol requests.

pub mod action;
pub mod config;
pub mod geometry;
pub mod help;
pub mod layout;
pub mod layout_picker;
pub mod prompt;
pub mod rules;
pub mod stack_set;
pub mod wm;

pub use action::{
    Action,
    Direction,
    Effect,
};
pub use geometry::{
    OutputId,
    Ratio,
    RationalRect,
    Rect,
    ScreenId,
    Size,
    WindowId,
};
pub use wm::{
    Arrangement,
    BarModel,
    Border,
    Focus,
    HelpModel,
    Layer,
    LayoutPickerModel,
    LayoutPreview,
    Placement,
    PromptModel,
    RestoreError,
    ScreenArrangement,
    ScreenSnapshot,
    Snapshot,
    Tab,
    TabStrip,
    WindowInfo,
    Wm,
    WorkspaceSnapshot,
    WorkspaceState,
};
