//! The layout picker: a grid of cards, each a small picture of a layout
//! (drawn like the pictures in `docs/layouts/`) above its name.

use crate::{
    Antialias,
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

/// A rectangle in a layout preview's coordinate space.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct PreviewRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl PreviewRect {
    #[must_use]
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }
}

/// One window of a layout preview.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PreviewWindow {
    /// The window's stack index; 0 is the master and focused window.
    pub index: usize,
    pub rect: PreviewRect,
}

/// One layout: its name and what it does with a few windows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PickerEntry {
    pub name: String,
    pub windows: Vec<PreviewWindow>,
    /// The tab strip's tabs in stack order (empty without a strip).
    pub tabs: Vec<PreviewRect>,
}

/// What the layout picker shows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LayoutPickerView {
    /// Shown top left, e.g. `"Layout of workspace 1"`.
    pub title: String,
    /// Shown top right in the dim color, e.g. the keys.
    pub hint: String,
    /// The area the previews' rectangles are relative to (width, height).
    pub preview_size: (u32, u32),
    pub entries: Vec<PickerEntry>,
    /// Index of the highlighted entry.
    pub selected: usize,
    /// Index of the layout currently in use (marked subtly).
    pub active: usize,
}

/// Picker colors and metrics, in logical pixels. The panel follows the
/// prompt's colors; the previews use the colors of the docs pictures.
#[derive(Debug, Clone, PartialEq)]
pub struct LayoutPickerStyle {
    pub bg: Color,
    pub fg: Color,
    /// The hint text and the active layout's marker.
    pub dim: Color,
    pub border: Color,
    pub border_width: u32,
    /// Background of a card that is not selected.
    pub card_bg: Color,
    /// Background of the selected card.
    pub bg_highlight: Color,
    /// Name of the selected card.
    pub fg_highlight: Color,
    /// The previewed screen.
    pub screen_bg: Color,
    pub screen_outline: Color,
    /// A previewed window.
    pub window_fill: Color,
    /// The previewed master (focused) window.
    pub master_fill: Color,
    pub window_stroke: Color,
    pub window_text: Color,
    /// Font size of titles and names in logical pixels.
    pub font_size: f32,
    /// Glyph edge antialiasing.
    pub antialias: Antialias,
    /// Most cards per row.
    pub max_columns: u32,
    /// Preferred preview width; smaller screens shrink it.
    pub preview_width: u32,
    /// Space around the grid and between cards.
    pub padding: u32,
    /// Height of the title row and of each card's name.
    pub line_height: u32,
}

impl Default for LayoutPickerStyle {
    fn default() -> Self {
        Self {
            bg: Color::hex("#161616"),
            fg: Color::hex("#F2F2F2"),
            dim: Color::hex("#969694"),
            border: Color::hex("#F2F2F2"),
            border_width: 1,
            card_bg: Color::hex("#242424"),
            bg_highlight: Color::hex("#1565C1"),
            fg_highlight: Color::hex("#F2F2F2"),
            screen_bg: Color::hex("#f4f4f4"),
            screen_outline: Color::hex("#888888"),
            window_fill: Color::hex("#dddddd"),
            master_fill: Color::hex("#9ec5f0"),
            window_stroke: Color::hex("#444444"),
            window_text: Color::hex("#222222"),
            font_size: 16.0,
            antialias: Antialias::Grayscale,
            max_columns: 5,
            preview_width: 224,
            padding: 16,
            line_height: 30,
        }
    }
}

/// Where everything goes, in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Metrics {
    columns: i32,
    width: i32,
    height: i32,
    preview_w: i32,
    preview_h: i32,
    card_w: i32,
    card_h: i32,
    /// Inner padding of a card around its preview.
    inset: i32,
    gap: i32,
    /// Top of the first row of cards.
    grid_top: i32,
}

/// The smallest preview width the picker shrinks to.
const MIN_PREVIEW_WIDTH: i32 = 64;

