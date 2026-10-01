//! Popup context menus (e.g. a tray item's menu): a bordered column of
//! rows with optional check / radio marks and submenu arrows.

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

/// What kind of row a [`MenuEntry`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MenuEntryKind {
    /// A plain item.
    Normal,
    /// A thin horizontal line between groups; never selectable.
    Separator,
    /// A checkbox item, checked when `true`.
    Check(bool),
    /// A radio item, selected when `true`.
    Radio(bool),
}

/// One row of a menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuEntry {
    /// Plain text (mnemonics already stripped); ignored for separators.
    pub label: String,
    pub kind: MenuEntryKind,
    /// Disabled entries are drawn dimmed and are never hit.
    pub enabled: bool,
    /// Whether the entry opens a submenu (drawn with an arrow at the right).
    pub submenu: bool,
}

impl MenuEntry {
    /// An enabled [`MenuEntryKind::Normal`] entry without a submenu.
    #[must_use]
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            kind: MenuEntryKind::Normal,
            enabled: true,
            submenu: false,
        }
    }

    /// A separator row.
    #[must_use]
    pub fn separator() -> Self {
        Self {
            kind: MenuEntryKind::Separator,
            ..Self::new("")
        }
    }

    /// Whether [`menu_entry_at`] can return this entry.
    #[must_use]
    pub fn is_selectable(&self) -> bool {
        self.enabled && self.kind != MenuEntryKind::Separator
    }
}

/// What a menu shows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MenuView {
    pub entries: Vec<MenuEntry>,
    /// Index of the hovered entry, drawn highlighted (unless it is a
    /// separator or disabled).
    pub selected: Option<usize>,
}

/// Menu colors and metrics, in logical pixels. Defaults match the bar's
/// dark theme.
#[derive(Debug, Clone, PartialEq)]
pub struct MenuStyle {
    pub bg: Color,
    pub fg: Color,
    /// Text and marks of disabled entries, and separator lines.
    pub fg_disabled: Color,
    /// Background of the selected (hovered) entry.
    pub bg_selected: Color,
    /// Text and marks of the selected entry.
    pub fg_selected: Color,
    pub border: Color,
    pub border_width: u32,
    /// Font size in logical pixels.
    pub font_size: f32,
    /// Glyph edge antialiasing.
    pub antialias: Antialias,
    /// Height of every entry except separators.
    pub row_height: u32,
    /// Height of a separator row.
    pub separator_height: u32,
    /// Space between the border and the content, left and right.
    pub padding: u32,
    /// The widest the menu gets; longer labels are ellipsized.
    pub max_width: u32,
}

impl Default for MenuStyle {
    fn default() -> Self {
        Self {
            bg: Color::hex("#161616"),
            fg: Color::hex("#F2F2F2"),
            fg_disabled: Color::hex("#6A6A6A"),
            bg_selected: Color::hex("#F8E63A"),
            fg_selected: Color::hex("#161616"),
            border: Color::hex("#F2F2F2"),
            border_width: 1,
            font_size: 16.0,
            antialias: Antialias::Grayscale,
            row_height: 24,
            separator_height: 9,
            padding: 8,
            max_width: 400,
        }
    }
}

/// A menu's geometry in logical pixels.
#[derive(Debug, Clone, PartialEq)]
struct Metrics {
    width: i32,
    height: i32,
    border: i32,
    /// Side of a check box / radio circle.
    mark_size: i32,
    /// Left edge of the marks.
    mark_x: i32,
    /// Left edge and width of the labels.
    label_x: i32,
    label_width: i32,
    /// Half the height (and the width) of a submenu arrow.
    arrow_size: i32,
    /// Left edge of the submenu arrows.
    arrow_x: i32,
    /// Top and height of each entry's row.
    rows: Vec<(i32, i32)>,
}

