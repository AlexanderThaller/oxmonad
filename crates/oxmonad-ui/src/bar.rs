//! The status bar: xmobar's `%StdinReader% }{ %battery% | %date%` template
//! with the xmonad pretty-printer (`myPP`) on the left.

use std::time::Duration;

use crate::{
    Antialias,
    Canvas,
    Color,
    Fonts,
    Icon,
    PreviewRect,
    PreviewWindow,
    canvas::{
        Painter,
        round_i32,
        to_f32,
        to_i32,
    },
};

/// A run of text in one color, an icon or a layout miniature, optionally on
/// its own background.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    /// The text drawn; ignored when [`Segment::icon`] or [`Segment::layout`]
    /// is set.
    pub text: String,
    /// Text color; `None` uses [`BarStyle::fg`].
    pub fg: Option<Color>,
    /// Background behind the text; `None` leaves the bar background.
    pub bg: Option<Color>,
    /// What clicking the segment means to the binary (like xmobar's
    /// `<action=…>`); [`pp_segments`] sets it to the workspace tag.
    pub action: Option<String>,
    /// An icon drawn instead of the text: a [`BarStyle::icon_size`] square
    /// with [`BarStyle::icon_spacing`] on either side, the icon scaled to
    /// fit it and vertically centered in the bar.
    pub icon: Option<Icon>,
    /// A layout miniature drawn instead of the text:
    /// [`BarStyle::layout_icon_size`] high, as wide as its area's aspect ratio,
    /// with [`BarStyle::icon_spacing`] on either side, in the segment's
    /// color.
    pub layout: Option<LayoutGlyph>,
}

impl Segment {
    /// Text in the bar's default colors.
    #[must_use]
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            fg: None,
            bg: None,
            action: None,
            icon: None,
            layout: None,
        }
    }

    /// A layout miniature segment (see [`Segment::layout`]).
    #[must_use]
    pub fn with_layout(glyph: LayoutGlyph) -> Self {
        Self {
            layout: Some(glyph),
            ..Self::new("")
        }
    }

    /// Whether the segment draws a picture instead of its text.
    fn is_graphic(&self) -> bool {
        self.icon.is_some() || self.layout.is_some()
    }

    /// An icon segment (see [`Segment::icon`]) in the bar's default colors.
    #[must_use]
    pub fn with_icon(icon: Icon) -> Self {
        Self::new("").icon(icon)
    }

    /// Sets the text color.
    #[must_use]
    pub fn fg(mut self, color: Color) -> Self {
        self.fg = Some(color);
        self
    }

    /// Sets the background color.
    #[must_use]
    pub fn bg(mut self, color: Color) -> Self {
        self.bg = Some(color);
        self
    }

    /// Sets what clicking the segment means to the binary.
    #[must_use]
    pub fn action(mut self, action: impl Into<String>) -> Self {
        self.action = Some(action.into());
        self
    }

    /// Draws `icon` instead of the text.
    #[must_use]
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }
}

/// A miniature of a layout: what it does with a few windows, like the
/// layout picker's cards, in `area` preview coordinates.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LayoutGlyph {
    /// Width and height of the previewed screen.
    pub area: (u32, u32),
    /// The windows the layout shows; window 0 (the master) is highlighted.
    pub windows: Vec<PreviewWindow>,
    /// The tab strip's tabs (empty without a strip).
    pub tabs: Vec<PreviewRect>,
}

impl LayoutGlyph {
    /// The logical width of the miniature at `height` logical pixels.
    fn width(&self, height: u32) -> u32 {
        let (width, area_height) = (u64::from(self.area.0), u64::from(self.area.1));
        if area_height == 0 {
            return height;
        }
        let scaled = (u64::from(height) * width + area_height / 2) / area_height;
        u32::try_from(scaled).unwrap_or(height).max(1)
    }
}

/// What the bar shows: segments flush left and flush right (xmobar's `}{`).
///
/// When both sides do not fit, the right side wins and the left side is
/// ellipsized at the segment that crosses into it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BarView {
    pub segments_left: Vec<Segment>,
    pub segments_right: Vec<Segment>,
}

impl BarView {
    /// The segment a [`SegmentId`] refers to.
    #[must_use]
    pub fn segment(&self, id: SegmentId) -> Option<&Segment> {
        match id.side {
            Side::Left => self.segments_left.get(id.index),
            Side::Right => self.segments_right.get(id.index),
        }
    }
}

