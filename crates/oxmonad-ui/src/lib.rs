//! Software rendering of oxmonad's own UI: the status bar (with tray
//! icons), the workspace prompt, the layout picker, the key binding help,
//! popup menus and the tabbed layout's tab strip.
//!
//! Every widget is a pure function from a plain view struct and a style to a
//! [`Canvas`]: premultiplied ARGB8888 pixels in the `wl_shm` `argb8888`
//! layout, rendered at an integer scale. No Wayland or other I/O happens here
//! apart from reading font files in [`Fonts::from_family`].

mod bar;
mod canvas;
mod color;
mod help;
mod icon;
mod layout_picker;
mod menu;
mod prompt;
mod tabs;
mod text;

pub use bar::{
    BarLayout,
    BarStyle,
    BarView,
    BatteryState,
    BatteryStatus,
    BatteryStyle,
    LayoutGlyph,
    PpStyle,
    Segment,
    SegmentId,
    Side,
    WorkspaceState,
    WorkspaceTag,
    format_battery,
    join_groups,
    layout_bar,
    pp_segments,
    render_bar,
    shorten,
};
pub use canvas::Canvas;
pub use color::{
    Color,
    ParseColorError,
};
pub use help::{
    HelpBinding,
    HelpGroup,
    HelpStyle,
    HelpView,
    help_size,
    help_viewport,
    render_help,
};
pub use icon::Icon;
pub use layout_picker::{
    LayoutPickerStyle,
    LayoutPickerView,
    PickerEntry,
    PreviewRect,
    PreviewWindow,
    layout_picker_size,
    picker_entry_at,
    picker_entry_rects,
    render_layout_picker,
};
pub use menu::{
    MenuEntry,
    MenuEntryKind,
    MenuStyle,
    MenuView,
    menu_entry_at,
    menu_entry_rects,
    menu_size,
    render_menu,
};
pub use prompt::{
    PromptPosition,
    PromptStyle,
    PromptView,
    prompt_size,
    render_prompt,
};
pub use tabs::{
    Tab,
    TabColors,
    TabStyle,
    TabsView,
    render_tabs,
    tab_at,
};
pub use text::{
    Antialias,
    FontError,
    FontFile,
    Fonts,
};