fn metrics(view: &MenuView, style: &MenuStyle, fonts: &Fonts) -> Metrics {
    let border = to_i32(style.border_width);
    let padding = to_i32(style.padding);
    let gap = round_i32(style.font_size * 0.5);
    let has_marks = view.entries.iter().any(|entry| {
        matches!(
            entry.kind,
            MenuEntryKind::Check(_) | MenuEntryKind::Radio(_)
        )
    });
    let has_submenus = view.entries.iter().any(|entry| entry.submenu);
    let mark_size = round_i32(style.font_size * 0.625);
    let arrow_size = round_i32(style.font_size * 0.3125);
    let mark_column = if has_marks { mark_size + gap } else { 0 };
    let arrow_column = if has_submenus { gap + arrow_size } else { 0 };
    let widest = view
        .entries
        .iter()
        .filter(|entry| entry.kind != MenuEntryKind::Separator)
        .map(|entry| fonts.text_width(&entry.label, style.font_size))
        .fold(0.0, f32::max);
    // One pixel of slack: hinted advances at other scales may round up.
    let label_width = round_i32(widest.ceil()) + 1;
    let fixed = 2 * border + 2 * padding + mark_column + arrow_column;
    let width = (fixed + label_width).min(to_i32(style.max_width).max(fixed + 1));
    let label_x = border + padding + mark_column;
    let arrow_x = width - border - padding - arrow_size;

    let mut rows = Vec::with_capacity(view.entries.len());
    let mut top = border;
    for entry in &view.entries {
        let height = to_i32(if entry.kind == MenuEntryKind::Separator {
            style.separator_height
        } else {
            style.row_height
        });
        rows.push((top, height));
        top += height;
    }
    Metrics {
        width,
        height: top + border,
        border,
        mark_size,
        mark_x: border + padding,
        label_x,
        label_width: width - label_x - border - padding - arrow_column,
        arrow_size,
        arrow_x,
        rows,
    }
}

/// The logical size of the menu surface: wide enough for the widest label
/// (up to [`MenuStyle::max_width`]), as tall as all rows.
#[must_use]
pub fn menu_size(view: &MenuView, style: &MenuStyle, fonts: &Fonts) -> (u32, u32) {
    let m = metrics(view, style, fonts);
    (m.width.unsigned_abs(), m.height.unsigned_abs())
}

/// Each entry's row as logical `(x, y, width, height)`, separators
/// included; e.g. to place a submenu next to its entry.
#[must_use]
pub fn menu_entry_rects(
    view: &MenuView,
    style: &MenuStyle,
    fonts: &Fonts,
) -> Vec<(i32, i32, i32, i32)> {
    let m = metrics(view, style, fonts);
    let inner_width = m.width - 2 * m.border;
    m.rows
        .iter()
        .map(|&(top, height)| (m.border, top, inner_width, height))
        .collect()
}

/// The selectable entry under logical (`x`, `y`) relative to the menu's top
/// left corner; `None` outside the rows, on the border, on separators and on
/// disabled entries.
#[must_use]
pub fn menu_entry_at(
    view: &MenuView,
    style: &MenuStyle,
    fonts: &Fonts,
    x: f64,
    y: f64,
) -> Option<usize> {
    menu_entry_rects(view, style, fonts)
        .into_iter()
        .position(|(rx, ry, rw, rh)| {
            x >= f64::from(rx)
                && x < f64::from(rx + rw)
                && y >= f64::from(ry)
                && y < f64::from(ry + rh)
        })
        .filter(|&index| view.entries[index].is_selectable())
}

/// Renders the menu at `scale`; the canvas is [`menu_size`] × `scale`.
#[must_use]
pub fn render_menu(view: &MenuView, style: &MenuStyle, fonts: &Fonts, scale: u32) -> Canvas {
    let m = metrics(view, style, fonts);
    let mut painter = Painter::new(m.width.unsigned_abs(), m.height.unsigned_abs(), scale)
        .with_antialias(style.antialias);
    let s = to_i32(painter.scale());
    let sf = to_f32(s);
    let (canvas_width, canvas_height) = (painter.width(), painter.height());
    let size = style.font_size * sf;
    painter.fill_rect(0, 0, canvas_width, canvas_height, style.bg);
    painter.stroke_rect(
        0,
        0,
        canvas_width,
        canvas_height,
        m.border * s,
        style.border,
    );

    let padding = to_i32(style.padding);
    for (index, (entry, &(top, height))) in view.entries.iter().zip(&m.rows).enumerate() {
        let (top, height) = (top * s, height * s);
        if entry.kind == MenuEntryKind::Separator {
            let x = (m.border + padding) * s;
            let y = top + (height - s) / 2;
            painter.fill_rect(x, y, canvas_width - 2 * x, s, style.fg_disabled);
            continue;
        }
        let selected = view.selected == Some(index) && entry.enabled;
        let fg = if !entry.enabled {
            style.fg_disabled
        } else if selected {
            style.fg_selected
        } else {
            style.fg
        };
        if selected {
            painter.fill_rect(
                m.border * s,
                top,
                canvas_width - 2 * m.border * s,
                height,
                style.bg_selected,
            );
        }
        let center_y = top + height / 2;
        draw_mark(
            &mut painter,
            entry.kind,
            (m.mark_x * s, center_y - m.mark_size * s / 2),
            m.mark_size * s,
            fg,
        );
        if entry.submenu {
            let (x, half) = (to_f32(m.arrow_x * s), to_f32(m.arrow_size * s));
            let cy = to_f32(center_y);
            painter.fill_polygon(&[(x, cy - half), (x + half, cy), (x, cy + half)], fg);
        }
        let label_x = m.label_x * s;
        let max_width = m.label_width * s;
        let label = fonts.ellipsize(&entry.label, size, to_f32(max_width));
        painter.draw_text(
            fonts,
            &label,
            to_f32(label_x),
            top,
            height,
            size,
            fg,
            (label_x, label_x + max_width),
        );
    }
    painter.finish()
}