/// Bar colors and metrics, in logical pixels. Defaults follow `xmobarrc`.
#[derive(Debug, Clone, PartialEq)]
pub struct BarStyle {
    pub bg: Color,
    pub fg: Color,
    /// Font size in logical pixels (xmobar's `size=12` at 96 dpi is 16 px).
    pub font_size: f32,
    /// Glyph edge antialiasing.
    pub antialias: Antialias,
    pub height: u32,
    /// Space before the first and after the last segment.
    pub padding: u32,
    /// Put between right-side groups by [`join_groups`] (the template's
    /// literal `" | "`).
    pub separator: String,
    /// Side of the square an icon segment's icon is fitted into.
    pub icon_size: u32,
    /// Space on either side of an icon segment's icon.
    pub icon_spacing: u32,
    /// Height of a layout miniature ([`Segment::layout`]).
    pub layout_icon_size: u32,
}

impl Default for BarStyle {
    fn default() -> Self {
        Self {
            bg: Color::hex("#161616"),
            fg: Color::hex("#F2F2F2"),
            font_size: 16.0,
            height: 24,
            padding: 6,
            separator: " | ".to_owned(),
            antialias: Antialias::Grayscale,
            icon_size: 16,
            icon_spacing: 4,
            layout_icon_size: 14,
        }
    }
}

/// Concatenates groups of segments (e.g. battery, date), putting a
/// default-colored `separator` between non-empty groups.
#[must_use]
pub fn join_groups(
    groups: impl IntoIterator<Item = Vec<Segment>>,
    separator: &str,
) -> Vec<Segment> {
    let mut joined = Vec::new();
    for group in groups.into_iter().filter(|group| !group.is_empty()) {
        if !joined.is_empty() {
            joined.push(Segment::new(separator));
        }
        joined.extend(group);
    }
    joined
}

/// Which side of the bar a segment is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    Left,
    Right,
}

/// Identifies a segment of a [`BarView`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SegmentId {
    pub side: Side,
    pub index: usize,
}

/// A segment's final placement: device-pixel span and the (possibly
/// ellipsized) text drawn there (empty for icon segments).
#[derive(Debug, Clone, PartialEq)]
struct Placed {
    id: SegmentId,
    x: f32,
    width: f32,
    text: String,
}

/// Where each segment of a bar ended up, for hit-testing clicks.
#[derive(Debug, Clone, PartialEq)]
pub struct BarLayout {
    scale: u32,
    placed: Vec<Placed>,
}

impl BarLayout {
    /// The segment under logical x coordinate `x` (relative to the bar's left
    /// edge), if any.
    #[must_use]
    pub fn segment_at(&self, x: f64) -> Option<SegmentId> {
        let x = x * f64::from(self.scale);
        self.placed
            .iter()
            .find(|placed| {
                let start = f64::from(placed.x);
                x >= start && x < start + f64::from(placed.width)
            })
            .map(|placed| placed.id)
    }

    /// Where segment `id` was placed: its `(start, end)` x coordinates in
    /// logical pixels relative to the bar's left edge, e.g. to anchor a popup
    /// menu under a clicked tray icon. `None` when the segment did not fit.
    #[must_use]
    pub fn segment_span(&self, id: SegmentId) -> Option<(f64, f64)> {
        let scale = f64::from(self.scale);
        self.placed
            .iter()
            .find(|placed| placed.id == id)
            .map(|placed| {
                let start = f64::from(placed.x);
                (start / scale, (start + f64::from(placed.width)) / scale)
            })
    }
}

/// The device-pixel width of a segment at device font size `size`.
fn segment_width(segment: &Segment, style: &BarStyle, fonts: &Fonts, size: f32, scale: u32) -> f32 {
    if let Some(glyph) = &segment.layout {
        to_f32(to_i32(
            (glyph.width(style.layout_icon_size) + 2 * style.icon_spacing) * scale,
        ))
    } else if segment.icon.is_some() {
        to_f32(to_i32((style.icon_size + 2 * style.icon_spacing) * scale))
    } else {
        fonts.text_width(&segment.text, size)
    }
}

