//! The key binding help overlay: every binding as a key column and an
//! action column under section headings, flowing into as many columns as
//! the screen is wide and scrolling when it still does not fit.

use crate::{
    Antialias,
    Canvas,
    Color,
    Fonts,
    canvas::{
        Painter,
        to_f32,
        to_i32,
    },
};

/// One binding: its combination and what it does.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HelpBinding {
    /// E.g. `"Super+Shift+Return"`.
    pub combo: String,
    /// E.g. `"spawn-terminal"`.
    pub action: String,
}

/// A titled group of bindings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HelpGroup {
    pub title: String,
    pub bindings: Vec<HelpBinding>,
}

/// What the help overlay shows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HelpView {
    /// Shown top left, e.g. `"Key bindings"`.
    pub title: String,
    /// Shown top right in the dim color, e.g. the keys.
    pub hint: String,
    pub groups: Vec<HelpGroup>,
    /// The first visible row (clamped to what can scroll).
    pub scroll: usize,
}

/// Help overlay colors and metrics, in logical pixels. The panel follows
/// the layout picker's.
#[derive(Debug, Clone, PartialEq)]
pub struct HelpStyle {
    pub bg: Color,
    /// The title and the key column.
    pub fg: Color,
    /// The hint text and the scroll bar.
    pub dim: Color,
    /// The action column.
    pub action_fg: Color,
    /// Section headings.
    pub heading: Color,
    /// The line under the title.
    pub rule: Color,
    pub border: Color,
    pub border_width: u32,
    /// Font size in logical pixels.
    pub font_size: f32,
    /// Glyph edge antialiasing.
    pub antialias: Antialias,
    /// Space around the content.
    pub padding: u32,
    /// Height of the title row.
    pub line_height: u32,
    /// Height of a binding or heading row.
    pub row_height: u32,
    /// Space between the key and the action column.
    pub key_gap: u32,
    /// Space between two columns of bindings.
    pub column_gap: u32,
    /// Longer actions (e.g. spawn commands) are ellipsized to this width.
    pub max_action_width: u32,
}

impl Default for HelpStyle {
    fn default() -> Self {
        Self {
            bg: Color::hex("#161616"),
            fg: Color::hex("#F2F2F2"),
            dim: Color::hex("#969694"),
            action_fg: Color::hex("#C8C8C6"),
            heading: Color::hex("#F8E63A"),
            rule: Color::hex("#3A3A3A"),
            border: Color::hex("#F2F2F2"),
            border_width: 1,
            font_size: 16.0,
            antialias: Antialias::Grayscale,
            padding: 16,
            line_height: 30,
            row_height: 22,
            key_gap: 16,
            column_gap: 40,
            max_action_width: 320,
        }
    }
}

/// One row of a column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row {
    /// Group `.0`'s heading.
    Heading(usize),
    /// Binding `.1` of group `.0`.
    Binding(usize, usize),
    /// Space between two groups.
    Gap,
}

/// Where everything goes, in logical pixels.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Metrics {
    columns: Vec<Vec<Row>>,
    /// Rows shown at once.
    visible: usize,
    /// The largest useful scroll offset.
    max_scroll: usize,
    width: i32,
    height: i32,
    key_w: i32,
    action_w: i32,
    column_w: i32,
    /// Top of the first row.
    grid_top: i32,
}

/// Fewest rows per column [`pack`] uses: a heading and two bindings.
const MIN_COLUMN_ROWS: usize = 3;

/// Flows the groups (given by their binding counts) into columns of at
/// most `limit` rows. A group starts a new column unless its heading and
/// its first two bindings fit below the previous group; longer groups
/// continue in the next column.
fn pack(groups: &[usize], limit: usize) -> Vec<Vec<Row>> {
    let limit = limit.max(MIN_COLUMN_ROWS);
    let mut columns: Vec<Vec<Row>> = vec![Vec::new()];
    for (group, &count) in groups.iter().enumerate() {
        let column = columns.last_mut().expect("never empty");
        if !column.is_empty() {
            if column.len() + 2 + count.min(2) <= limit {
                column.push(Row::Gap);
            } else {
                columns.push(Vec::new());
            }
        }
        let rows = std::iter::once(Row::Heading(group))
            .chain((0..count).map(|binding| Row::Binding(group, binding)));
        for row in rows {
            if columns.last().is_some_and(|c| c.len() >= limit) {
                columns.push(Vec::new());
            }
            columns.last_mut().expect("never empty").push(row);
        }
    }
    columns
}