/// Draws a check box or radio circle `size` device pixels across with its
/// top left corner at `origin`; nothing for other kinds.
fn draw_mark(
    painter: &mut Painter,
    kind: MenuEntryKind,
    origin: (i32, i32),
    size: i32,
    color: Color,
) {
    let s = to_f32(to_i32(painter.scale()));
    let (x, y) = origin;
    let (xf, yf, sizef) = (to_f32(x), to_f32(y), to_f32(size));
    match kind {
        MenuEntryKind::Check(checked) => {
            painter.stroke_rect(x, y, size, size, to_i32(painter.scale()), color);
            if checked {
                let point = |px: f32, py: f32| (xf + px * sizef, yf + py * sizef);
                painter.stroke_polyline(
                    &[point(0.22, 0.52), point(0.42, 0.72), point(0.78, 0.28)],
                    1.5 * s,
                    color,
                );
            }
        }
        MenuEntryKind::Radio(on) => {
            let center = (xf + sizef / 2.0, yf + sizef / 2.0);
            painter.circle(center, sizef / 2.0, None, Some((color, s)));
            if on {
                painter.circle(center, sizef / 2.0 - 2.5 * s, Some(color), None);
            }
        }
        MenuEntryKind::Normal | MenuEntryKind::Separator => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view() -> MenuView {
        MenuView {
            entries: vec![
                MenuEntry::new("Open"),
                MenuEntry::separator(),
                MenuEntry {
                    enabled: false,
                    ..MenuEntry::new("Disabled")
                },
                MenuEntry {
                    kind: MenuEntryKind::Check(true),
                    ..MenuEntry::new("Mute")
                },
            ],
            selected: None,
        }
    }

    #[test]
    fn size_covers_rows_and_labels() {
        let fonts = Fonts::embedded();
        let style = MenuStyle::default();
        let (width, height) = menu_size(&view(), &style, &fonts);
        assert_eq!(height, 2 + 3 * 24 + 9);
        assert!(f64::from(width) > f64::from(fonts.text_width("Disabled", 16.0)) + 16.0);
    }

    #[test]
    fn hit_test_skips_separators_and_disabled_entries() {
        let fonts = Fonts::embedded();
        let style = MenuStyle::default();
        let view = view();
        let at = |x, y| menu_entry_at(&view, &style, &fonts, x, y);
        assert_eq!(at(10.0, 1.0), Some(0));
        assert_eq!(at(10.0, 24.9), Some(0));
        assert_eq!(at(10.0, 25.0), None, "separator");
        assert_eq!(at(10.0, 40.0), None, "disabled");
        assert_eq!(at(10.0, 60.0), Some(3));
        assert_eq!(at(0.5, 60.0), None, "border");
        assert_eq!(at(10.0, 200.0), None, "below");
        assert_eq!(at(-1.0, 10.0), None, "left");
    }

    #[test]
    fn long_labels_are_clamped() {
        let fonts = Fonts::embedded();
        let style = MenuStyle::default();
        let view = MenuView {
            entries: vec![MenuEntry::new("x".repeat(200))],
            selected: None,
        };
        assert_eq!(menu_size(&view, &style, &fonts).0, style.max_width);
    }

    #[test]
    fn renders_at_scale() {
        let fonts = Fonts::embedded();
        let style = MenuStyle::default();
        let (width, height) = menu_size(&view(), &style, &fonts);
        let canvas = render_menu(&view(), &style, &fonts, 2);
        assert_eq!((canvas.width(), canvas.height()), (width * 2, height * 2));
    }
}