/// Lays the bar out without drawing it (for hit-testing).
#[must_use]
pub fn layout_bar(
    view: &BarView,
    style: &BarStyle,
    fonts: &Fonts,
    width: u32,
    scale: u32,
) -> BarLayout {
    let scale = scale.max(1);
    let size = style.font_size * to_f32(to_i32(scale));
    let width = to_f32(to_i32(width.max(1) * scale));
    let padding = to_f32(to_i32(style.padding * scale));
    let mut placed = Vec::new();

    let right_width: f32 = view
        .segments_right
        .iter()
        .map(|segment| segment_width(segment, style, fonts, size, scale))
        .sum();
    let right_start = (width - padding - right_width).max(padding);
    let mut pen = right_start;
    for (index, segment) in view.segments_right.iter().enumerate() {
        let segment_width = segment_width(segment, style, fonts, size, scale);
        let text = if segment.is_graphic() {
            String::new()
        } else {
            segment.text.clone()
        };
        placed.push(Placed {
            id: SegmentId {
                side: Side::Right,
                index,
            },
            x: pen,
            width: segment_width,
            text,
        });
        pen += segment_width;
    }

    let left_limit = if view.segments_right.is_empty() {
        width - padding
    } else {
        right_start - padding
    };
    let mut pen = padding;
    for (index, segment) in view.segments_left.iter().enumerate() {
        let id = SegmentId {
            side: Side::Left,
            index,
        };
        if segment.is_graphic() {
            let segment_width = segment_width(segment, style, fonts, size, scale);
            if pen + segment_width > left_limit {
                break;
            }
            placed.push(Placed {
                id,
                x: pen,
                width: segment_width,
                text: String::new(),
            });
            pen += segment_width;
            continue;
        }
        let text = fonts.ellipsize(&segment.text, size, left_limit - pen);
        if text.is_empty() {
            break;
        }
        let segment_width = fonts.text_width(&text, size);
        let truncated = *text != *segment.text;
        placed.push(Placed {
            id,
            x: pen,
            width: segment_width,
            text: text.into_owned(),
        });
        pen += segment_width;
        if truncated {
            break;
        }
    }

    BarLayout { scale, placed }
}

/// Renders the bar `width` logical pixels wide at `scale`; the canvas is
/// `width * scale` × `style.height * scale` device pixels.
#[must_use]
pub fn render_bar(
    view: &BarView,
    style: &BarStyle,
    fonts: &Fonts,
    width: u32,
    scale: u32,
) -> Canvas {
    let layout = layout_bar(view, style, fonts, width, scale);
    let mut painter = Painter::new(width, style.height, scale).with_antialias(style.antialias);
    let (canvas_width, canvas_height) = (painter.width(), painter.height());
    let size = style.font_size * to_f32(to_i32(painter.scale()));
    painter.fill_rect(0, 0, canvas_width, canvas_height, style.bg);
    for placed in &layout.placed {
        let Some(segment) = view.segment(placed.id) else {
            continue;
        };
        let (start, end) = (round_i32(placed.x), round_i32(placed.x + placed.width));
        if let Some(bg) = segment.bg {
            painter.fill_rect(start, 0, end - start, canvas_height, bg);
        }
        if let Some(icon) = &segment.icon {
            let box_size = style.icon_size * painter.scale();
            let (icon_width, icon_height) = icon.fitted_size(box_size);
            let x = start + (end - start - to_i32(icon_width)) / 2;
            let y = (canvas_height - to_i32(icon_height)) / 2;
            painter.draw_icon(icon, x, y, icon_width, icon_height);
            continue;
        }
        if let Some(glyph) = &segment.layout {
            let fg = segment.fg.unwrap_or(style.fg);
            draw_layout_glyph(
                &mut painter,
                glyph,
                (start, end),
                style.layout_icon_size,
                fg,
            );
            continue;
        }
        painter.draw_text(
            fonts,
            &placed.text,
            placed.x,
            0,
            canvas_height,
            size,
            segment.fg.unwrap_or(style.fg),
            (0, canvas_width),
        );
    }
    painter.finish()
}

/// Smallest height of a layout miniature's tab strip in logical pixels
/// (without the gap below it).
const MIN_TAB_STRIP: i32 = 4;

/// Draws `glyph` `size` logical pixels high, centered in the device
/// span `(start, end)`: the master window and its tab in `fg`, the others
/// fainter, each separated from its neighbours by a device-pixel gap per
/// scale step.
fn draw_layout_glyph(
    painter: &mut Painter,
    glyph: &LayoutGlyph,
    (start, end): (i32, i32),
    size: u32,
    fg: Color,
) {
    let scale = painter.scale();
    let gap = to_i32(scale);
    let (width, height) = (to_i32(glyph.width(size) * scale), to_i32(size * scale));
    let (left, top) = (
        start + (end - start - width) / 2,
        (painter.height() - height) / 2,
    );
    let (area_width, area_height) = (
        i64::from(glyph.area.0.max(1)),
        i64::from(glyph.area.1.max(1)),
    );
    // Rectangles are mapped into a box one gap larger and lose the gap on
    // their right and bottom edges, so neighbours are separated and the
    // outer edges stay inside `width` × `height`.
    let scaled = |value: i64, extent: i64, area: i64| -> i32 {
        i32::try_from(value * extent / area.max(1)).unwrap_or(0)
    };
    let map_x = |x: i32| scaled(i64::from(x), i64::from(width + gap), area_width);
    // A tab strip is a small part of the preview and would vanish at bar
    // size, so it gets at least `MIN_TAB_STRIP` and the windows below it
    // share the rest.
    let strip_bottom = glyph
        .tabs
        .iter()
        .map(|tab| i64::from(tab.y + tab.h))
        .max()
        .unwrap_or(0)
        .clamp(0, area_height);
    let strip_px = if strip_bottom > 0 {
        scaled(strip_bottom, i64::from(height + gap), area_height)
            .max(MIN_TAB_STRIP * gap + gap)
            .min(height / 2 + gap)
    } else {
        0
    };
    let map_y = |y: i32| -> i32 {
        let y = i64::from(y);
        if y <= strip_bottom {
            scaled(y, i64::from(strip_px), strip_bottom)
        } else {
            strip_px
                + scaled(
                    y - strip_bottom,
                    i64::from(height + gap - strip_px),
                    area_height - strip_bottom,
                )
        }
    };
    let mut fill = |rect: &PreviewRect, color: Color| {
        let (x0, x1) = (map_x(rect.x), map_x(rect.x + rect.w));
        let (y0, y1) = (map_y(rect.y), map_y(rect.y + rect.h));
        let (w, h) = (x1 - x0 - gap, y1 - y0 - gap);
        if w > 0 && h > 0 {
            painter.fill_rect(left + x0, top + y0, w, h, color);
        }
    };
    let faint = fg.with_alpha(0x70);
    // Tabs are in stack order; the first is the focused master's.
    for (index, tab) in glyph.tabs.iter().enumerate() {
        fill(tab, if index == 0 { fg } else { faint });
    }
    for window in &glyph.windows {
        fill(&window.rect, if window.index == 0 { fg } else { faint });
    }
}