/// The shortest column length that fits the groups into `columns`
/// columns, and the packing.
fn shortest_packing(groups: &[usize], columns: usize) -> Vec<Vec<Row>> {
    let rows: usize = groups.iter().map(|count| count + 1).sum();
    // One column holds every row and the gaps between groups.
    let single = rows + groups.len();
    let mut limit = rows.div_ceil(columns.max(1)).max(MIN_COLUMN_ROWS);
    loop {
        let packed = pack(groups, limit);
        if packed.len() <= columns || limit >= single {
            return packed;
        }
        limit += 1;
    }
}

/// The logical width of `text`, measured at the device size for `scale`.
fn logical_width(fonts: &Fonts, text: &str, size: f32, scale: u32) -> i32 {
    let unit = to_f32(to_i32(scale.max(1)));
    let device = fonts.text_width(text, size * unit).ceil();
    #[expect(
        clippy::cast_possible_truncation,
        reason = "text widths are far below i32::MAX"
    )]
    let logical = (device / unit).ceil() as i32;
    logical
}

fn metrics(
    view: &HelpView,
    style: &HelpStyle,
    fonts: &Fonts,
    (max_w, max_h): (u32, u32),
    scale: u32,
) -> Metrics {
    let width_of = |text: &str| logical_width(fonts, text, style.font_size, scale);
    let bindings = view.groups.iter().flat_map(|g| &g.bindings);
    let key_w = bindings
        .clone()
        .map(|b| width_of(&b.combo))
        .chain(view.groups.iter().map(|g| width_of(&g.title)))
        .max()
        .unwrap_or(0);
    let pad = to_i32(style.padding);
    let key_gap = to_i32(style.key_gap);
    let gap = to_i32(style.column_gap);
    // Leave some of the screen around the overlay.
    let (avail_w, avail_h) = (to_i32(max_w) * 19 / 20, to_i32(max_h) * 19 / 20);
    let action_w = bindings
        .map(|b| width_of(&b.action))
        .max()
        .unwrap_or(0)
        .min(to_i32(style.max_action_width))
        .min(avail_w - 2 * pad - key_w - key_gap)
        .max(0);
    let column_w = key_w + key_gap + action_w;
    let max_columns = usize::try_from((avail_w - 2 * pad + gap) / (column_w + gap).max(1))
        .unwrap_or(0)
        .max(1);

    let line = to_i32(style.line_height);
    let row_h = to_i32(style.row_height).max(1);
    let grid_top = pad + line + pad / 2;
    let fit_rows = usize::try_from((avail_h - grid_top - pad) / row_h)
        .unwrap_or(0)
        .max(1);
    let counts: Vec<usize> = view.groups.iter().map(|g| g.bindings.len()).collect();
    // The fewest columns that show everything, else every column that fits
    // and a scrolling view.
    let columns = (1..=max_columns)
        .map(|count| shortest_packing(&counts, count))
        .find(|packed| packed.iter().all(|c| c.len() <= fit_rows))
        .unwrap_or_else(|| shortest_packing(&counts, max_columns));
    let rows = columns.iter().map(Vec::len).max().unwrap_or(0);
    let visible = rows.min(fit_rows);
    let count = to_i32(columns.len());
    Metrics {
        visible,
        max_scroll: rows - visible,
        width: 2 * pad + count * column_w + (count - 1) * gap,
        height: grid_top + to_i32(visible) * row_h + pad,
        columns,
        key_w,
        action_w,
        column_w,
        grid_top,
    }
}

/// The logical size of the overlay for `view` on a screen of at most
/// `max_width` × `max_height` logical pixels, laid out for `scale`.
#[must_use]
pub fn help_size(
    view: &HelpView,
    style: &HelpStyle,
    fonts: &Fonts,
    max_width: u32,
    max_height: u32,
    scale: u32,
) -> (u32, u32) {
    let m = metrics(view, style, fonts, (max_width, max_height), scale);
    (m.width.unsigned_abs(), m.height.unsigned_abs())
}

/// How many rows the overlay shows at once and the largest useful scroll
/// offset (0 when everything fits), for [`help_size`]'s layout.
#[must_use]
pub fn help_viewport(
    view: &HelpView,
    style: &HelpStyle,
    fonts: &Fonts,
    max_width: u32,
    max_height: u32,
    scale: u32,
) -> (usize, usize) {
    let m = metrics(view, style, fonts, (max_width, max_height), scale);
    (m.visible, m.max_scroll)
}

