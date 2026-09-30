//! Software rendering of oxmonad's own UI: the status bar, the workspace
//! prompt and the tabbed layout's tab strip.
//!
//! Every widget is a pure function from a plain view struct and a style to a
//! [`Canvas`]: premultiplied ARGB8888 pixels in the `wl_shm` `argb8888`
//! layout, rendered at an integer scale. No Wayland or other I/O happens here
//! apart from reading font files in [`Fonts::from_family`].

mod bar;
mod canvas;
mod color;
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
    Fonts,
};