/// How a workspace relates to the screens, as xmonad's pretty-printer sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WorkspaceState {
    /// Shown on the focused screen.
    Current,
    /// Shown on another screen.
    Visible,
    /// Not shown on any screen.
    Hidden,
}

/// One workspace as input to [`pp_segments`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceTag {
    pub name: String,
    pub state: WorkspaceState,
    pub has_windows: bool,
    pub urgent: bool,
}

/// xmonad `PP` settings; defaults are `myPP` (`xmobarPP` with
/// `ppCurrent = xmobarColor colorYellow "" . wrap "<" ">"` and
/// `ppTitle = xmobarColor colorGreen "" . shorten 100`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PpStyle {
    pub current_fg: Color,
    pub current_wrap: (String, String),
    pub visible_wrap: (String, String),
    /// `ppVisible`'s color; `None` uses [`BarStyle::fg`].
    pub visible_fg: Option<Color>,
    /// `ppHidden`'s color (hidden workspaces with windows); `None` uses
    /// [`BarStyle::fg`].
    pub hidden_fg: Option<Color>,
    /// Whether hidden workspaces without windows are shown
    /// (`ppHiddenNoWindows`); `myPP` drops them.
    pub show_hidden_empty: bool,
    /// Their color; `None` uses [`BarStyle::fg`].
    pub hidden_empty_fg: Option<Color>,
    /// `ppLayout`'s color, also the layout miniature's; `None` uses
    /// [`BarStyle::fg`].
    pub layout_fg: Option<Color>,
    /// Whether the layout's name is shown; without it only the layout
    /// miniature (if any) represents the layout.
    pub show_layout_name: bool,
    /// Whether workspace tags carry their name as [`Segment::action`], so
    /// clicking one views that workspace.
    pub clickable_workspaces: bool,
    pub urgent_fg: Color,
    pub urgent_bg: Color,
    pub title_fg: Color,
    /// `shorten n`: titles of `n` or more characters are cut to `n - 3`
    /// characters plus `...`.
    pub title_max_chars: usize,
    /// `ppSep`, between the workspaces, layout and title sections.
    pub sep: String,
    /// `ppWsSep`, between workspace tags.
    pub ws_sep: String,
}

impl Default for PpStyle {
    fn default() -> Self {
        Self {
            current_fg: Color::hex("#F8E63A"),
            current_wrap: ("<".to_owned(), ">".to_owned()),
            visible_wrap: ("(".to_owned(), ")".to_owned()),
            visible_fg: None,
            hidden_fg: None,
            show_hidden_empty: false,
            hidden_empty_fg: None,
            layout_fg: None,
            show_layout_name: true,
            clickable_workspaces: true,
            urgent_fg: Color::hex("#FF0000"),
            urgent_bg: Color::hex("#FFFF00"),
            title_fg: Color::hex("#86BD47"),
            title_max_chars: 100,
            sep: " : ".to_owned(),
            ws_sep: " ".to_owned(),
        }
    }
}