fn metrics(view: &LayoutPickerView, style: &LayoutPickerStyle, max_w: u32, max_h: u32) -> Metrics {
    let count = to_i32(view.entries.len()).max(1);
    let columns = to_i32(style.max_columns).clamp(1, count);
    let rows = (count + columns - 1) / columns;
    let pad = to_i32(style.padding);
    let gap = pad * 3 / 4;
    let inset = pad / 2;
    let line = to_i32(style.line_height);
    let (aspect_w, aspect_h) = (
        to_i32(view.preview_size.0).max(1),
        to_i32(view.preview_size.1).max(1),
    );
    // Leave some of the screen around the picker.
    let (avail_w, avail_h) = (to_i32(max_w) * 19 / 20, to_i32(max_h) * 19 / 20);
    let fixed_w = 2 * pad + columns * 2 * inset + (columns - 1) * gap;
    let from_width = (avail_w - fixed_w) / columns;
    let fixed_h = 2 * pad + line + gap + rows * (2 * inset + line) + (rows - 1) * gap;
    let from_height = (avail_h - fixed_h) / rows * aspect_w / aspect_h;
    let preview_w = to_i32(style.preview_width)
        .min(from_width)
        .min(from_height)
        .max(MIN_PREVIEW_WIDTH);
    let preview_h = preview_w * aspect_h / aspect_w;
    let card_w = preview_w + 2 * inset;
    let card_h = inset + preview_h + line;
    let grid_top = pad + line + gap / 2;
    Metrics {
        columns,
        width: 2 * pad + columns * card_w + (columns - 1) * gap,
        height: grid_top + rows * card_h + (rows - 1) * gap + pad,
        preview_w,
        preview_h,
        card_w,
        card_h,
        inset,
        gap,
        grid_top,
    }
}

impl Metrics {
    /// The logical rectangle `(x, y, w, h)` of card `index` of `count`; a
    /// short last row is centred.
    fn card(&self, index: usize, count: usize, pad: i32) -> (i32, i32, i32, i32) {
        let (index, count) = (to_i32(index), to_i32(count));
        let (row, column) = (index / self.columns, index % self.columns);
        let in_row = (count - row * self.columns).min(self.columns);
        let offset = (self.columns - in_row) * (self.card_w + self.gap) / 2;
        (
            pad + offset + column * (self.card_w + self.gap),
            self.grid_top + row * (self.card_h + self.gap),
            self.card_w,
            self.card_h,
        )
    }
}

/// The logical size of the picker for `view` on a screen of at most
/// `max_width` × `max_height` logical pixels.
#[must_use]
pub fn layout_picker_size(
    view: &LayoutPickerView,
    style: &LayoutPickerStyle,
    max_width: u32,
    max_height: u32,
) -> (u32, u32) {
    let m = metrics(view, style, max_width, max_height);
    (m.width.unsigned_abs(), m.height.unsigned_abs())
}

/// Every entry's card as a logical `(x, y, w, h)` within the picker.
#[must_use]
pub fn picker_entry_rects(
    view: &LayoutPickerView,
    style: &LayoutPickerStyle,
    max_width: u32,
    max_height: u32,
) -> Vec<(i32, i32, i32, i32)> {
    let m = metrics(view, style, max_width, max_height);
    let pad = to_i32(style.padding);
    (0..view.entries.len())
        .map(|index| m.card(index, view.entries.len(), pad))
        .collect()
}

/// The entry whose card contains the logical point (`x`, `y`) of the
/// picker, if any.
#[must_use]
pub fn picker_entry_at(
    view: &LayoutPickerView,
    style: &LayoutPickerStyle,
    max_width: u32,
    max_height: u32,
    x: f64,
    y: f64,
) -> Option<usize> {
    picker_entry_rects(view, style, max_width, max_height)
        .into_iter()
        .position(|(cx, cy, w, h)| {
            x >= f64::from(cx)
                && x < f64::from(cx + w)
                && y >= f64::from(cy)
                && y < f64::from(cy + h)
        })
}