/// Renders the overlay at `scale`; the canvas is [`help_size`] × `scale`.
#[must_use]
pub fn render_help(
    view: &HelpView,
    style: &HelpStyle,
    fonts: &Fonts,
    max_width: u32,
    max_height: u32,
    scale: u32,
) -> Canvas {
    let m = metrics(view, style, fonts, (max_width, max_height), scale);
    let mut painter = Painter::new(m.width.unsigned_abs(), m.height.unsigned_abs(), scale)
        .with_antialias(style.antialias);
    let unit = to_i32(painter.scale());
    let (canvas_w, canvas_h) = (painter.width(), painter.height());
    let size = style.font_size * to_f32(unit);
    let pad = to_i32(style.padding);
    painter.fill_rect(0, 0, canvas_w, canvas_h, style.bg);
    painter.stroke_rect(
        0,
        0,
        canvas_w,
        canvas_h,
        to_i32(style.border_width) * unit,
        style.border,
    );

    draw_title(&mut painter, view, style, fonts);

    // The bindings.
    let scroll = view.scroll.min(m.max_scroll);
    let row_h = to_i32(style.row_height) * unit;
    let gap = to_i32(style.column_gap);
    for (index, column) in m.columns.iter().enumerate() {
        let x = (pad + to_i32(index) * (m.column_w + gap)) * unit;
        let action_x = x + (m.key_w + to_i32(style.key_gap)) * unit;
        let clip = (x, x + m.column_w * unit);
        let rows = column.iter().skip(scroll).take(m.visible);
        for (offset, row) in rows.enumerate() {
            let y = m.grid_top * unit + to_i32(offset) * row_h;
            match *row {
                Row::Heading(group) => painter.draw_text(
                    fonts,
                    &view.groups[group].title,
                    to_f32(x),
                    y,
                    row_h,
                    size,
                    style.heading,
                    clip,
                ),
                Row::Binding(group, binding) => {
                    let binding = &view.groups[group].bindings[binding];
                    painter.draw_text(
                        fonts,
                        &binding.combo,
                        to_f32(x),
                        y,
                        row_h,
                        size,
                        style.fg,
                        clip,
                    );
                    let action = fonts.ellipsize(&binding.action, size, to_f32(m.action_w * unit));
                    painter.draw_text(
                        fonts,
                        &action,
                        to_f32(action_x),
                        y,
                        row_h,
                        size,
                        style.action_fg,
                        clip,
                    );
                }
                Row::Gap => {}
            }
        }
    }

    if m.max_scroll > 0 {
        draw_scroll_bar(&mut painter, &m, style, scroll);
    }
    painter.finish()
}

/// The title, the hint and a rule below them.
fn draw_title(painter: &mut Painter, view: &HelpView, style: &HelpStyle, fonts: &Fonts) {
    let unit = to_i32(painter.scale());
    let size = style.font_size * to_f32(unit);
    let pad = to_i32(style.padding);
    let line = to_i32(style.line_height) * unit;
    let (left, right) = (pad * unit, painter.width() - pad * unit);
    let hint_w = fonts.text_width(&view.hint, size);
    let hint_x = to_f32(right) - hint_w;
    let title_room = (hint_x - to_f32(left) - size).max(0.0);
    let title = fonts.ellipsize(&view.title, size, title_room);
    painter.draw_text(
        fonts,
        &title,
        to_f32(left),
        pad * unit,
        line,
        size,
        style.fg,
        (left, right),
    );
    if to_f32(left) + fonts.text_width(&title, size) + size <= hint_x {
        painter.draw_text(
            fonts,
            &view.hint,
            hint_x,
            pad * unit,
            line,
            size,
            style.dim,
            (left, right),
        );
    }
    let rule_y = (pad + to_i32(style.line_height) + pad / 4) * unit;
    painter.fill_rect(left, rule_y, right - left, unit, style.rule);
}