/// xmonad's `dynamicLogString`: workspace tags, layout name and window
/// title, joined by `ppSep`, with empty sections dropped. Hidden workspaces
/// without windows are omitted (`ppHiddenNoWindows = const ""`) unless
/// [`PpStyle::show_hidden_empty`] is set; urgency takes precedence over
/// every other state. Each tag segment's
/// [`Segment::action`] is the workspace name, unless
/// [`PpStyle::clickable_workspaces`] is off. A `layout_glyph` is drawn
/// right before the layout name, or alone without
/// [`PpStyle::show_layout_name`].
#[must_use]
pub fn pp_segments(
    tags: &[WorkspaceTag],
    layout: &str,
    layout_glyph: Option<LayoutGlyph>,
    title: &str,
    pp: &PpStyle,
) -> Vec<Segment> {
    let mut workspaces = Vec::new();
    for tag in tags {
        let segment = if tag.urgent {
            Segment::new(&tag.name).fg(pp.urgent_fg).bg(pp.urgent_bg)
        } else {
            match tag.state {
                WorkspaceState::Current => {
                    let (open, close) = &pp.current_wrap;
                    Segment::new(format!("{open}{}{close}", tag.name)).fg(pp.current_fg)
                }
                WorkspaceState::Visible => {
                    let (open, close) = &pp.visible_wrap;
                    with_fg(
                        Segment::new(format!("{open}{}{close}", tag.name)),
                        pp.visible_fg,
                    )
                }
                WorkspaceState::Hidden if tag.has_windows => {
                    with_fg(Segment::new(&tag.name), pp.hidden_fg)
                }
                WorkspaceState::Hidden if pp.show_hidden_empty => {
                    with_fg(Segment::new(&tag.name), pp.hidden_empty_fg)
                }
                WorkspaceState::Hidden => continue,
            }
        };
        if !workspaces.is_empty() {
            workspaces.push(Segment::new(&pp.ws_sep));
        }
        workspaces.push(if pp.clickable_workspaces {
            segment.action(&tag.name)
        } else {
            segment
        });
    }

    let mut layout_section = Vec::new();
    if let Some(glyph) = layout_glyph {
        layout_section.push(with_fg(Segment::with_layout(glyph), pp.layout_fg));
        if pp.show_layout_name {
            layout_section.push(Segment::new(" "));
        }
    }
    if pp.show_layout_name {
        layout_section.push(with_fg(Segment::new(layout), pp.layout_fg));
    }

    let title = shorten(title, pp.title_max_chars);
    let sections = [
        workspaces,
        layout_section,
        vec![Segment::new(title).fg(pp.title_fg)],
    ];
    let non_empty = sections.into_iter().filter(|section| {
        section
            .iter()
            .any(|segment| segment.is_graphic() || !segment.text.is_empty())
    });
    join_groups(non_empty, &pp.sep)
}

/// `segment` in `fg`, if any.
fn with_fg(segment: Segment, fg: Option<Color>) -> Segment {
    match fg {
        Some(fg) => segment.fg(fg),
        None => segment,
    }
}

/// xmonad's `shorten`: strings of `max_chars` or more characters are cut to
/// `max_chars - 3` characters followed by `...`.
#[must_use]
pub fn shorten(text: &str, max_chars: usize) -> String {
    const END: &str = "...";
    if text.chars().count() < max_chars {
        return text.to_owned();
    }
    let kept: String = text
        .chars()
        .take(max_chars.saturating_sub(END.len()))
        .collect();
    kept + END
}

/// Whether the battery is charging, as reported by the power supply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BatteryStatus {
    Charging,
    Discharging,
    /// Full or idle on AC power.
    Full,
    Unknown,
}

/// A battery reading, as input to [`format_battery`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BatteryState {
    /// Charge left, 0–100.
    pub percent: u8,
    pub status: BatteryStatus,
    /// Estimated time until empty, while discharging.
    pub time_left: Option<Duration>,
}

/// xmobar `Run Battery` options; defaults are from `xmobarrc`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatteryStyle {
    /// Literal text of the `--template` before `<acstatus>`.
    pub prefix: String,
    /// `--Low`: percentages at or below this are `low`.
    pub low_threshold: u8,
    /// `--High`: percentages at or above this are `high`.
    pub high_threshold: u8,
    pub low: Color,
    pub normal: Color,
    pub high: Color,
    /// `-O`, shown while charging.
    pub charging_text: String,
    pub charging: Color,
    /// `-i`, shown when full / idle on AC.
    pub charged_text: String,
    pub charged: Color,
}

impl Default for BatteryStyle {
    fn default() -> Self {
        Self {
            prefix: "Batt: ".to_owned(),
            low_threshold: 10,
            high_threshold: 80,
            low: Color::hex("#F15250"),
            normal: Color::hex("#F8E63A"),
            high: Color::hex("#86BD47"),
            charging_text: "Charging".to_owned(),
            charging: Color::hex("#77B2F6"),
            charged_text: "Charged".to_owned(),
            charged: Color::hex("#86BD47"),
        }
    }
}