/// Renders the picker at `scale`; the canvas is [`layout_picker_size`] ×
/// `scale`.
#[must_use]
pub fn render_layout_picker(
    view: &LayoutPickerView,
    style: &LayoutPickerStyle,
    fonts: &Fonts,
    max_width: u32,
    max_height: u32,
    scale: u32,
) -> Canvas {
    let grid = metrics(view, style, max_width, max_height);
    let mut painter = Painter::new(grid.width.unsigned_abs(), grid.height.unsigned_abs(), scale)
        .with_antialias(style.antialias);
    let unit = to_i32(painter.scale());
    let (canvas_w, canvas_h) = (painter.width(), painter.height());
    let size = style.font_size * to_f32(unit);
    let pad = to_i32(style.padding);
    let line = to_i32(style.line_height) * unit;
    painter.fill_rect(0, 0, canvas_w, canvas_h, style.bg);
    painter.stroke_rect(
        0,
        0,
        canvas_w,
        canvas_h,
        to_i32(style.border_width) * unit,
        style.border,
    );

    // Title and hint.
    let (left, right) = (pad * unit, canvas_w - pad * unit);
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

    for (index, entry) in view.entries.iter().enumerate() {
        let card = grid.card(index, view.entries.len(), pad);
        let (x, y, w, h) = (card.0 * unit, card.1 * unit, card.2 * unit, card.3 * unit);
        let selected = index == view.selected;
        let radius = to_f32(grid.inset * unit) * 0.75;
        let card_fill = if selected {
            style.bg_highlight
        } else {
            style.card_bg
        };
        let marker = (index == view.active).then_some((
            if selected {
                style.fg_highlight
            } else {
                style.dim
            },
            to_f32(unit),
        ));
        painter.rounded_rect(
            (to_f32(x), to_f32(y), to_f32(w), to_f32(h)),
            radius,
            Some(card_fill),
            marker,
        );
        let inset = grid.inset * unit;
        let preview = (
            x + inset,
            y + inset,
            grid.preview_w * unit,
            grid.preview_h * unit,
        );
        draw_preview(&mut painter, view, entry, style, fonts, preview);

        let name_color = if selected {
            style.fg_highlight
        } else {
            style.fg
        };
        let room = to_f32(w - 2 * inset);
        let name = fonts.ellipsize(&entry.name, size, room);
        let name_x = to_f32(x) + (to_f32(w) - fonts.text_width(&name, size)) / 2.0;
        painter.draw_text(
            fonts,
            &name,
            name_x,
            y + inset + grid.preview_h * unit,
            line,
            size,
            name_color,
            (x + inset, x + w - inset),
        );
    }
    painter.finish()
}

/// One layout'unit picture in the device-pixel rectangle `area`: the screen,
/// the tab strip and the numbered windows, like `docs/layouts/*.svg`.
fn draw_preview(
    painter: &mut Painter,
    view: &LayoutPickerView,
    entry: &PickerEntry,
    style: &LayoutPickerStyle,
    fonts: &Fonts,
    area: (i32, i32, i32, i32),
) {
    let (x, y, w, h) = area;
    let unit = to_f32(to_i32(painter.scale()));
    let (preview_w, preview_h) = (
        to_f32(to_i32(view.preview_size.0).max(1)),
        to_f32(to_i32(view.preview_size.1).max(1)),
    );
    let (kx, ky) = (to_f32(w) / preview_w, to_f32(h) / preview_h);
    // Snap each edge to the device pixel grid so neighbours line up.
    let map = |r: PreviewRect| {
        let x0 = x + round_i32(to_f32(r.x) * kx);
        let y0 = y + round_i32(to_f32(r.y) * ky);
        let x1 = x + round_i32(to_f32(r.x + r.w) * kx);
        let y1 = y + round_i32(to_f32(r.y + r.h) * ky);
        (x0, y0, x1 - x0, y1 - y0)
    };
    painter.fill_rect(x, y, w, h, style.screen_bg);
    painter.stroke_rect(x, y, w, h, to_i32(painter.scale()), style.screen_outline);

    let number_size = (style.font_size * 0.8 * unit).round();
    for (index, &tab) in entry.tabs.iter().enumerate() {
        let (tx, ty, tw, th) = map(tab);
        let fill = if index == 0 {
            style.master_fill
        } else {
            style.window_fill
        };
        painter.fill_rect(tx, ty, tw, th, fill);
        painter.stroke_rect(tx, ty, tw, th, to_i32(painter.scale()), style.window_stroke);
        let size = number_size.min((to_f32(th) * 0.8).round());
        draw_centered(
            painter,
            fonts,
            &(index + 1).to_string(),
            (tx, ty, tw, th),
            size,
            style.window_text,
        );
    }

    // The docs pictures leave a 2px gap (of 320) around each window.
    let gap = (2.0 * kx).max(unit);
    let stroke = (1.5 * kx).max(unit).round();
    for window in &entry.windows {
        let (wx, wy, ww, wh) = map(window.rect);
        let fill = if window.index == 0 {
            style.master_fill
        } else {
            style.window_fill
        };
        let rect = (
            to_f32(wx) + gap,
            to_f32(wy) + gap,
            to_f32(ww) - 2.0 * gap,
            to_f32(wh) - 2.0 * gap,
        );
        painter.rounded_rect(
            rect,
            (3.0 * kx).max(2.0 * unit),
            Some(fill),
            Some((style.window_stroke, stroke)),
        );
        let size = number_size.min((to_f32(wh) * 0.6).round());
        draw_centered(
            painter,
            fonts,
            &(window.index + 1).to_string(),
            (wx, wy, ww, wh),
            size,
            style.window_text,
        );
    }
}