/// A scroll bar in the right padding, for content scrolled by `scroll` of
/// `m.max_scroll` (which is positive) rows.
fn draw_scroll_bar(painter: &mut Painter, m: &Metrics, style: &HelpStyle, scroll: usize) {
    let unit = to_i32(painter.scale());
    let pad = to_i32(style.padding) * unit;
    let rows = to_i32(m.visible + m.max_scroll);
    let track_top = m.grid_top * unit;
    let track_h = to_i32(m.visible) * to_i32(style.row_height) * unit;
    let thumb_h = (track_h * to_i32(m.visible) / rows).max(2 * unit);
    let thumb_y = track_top + (track_h - thumb_h) * to_i32(scroll) / to_i32(m.max_scroll);
    let bar_w = 3 * unit;
    // Centred in the right padding.
    let bar_x = painter.width() - pad + (pad - bar_w) / 2;
    painter.fill_rect(bar_x, track_top, bar_w, track_h, style.rule);
    painter.fill_rect(bar_x, thumb_y, bar_w, thumb_h, style.dim);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(groups: &[usize]) -> HelpView {
        HelpView {
            title: "Key bindings".to_owned(),
            hint: "Esc: close".to_owned(),
            groups: groups
                .iter()
                .enumerate()
                .map(|(g, &count)| HelpGroup {
                    title: format!("Group {g}"),
                    bindings: (0..count)
                        .map(|b| HelpBinding {
                            combo: format!("Super+{b}"),
                            action: format!("action {g}.{b}"),
                        })
                        .collect(),
                })
                .collect(),
            scroll: 0,
        }
    }

    #[test]
    fn packing_keeps_headings_with_their_bindings() {
        use Row::{
            Binding as B,
            Gap,
            Heading as H,
        };
        assert_eq!(
            pack(&[2, 3], 10),
            [vec![
                H(0),
                B(0, 0),
                B(0, 1),
                Gap,
                H(1),
                B(1, 0),
                B(1, 1),
                B(1, 2)
            ]]
        );
        // The second heading and two bindings don't fit below the first
        // group: it starts a new column.
        assert_eq!(
            pack(&[2, 3], 5),
            [
                vec![H(0), B(0, 0), B(0, 1)],
                vec![H(1), B(1, 0), B(1, 1), B(1, 2)],
            ]
        );
        // A long group continues in the next column.
        assert_eq!(
            pack(&[5], 4),
            [
                vec![H(0), B(0, 0), B(0, 1), B(0, 2)],
                vec![B(0, 3), B(0, 4)]
            ]
        );
    }

    #[test]
    fn shortest_packing_balances_columns() {
        let packed = shortest_packing(&[10, 10], 2);
        assert_eq!(packed.len(), 2);
        assert_eq!(packed.iter().map(Vec::len).max(), Some(11));
        assert_eq!(shortest_packing(&[10, 10], 1).len(), 1);
        assert_eq!(shortest_packing(&[], 3).len(), 1);
    }

    #[test]
    fn fits_without_scrolling_on_a_large_screen() {
        let fonts = Fonts::embedded();
        let style = HelpStyle::default();
        let v = view(&[3, 11, 8, 23, 7, 6, 2]);
        let (w, h) = help_size(&v, &style, &fonts, 1920, 1080, 1);
        assert!(w <= 1920 * 19 / 20 && h <= 1080 * 19 / 20, "{w}x{h}");
        assert_eq!(help_viewport(&v, &style, &fonts, 1920, 1080, 1).1, 0);
        // Text is measured at the device size, so scale 2 may differ a bit.
        let (w2, h2) = help_size(&v, &style, &fonts, 1920, 1080, 2);
        assert!(w2.abs_diff(w) <= w / 10 && h2 == h, "{w2}x{h2}");
    }

    #[test]
    fn scrolls_on_a_small_screen() {
        let fonts = Fonts::embedded();
        let style = HelpStyle::default();
        let v = view(&[3, 11, 8, 23, 7, 6, 2]);
        let (w, h) = help_size(&v, &style, &fonts, 640, 400, 1);
        assert!(w <= 640 * 19 / 20 && h <= 400 * 19 / 20, "{w}x{h}");
        let (page, max_scroll) = help_viewport(&v, &style, &fonts, 640, 400, 1);
        assert!(page > 0 && max_scroll > 0, "{page} {max_scroll}");
        // Scrolled past the end renders like scrolled to the end.
        let end = HelpView {
            scroll: max_scroll,
            ..v.clone()
        };
        let past = HelpView {
            scroll: max_scroll + 5,
            ..v
        };
        assert_eq!(
            render_help(&end, &style, &fonts, 640, 400, 1).as_bytes(),
            render_help(&past, &style, &fonts, 640, 400, 1).as_bytes()
        );
    }
}