/// The battery module: `Batt: <acstatus>` where the status is the colored
/// charging / charged text, or, while discharging, `-o "<left>% (<timeleft>)"`
/// with `<left>` colored by the low/normal/high thresholds. The parenthesized
/// time is omitted when unknown.
#[must_use]
pub fn format_battery(state: BatteryState, style: &BatteryStyle) -> Vec<Segment> {
    let mut segments = vec![Segment::new(&style.prefix)];
    match state.status {
        BatteryStatus::Charging => {
            segments.push(Segment::new(&style.charging_text).fg(style.charging));
        }
        BatteryStatus::Full => {
            segments.push(Segment::new(&style.charged_text).fg(style.charged));
        }
        BatteryStatus::Discharging | BatteryStatus::Unknown => {
            let percent = state.percent.min(100);
            let color = if percent >= style.high_threshold {
                style.high
            } else if percent > style.low_threshold {
                style.normal
            } else {
                style.low
            };
            segments.push(Segment::new(percent.to_string()).fg(color));
            let rest = state.time_left.map_or_else(
                || "%".to_owned(),
                |left| {
                    let minutes = left.as_secs() / 60;
                    format!("% ({}:{:02})", minutes / 60, minutes % 60)
                },
            );
            segments.push(Segment::new(rest));
        }
    }
    segments
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(name: &str, state: WorkspaceState, has_windows: bool) -> WorkspaceTag {
        WorkspaceTag {
            name: name.to_owned(),
            state,
            has_windows,
            urgent: false,
        }
    }

    fn texts(segments: &[Segment]) -> String {
        segments
            .iter()
            .map(|segment| segment.text.as_str())
            .collect()
    }

    #[test]
    fn pp_matches_my_pp() {
        let tags = [
            tag("1", WorkspaceState::Hidden, true),
            tag("2", WorkspaceState::Current, false),
            tag("3", WorkspaceState::Hidden, false),
            tag("4", WorkspaceState::Visible, false),
        ];
        let segments = pp_segments(&tags, "Tall", None, "vim", &PpStyle::default());
        assert_eq!(texts(&segments), "1 <2> (4) : Tall : vim");
        let current = segments
            .iter()
            .find(|s| s.text == "<2>")
            .map(|s| (s.fg, s.action.clone()));
        assert_eq!(
            current,
            Some((Some(PpStyle::default().current_fg), Some("2".to_owned())))
        );
    }

    #[test]
    fn pp_puts_the_layout_glyph_before_the_name() {
        let tags = [tag("1", WorkspaceState::Current, false)];
        let glyph = LayoutGlyph {
            area: (320, 180),
            ..LayoutGlyph::default()
        };
        let segments = pp_segments(&tags, "Tall", Some(glyph.clone()), "", &PpStyle::default());
        assert_eq!(texts(&segments), "<1> :  Tall");
        assert_eq!(segments[2].layout.as_ref(), Some(&glyph));
        assert_eq!(segments[3].text, " ");
    }

    #[test]
    fn pp_without_clickable_workspaces_has_no_actions() {
        let tags = [
            tag("1", WorkspaceState::Current, false),
            tag("2", WorkspaceState::Hidden, true),
        ];
        let pp = PpStyle {
            clickable_workspaces: false,
            ..PpStyle::default()
        };
        let segments = pp_segments(&tags, "Tall", None, "x", &pp);
        assert_eq!(texts(&segments), "<1> 2 : Tall : x");
        assert!(segments.iter().all(|s| s.action.is_none()));
    }

    #[test]
    fn pp_without_the_layout_name_shows_only_the_glyph() {
        let tags = [tag("1", WorkspaceState::Current, false)];
        let glyph = LayoutGlyph {
            area: (320, 180),
            ..LayoutGlyph::default()
        };
        let pp = PpStyle {
            show_layout_name: false,
            ..PpStyle::default()
        };
        let segments = pp_segments(&tags, "Tall", Some(glyph.clone()), "x", &pp);
        assert_eq!(texts(&segments), "<1> :  : x");
        assert_eq!(segments[2].layout.as_ref(), Some(&glyph));
        // Neither glyph nor name: the layout section is dropped.
        let segments = pp_segments(&tags, "Tall", None, "x", &pp);
        assert_eq!(texts(&segments), "<1> : x");
    }

    #[test]
    fn layout_glyph_width_follows_the_aspect_ratio() {
        let glyph = |area| LayoutGlyph {
            area,
            ..LayoutGlyph::default()
        };
        assert_eq!(glyph((320, 180)).width(14), 25);
        assert_eq!(glyph((100, 100)).width(14), 14);
        assert_eq!(glyph((0, 0)).width(14), 14);
        let fonts = Fonts::embedded();
        let style = BarStyle::default();
        let view = BarView {
            segments_left: vec![Segment::with_layout(glyph((320, 180))).action("layout")],
            segments_right: Vec::new(),
        };
        let layout = layout_bar(&view, &style, &fonts, 400, 2);
        let start = f64::from(style.padding);
        let width = f64::from(25 + 2 * style.icon_spacing);
        assert_eq!(
            layout.segment_span(SegmentId {
                side: Side::Left,
                index: 0
            }),
            Some((start, start + width))
        );
    }

    #[test]
    fn layout_glyph_tabs_stay_visible() {
        // Tabbed's real preview: a 20-unit strip of four tabs over the
        // focused window, which would round away at bar size.
        let window = PreviewWindow {
            index: 0,
            rect: PreviewRect::new(0, 20, 320, 160),
        };
        let tabs = (0..4)
            .map(|i| PreviewRect::new(i * 80, 0, 80, 20))
            .collect();
        let full = LayoutGlyph {
            area: (320, 180),
            windows: vec![window],
            tabs: Vec::new(),
        };
        let tabbed = LayoutGlyph {
            tabs,
            ..full.clone()
        };
        let fonts = Fonts::embedded();
        let style = BarStyle::default();
        let render = |glyph| {
            let view = BarView {
                segments_left: vec![Segment::with_layout(glyph)],
                segments_right: Vec::new(),
            };
            render_bar(&view, &style, &fonts, 60, 1)
        };
        let (full, tabbed) = (render(full), render(tabbed));
        // The glyph is 14 px high in a 24 px bar, so it starts at row 5;
        // the strip takes the first four rows, then a gap.
        let x = style.padding + style.icon_spacing + 1;
        let pixel = |canvas: &Canvas, y| canvas.pixel(x, y);
        assert_ne!(pixel(&tabbed, 6), pixel(&tabbed, 9), "gap under the tabs");
        assert_eq!(pixel(&full, 6), pixel(&full, 9), "full is one block");
        assert_eq!(pixel(&tabbed, 6), pixel(&full, 6), "focused tab is bright");
    }

    #[test]
    fn pp_colors_every_workspace_state_and_the_layout() {
        let pp = PpStyle {
            visible_fg: Some(Color::hex("#000001")),
            hidden_fg: Some(Color::hex("#000002")),
            hidden_empty_fg: Some(Color::hex("#000003")),
            layout_fg: Some(Color::hex("#000004")),
            ..PpStyle::default()
        };
        let tags = [
            tag("1", WorkspaceState::Hidden, true),
            tag("2", WorkspaceState::Current, false),
            tag("3", WorkspaceState::Hidden, false),
            tag("4", WorkspaceState::Visible, false),
        ];
        let glyph = LayoutGlyph::default();
        let fg = |segments: &[Segment], text: &str| {
            segments
                .iter()
                .find(|segment| segment.text == text)
                .and_then(|segment| segment.fg)
        };
        let segments = pp_segments(&tags, "Tall", Some(glyph.clone()), "vim", &pp);
        assert_eq!(texts(&segments), "1 <2> (4) :  Tall : vim");
        assert_eq!(fg(&segments, "1"), pp.hidden_fg);
        assert_eq!(fg(&segments, "(4)"), pp.visible_fg);
        assert_eq!(fg(&segments, "Tall"), pp.layout_fg);
        let glyph_fg = segments
            .iter()
            .find(|s| s.layout.is_some())
            .and_then(|s| s.fg);
        assert_eq!(glyph_fg, pp.layout_fg);

        let shown = PpStyle {
            show_hidden_empty: true,
            ..pp
        };
        let segments = pp_segments(&tags, "Tall", None, "vim", &shown);
        assert_eq!(texts(&segments), "1 <2> 3 (4) : Tall : vim");
        assert_eq!(fg(&segments, "3"), shown.hidden_empty_fg);
    }

    #[test]
    fn pp_drops_empty_title_section() {
        let tags = [tag("1", WorkspaceState::Current, false)];
        assert_eq!(
            texts(&pp_segments(&tags, "Full", None, "", &PpStyle::default())),
            "<1> : Full"
        );
    }

    #[test]
    fn urgent_overrides_hidden_empty() {
        let mut urgent = tag("9", WorkspaceState::Hidden, false);
        urgent.urgent = true;
        let segments = pp_segments(&[urgent], "Full", None, "", &PpStyle::default());
        assert_eq!(segments[0].bg, Some(PpStyle::default().urgent_bg));
    }

    #[test]
    fn shorten_matches_xmonad() {
        assert_eq!(shorten("abcd", 5), "abcd");
        assert_eq!(shorten("abcde", 5), "ab...");
    }

    #[test]
    fn battery_thresholds() {
        let style = BatteryStyle::default();
        let color_at = |percent| {
            let state = BatteryState {
                percent,
                status: BatteryStatus::Discharging,
                time_left: None,
            };
            format_battery(state, &style)[1].fg
        };
        assert_eq!(color_at(10), Some(style.low));
        assert_eq!(color_at(11), Some(style.normal));
        assert_eq!(color_at(79), Some(style.normal));
        assert_eq!(color_at(80), Some(style.high));
    }

    #[test]
    fn battery_time_left() {
        let state = BatteryState {
            percent: 42,
            status: BatteryStatus::Discharging,
            time_left: Some(Duration::from_secs(3 * 3600 + 7 * 60 + 59)),
        };
        assert_eq!(
            texts(&format_battery(state, &BatteryStyle::default())),
            "Batt: 42% (3:07)"
        );
    }

    #[test]
    fn hit_test_finds_segments() {
        let fonts = Fonts::embedded();
        let style = BarStyle::default();
        let view = BarView {
            segments_left: vec![Segment::new("1"), Segment::new(" "), Segment::new("2")],
            segments_right: vec![Segment::new("date")],
        };
        let char_width = f64::from(fonts.text_width("x", style.font_size));
        let layout = layout_bar(&view, &style, &fonts, 400, 1);
        let padding = f64::from(style.padding);
        let left = |index| {
            Some(SegmentId {
                side: Side::Left,
                index,
            })
        };
        assert_eq!(layout.segment_at(padding + 0.5), left(0));
        assert_eq!(layout.segment_at(padding + 2.5 * char_width), left(2));
        assert_eq!(layout.segment_at(0.0), None);
        assert_eq!(
            layout.segment_at(399.0 - padding),
            Some(SegmentId {
                side: Side::Right,
                index: 0
            })
        );
        let scaled = layout_bar(&view, &style, &fonts, 400, 2);
        assert_eq!(scaled.segment_at(padding + 2.5 * char_width), left(2));
    }

    fn icon() -> Icon {
        Icon::from_straight_rgba(4, 2, &[255; 4 * 2 * 4]).expect("valid size")
    }

    #[test]
    fn icon_segments_are_padded_squares() {
        let fonts = Fonts::embedded();
        let style = BarStyle::default();
        let view = BarView {
            segments_left: vec![Segment::with_icon(icon()).action("left")],
            segments_right: vec![
                Segment::with_icon(icon()).action("a"),
                Segment::with_icon(icon()).action("b"),
                Segment::new("date"),
            ],
        };
        let right = |index| SegmentId {
            side: Side::Right,
            index,
        };
        let icon_width = f64::from(style.icon_size + 2 * style.icon_spacing);
        let padding = f64::from(style.padding);
        for scale in [1, 2] {
            let layout = layout_bar(&view, &style, &fonts, 400, scale);
            let device_size = style.font_size * to_f32(to_i32(scale));
            let date_width = f64::from(fonts.text_width("date", device_size)) / f64::from(scale);
            let date_start = 400.0 - padding - date_width;
            assert_eq!(
                layout.segment_span(right(1)),
                Some((date_start - icon_width, date_start))
            );
            assert_eq!(
                layout.segment_span(right(0)),
                Some((date_start - 2.0 * icon_width, date_start - icon_width))
            );
            assert_eq!(layout.segment_at(date_start - 1.0), Some(right(1)));
            assert_eq!(
                layout.segment_at(date_start - icon_width - 1.0),
                Some(right(0))
            );
            assert_eq!(
                layout.segment_span(SegmentId {
                    side: Side::Left,
                    index: 0
                }),
                Some((padding, padding + icon_width))
            );
        }
    }

    #[test]
    fn left_icons_that_do_not_fit_are_dropped() {
        let fonts = Fonts::embedded();
        let style = BarStyle::default();
        let view = BarView {
            segments_left: vec![Segment::with_icon(icon()); 3],
            segments_right: Vec::new(),
        };
        let layout = layout_bar(&view, &style, &fonts, 60, 1);
        let id = |index| SegmentId {
            side: Side::Left,
            index,
        };
        assert!(layout.segment_span(id(1)).is_some());
        assert_eq!(layout.segment_span(id(2)), None);
    }

    #[test]
    fn icons_are_centered_in_their_square() {
        let fonts = Fonts::embedded();
        let style = BarStyle {
            padding: 0,
            ..BarStyle::default()
        };
        let view = BarView {
            segments_left: vec![Segment::with_icon(icon())],
            segments_right: Vec::new(),
        };
        // The 4×2 icon becomes 16×8, at x 4..20 and y 8..16.
        let canvas = render_bar(&view, &style, &fonts, 40, 1);
        let white = Some([255; 4]);
        assert_eq!(canvas.pixel(4, 8), white);
        assert_eq!(canvas.pixel(19, 15), white);
        assert_ne!(canvas.pixel(3, 8), white);
        assert_ne!(canvas.pixel(20, 8), white);
        assert_ne!(canvas.pixel(4, 7), white);
        assert_ne!(canvas.pixel(4, 16), white);
    }
}