/// `text` centred in the device-pixel rectangle `(x, y, w, h)`.
fn draw_centered(
    painter: &mut Painter,
    fonts: &Fonts,
    text: &str,
    (x, y, w, h): (i32, i32, i32, i32),
    size: f32,
    color: Color,
) {
    if size < 1.0 {
        return;
    }
    let pen = to_f32(x) + (to_f32(w) - fonts.text_width(text, size)) / 2.0;
    painter.draw_text(fonts, text, pen, y, h, size, color, (x, x + w));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(count: usize) -> LayoutPickerView {
        LayoutPickerView {
            preview_size: (320, 180),
            entries: (0..count)
                .map(|i| PickerEntry {
                    name: format!("Layout {i}"),
                    ..PickerEntry::default()
                })
                .collect(),
            ..LayoutPickerView::default()
        }
    }

    #[test]
    fn preferred_size_on_a_large_screen() {
        let style = LayoutPickerStyle::default();
        let (w, h) = layout_picker_size(&view(10), &style, 1920, 1080);
        // 5 cards of 224 + 2 * 8, 4 gaps of 12, 16 padding each side.
        assert_eq!(w, 2 * 16 + 5 * 240 + 4 * 12);
        // Title row 30 + 6, two rows of 8 + 126 + 30 and a gap, padding.
        assert_eq!(h, 16 + 30 + 6 + 2 * (8 + 126 + 30) + 12 + 16);
    }

    #[test]
    fn shrinks_to_fit_small_screens() {
        let style = LayoutPickerStyle::default();
        let (w, h) = layout_picker_size(&view(10), &style, 800, 600);
        assert!(w <= 800 * 19 / 20, "{w}");
        assert!(h <= 600 * 19 / 20, "{h}");
        let (_, h) = layout_picker_size(&view(10), &style, 1920, 400);
        assert!(h <= 400 * 19 / 20, "{h}");
    }

    #[test]
    fn hit_test_matches_cards() {
        let style = LayoutPickerStyle::default();
        let v = view(7);
        let rects = picker_entry_rects(&v, &style, 1920, 1080);
        assert_eq!(rects.len(), 7);
        for (index, &(x, y, w, h)) in rects.iter().enumerate() {
            let (cx, cy) = (f64::from(x + w / 2), f64::from(y + h / 2));
            assert_eq!(picker_entry_at(&v, &style, 1920, 1080, cx, cy), Some(index));
        }
        // The short second row (2 of 5) is centred.
        let (first, sixth) = (rects[0], rects[5]);
        assert_eq!(sixth.0 - first.0, 3 * (first.2 + 12) / 2);
        assert_eq!(picker_entry_at(&v, &style, 1920, 1080, 1.0, 1.0), None);
        assert_eq!(picker_entry_at(&v, &style, 1920, 1080, 20.0, 20.0), None);
    }
}
