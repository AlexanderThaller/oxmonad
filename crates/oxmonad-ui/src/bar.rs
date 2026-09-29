//! The status bar: xmobar's `%StdinReader% }{ %battery% | %date%` template
//! with the xmonad pretty-printer (`myPP`) on the left.

use std::time::Duration;

use crate::{
    Canvas,
    Color,
    Fonts,
    canvas::{
        Painter,
        round_i32,
        to_f32,
        to_i32,
    },
};

/// A run of text in one color, optionally on its own background.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub text: String,
    /// Text color; `None` uses [`BarStyle::fg`].
    pub fg: Option<Color>,
    /// Background behind the text; `None` leaves the bar background.
    pub bg: Option<Color>,
    /// What clicking the segment means to the binary (like xmobar's
    /// `<action=…>`); [`pp_segments`] sets it to the workspace tag.
    pub action: Option<String>,
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
        }
    }

    #[must_use]
    pub fn fg(mut self, color: Color) -> Self {
        self.fg = Some(color);
        self
    }

    #[must_use]
    pub fn bg(mut self, color: Color) -> Self {
        self.bg = Some(color);
        self
    }

    #[must_use]
    pub fn action(mut self, action: impl Into<String>) -> Self {
        self.action = Some(action.into());
        self
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
    pub height: u32,
    /// Space before the first and after the last segment.
    pub padding: u32,
    /// Put between right-side groups by [`join_groups`] (the template's
    /// literal `" | "`).
    pub separator: String,
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
/// ellipsized) text drawn there.
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
        .map(|segment| fonts.text_width(&segment.text, size))
        .sum();
    let right_start = (width - padding - right_width).max(padding);
    let mut pen = right_start;
    for (index, segment) in view.segments_right.iter().enumerate() {
        let segment_width = fonts.text_width(&segment.text, size);
        placed.push(Placed {
            id: SegmentId {
                side: Side::Right,
                index,
            },
            x: pen,
            width: segment_width,
            text: segment.text.clone(),
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
        let text = fonts.ellipsize(&segment.text, size, left_limit - pen);
        if text.is_empty() {
            break;
        }
        let segment_width = fonts.text_width(&text, size);
        let truncated = *text != *segment.text;
        placed.push(Placed {
            id: SegmentId {
                side: Side::Left,
                index,
            },
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
    let mut painter = Painter::new(width, style.height, scale);
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
/// without windows are omitted (`ppHiddenNoWindows = const ""`); urgency
/// takes precedence over every other state. Each tag segment's
/// [`Segment::action`] is the workspace name.
#[must_use]
pub fn pp_segments(tags: &[WorkspaceTag], layout: &str, title: &str, pp: &PpStyle) -> Vec<Segment> {
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
                    Segment::new(format!("{open}{}{close}", tag.name))
                }
                WorkspaceState::Hidden if tag.has_windows => Segment::new(&tag.name),
                WorkspaceState::Hidden => continue,
            }
        };
        if !workspaces.is_empty() {
            workspaces.push(Segment::new(&pp.ws_sep));
        }
        workspaces.push(segment.action(&tag.name));
    }

    let title = shorten(title, pp.title_max_chars);
    let sections = [
        workspaces,
        vec![Segment::new(layout)],
        vec![Segment::new(title).fg(pp.title_fg)],
    ];
    let non_empty = sections
        .into_iter()
        .filter(|section| section.iter().any(|segment| !segment.text.is_empty()));
    join_groups(non_empty, &pp.sep)
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
        let segments = pp_segments(&tags, "Tall", "vim", &PpStyle::default());
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
    fn pp_drops_empty_title_section() {
        let tags = [tag("1", WorkspaceState::Current, false)];
        assert_eq!(
            texts(&pp_segments(&tags, "Full", "", &PpStyle::default())),
            "<1> : Full"
        );
    }

    #[test]
    fn urgent_overrides_hidden_empty() {
        let mut urgent = tag("9", WorkspaceState::Hidden, false);
        urgent.urgent = true;
        let segments = pp_segments(&[urgent], "Full", "", &PpStyle::default());
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
}
