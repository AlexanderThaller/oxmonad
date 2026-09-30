//! The `XMonad.Prompt`-style input line and its completion grid.

use crate::{
    Antialias,
    Canvas,
    Color,
    Fonts,
    canvas::{
        Painter,
        baseline,
        round_i32,
        to_f32,
        to_i32,
    },
};

/// What the prompt shows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PromptView {
    /// The prompter text before the input, e.g. `"Workspace: "`.
    pub label: String,
    pub input: String,
    /// Cursor position within `input`, in characters.
    pub cursor: usize,
    pub completions: Vec<String>,
    /// Index into `completions` of the highlighted entry. With `None`, an
    /// entry equal to `input` is highlighted, as xmonad does.
    pub selected: Option<usize>,
}

/// Where the prompt sits on the screen; completions open towards the middle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PromptPosition {
    Top,
    Bottom,
}

/// Prompt colors and metrics, in logical pixels. Defaults are `myXPConfig`
/// over xmonad's `def` `XPConfig`.
#[derive(Debug, Clone, PartialEq)]
pub struct PromptStyle {
    pub bg: Color,
    pub fg: Color,
    pub bg_highlight: Color,
    pub fg_highlight: Color,
    pub border: Color,
    pub border_width: u32,
    /// Font size in logical pixels.
    pub font_size: f32,
    /// Glyph edge antialiasing.
    pub antialias: Antialias,
    /// Height of the input line and of each completion row.
    pub height: u32,
    pub position: PromptPosition,
    /// `maxComplRows`; the screen height always limits the rows too.
    pub max_rows: Option<u32>,
    /// `maxComplColumns`.
    pub max_columns: Option<u32>,
}

impl Default for PromptStyle {
    fn default() -> Self {
        Self {
            bg: Color::hex("#161616"),
            fg: Color::hex("#F2F2F2"),
            bg_highlight: Color::hex("#1565C1"),
            fg_highlight: Color::hex("#F2F2F2"),
            border: Color::hex("#F2F2F2"),
            border_width: 1,
            font_size: 20.0,
            height: 30,
            position: PromptPosition::Bottom,
            max_rows: None,
            max_columns: None,
            antialias: Antialias::Grayscale,
        }
    }
}

/// The completion grid's shape in cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Grid {
    columns: u32,
    rows: u32,
}

/// xmonad's `getComplWinDim`: columns as wide as the widest entry plus half
/// a row height, as many as fit; rows limited by the screen and the config.
fn grid(
    view: &PromptView,
    style: &PromptStyle,
    fonts: &Fonts,
    width: u32,
    max_height: u32,
) -> Grid {
    if view.completions.is_empty() || style.height == 0 {
        return Grid {
            columns: 1,
            rows: 0,
        };
    }
    let widest = view
        .completions
        .iter()
        .map(|entry| fonts.text_width(entry, style.font_size))
        .fold(0.0, f32::max);
    let cell = to_f32(to_i32(style.height / 2)) + widest;
    let fitting = u32::try_from(round_i32((to_f32(to_i32(width)) / cell).floor())).unwrap_or(0);
    let columns = style
        .max_columns
        .map_or(fitting, |max| fitting.min(max))
        .max(1);
    let count = u32::try_from(view.completions.len()).unwrap_or(u32::MAX);
    let needed = count.div_ceil(columns);
    let screen_rows = max_height.saturating_sub(style.height) / style.height;
    let rows = style
        .max_rows
        .map_or(screen_rows, |max| screen_rows.min(max))
        .min(needed);
    Grid { columns, rows }
}

/// The logical size of the prompt surface for `view` on a screen of
/// `width` × `max_height` logical pixels: full width, one input line plus the
/// completion rows (which share a border line with it).
#[must_use]
pub fn prompt_size(
    view: &PromptView,
    style: &PromptStyle,
    fonts: &Fonts,
    width: u32,
    max_height: u32,
) -> (u32, u32) {
    let grid = grid(view, style, fonts, width, max_height);
    let completions = if grid.rows == 0 {
        0
    } else {
        (grid.rows * style.height).saturating_sub(style.border_width)
    };
    (width, style.height + completions)
}

/// Renders the prompt at `scale`; the canvas is [`prompt_size`] × `scale`.
/// With [`PromptPosition::Bottom`] the input line is the bottom row and the
/// completions sit above it, otherwise below it.
#[must_use]
pub fn render_prompt(
    view: &PromptView,
    style: &PromptStyle,
    fonts: &Fonts,
    width: u32,
    max_height: u32,
    scale: u32,
) -> Canvas {
    let grid = grid(view, style, fonts, width, max_height);
    let (_, height) = prompt_size(view, style, fonts, width, max_height);
    let mut painter = Painter::new(width, height, scale).with_antialias(style.antialias);
    let s = to_i32(painter.scale());
    let (canvas_width, canvas_height) = (painter.width(), painter.height());
    let row_height = to_i32(style.height) * s;
    let border = to_i32(style.border_width) * s;
    let size = style.font_size * to_f32(s);
    painter.fill_rect(0, 0, canvas_width, canvas_height, style.bg);

    let grid_height = to_i32(grid.rows) * row_height;
    let (input_top, grid_top) = match style.position {
        PromptPosition::Top => (0, row_height - border),
        PromptPosition::Bottom => (canvas_height - row_height, 0),
    };
    if grid.rows > 0 {
        painter.stroke_rect(0, grid_top, canvas_width, grid_height, border, style.border);
        draw_completions(&mut painter, view, style, fonts, grid, grid_top, size);
    }
    painter.stroke_rect(0, input_top, canvas_width, row_height, border, style.border);
    draw_input(&mut painter, view, style, fonts, input_top, size);
    painter.finish()
}

/// Horizontal text inset: half the font's line height, like xmonad.
fn inset(fonts: &Fonts, size: f32) -> f32 {
    let (ascent, descent) = fonts.vertical_metrics(size);
    f32::midpoint(ascent, descent).round()
}

/// The label and input with a reverse-video block cursor (a space when the
/// cursor is at the end), like `printPrompt`.
fn draw_input(
    painter: &mut Painter,
    view: &PromptView,
    style: &PromptStyle,
    fonts: &Fonts,
    top: i32,
    size: f32,
) {
    let s = to_i32(painter.scale());
    let row_height = to_i32(style.height) * s;
    let clip = (0, painter.width() - to_i32(style.border_width) * s);
    let split = view
        .input
        .char_indices()
        .nth(view.cursor)
        .map_or(view.input.len(), |(index, _)| index);
    let (before, after) = view.input.split_at(split);
    let mut chars = after.chars();
    let cursor = chars.next().map_or_else(|| " ".to_owned(), String::from);
    let after = chars.as_str();

    let x = inset(fonts, size);
    let before = format!("{}{before}", view.label);
    painter.draw_text(fonts, &before, x, top, row_height, size, style.fg, clip);
    let cursor_x = x + fonts.text_width(&before, size);
    let cursor_width = fonts.text_width(&cursor, size);
    let (ascent, descent) = fonts.vertical_metrics(size);
    let text_top = baseline(fonts, top, row_height, size) - round_i32(ascent);
    let (start, end) = (round_i32(cursor_x), round_i32(cursor_x + cursor_width));
    painter.fill_rect(
        start,
        text_top,
        end - start,
        round_i32(ascent + descent),
        style.fg,
    );
    painter.draw_text(
        fonts, &cursor, cursor_x, top, row_height, size, style.bg, clip,
    );
    painter.draw_text(
        fonts,
        after,
        cursor_x + cursor_width,
        top,
        row_height,
        size,
        style.fg,
        clip,
    );
}

/// The completion entries, column-major, paged so the selection is visible.
fn draw_completions(
    painter: &mut Painter,
    view: &PromptView,
    style: &PromptStyle,
    fonts: &Fonts,
    grid: Grid,
    top: i32,
    size: f32,
) {
    let s = to_i32(painter.scale());
    let row_height = to_i32(style.height) * s;
    let border = to_i32(style.border_width) * s;
    let columns = to_i32(grid.columns);
    let column_width = painter.width() / columns;
    let page = (grid.rows * grid.columns) as usize;
    let first = view.selected.map_or(0, |selected| selected / page * page);
    let highlighted = view.selected.or_else(|| {
        view.completions
            .iter()
            .position(|entry| *entry == view.input)
    });
    let x_inset = inset(fonts, size);
    let (ascent, descent) = fonts.vertical_metrics(size);

    for (offset, entry) in view.completions.iter().skip(first).take(page).enumerate() {
        let index = to_i32(offset);
        let (column, row) = (index / to_i32(grid.rows), index % to_i32(grid.rows));
        let cell_x = column * column_width;
        let cell_top = top + row * row_height;
        let clip_right = (cell_x + column_width).min(painter.width() - border);
        let text = fonts.ellipsize(entry, size, to_f32(clip_right - cell_x) - x_inset);
        let text_x = to_f32(cell_x) + x_inset;
        let mut fg = style.fg;
        if highlighted == Some(first + offset) {
            let pad = round_i32(x_inset / 3.0);
            let text_top = baseline(fonts, cell_top, row_height, size) - round_i32(ascent);
            let (start, end) = (
                round_i32(text_x) - pad,
                round_i32(text_x + fonts.text_width(&text, size)) + pad,
            );
            // Kept inside its row, so it never covers the prompt's border or
            // the neighbouring rows.
            let highlight_top = (text_top - pad).max(cell_top).max(border);
            let highlight_bottom = (text_top - pad + round_i32(ascent + descent) + 2 * pad)
                .min(cell_top + row_height)
                .min(painter.height() - border);
            painter.fill_rect(
                start,
                highlight_top,
                end - start,
                highlight_bottom - highlight_top,
                style.bg_highlight,
            );
            fg = style.fg_highlight;
        }
        painter.draw_text(
            fonts,
            &text,
            text_x,
            cell_top,
            row_height,
            size,
            fg,
            (cell_x, clip_right),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(completions: usize) -> PromptView {
        PromptView {
            completions: (0..completions).map(|i| format!("ws{i}")).collect(),
            ..PromptView::default()
        }
    }

    #[test]
    fn size_without_completions_is_one_line() {
        let style = PromptStyle::default();
        assert_eq!(
            prompt_size(&view(0), &style, &Fonts::embedded(), 800, 600),
            (800, 30)
        );
    }

    #[test]
    fn completions_fill_columns_then_rows() {
        let fonts = Fonts::embedded();
        let style = PromptStyle::default();
        let grid = grid(&view(3), &style, &fonts, 800, 600);
        assert_eq!(grid.rows, 1);
        assert!(grid.columns >= 3);
        let (_, height) = prompt_size(&view(3), &style, &fonts, 800, 600);
        assert_eq!(height, 30 + 30 - 1);
    }

    #[test]
    fn rows_are_limited_by_screen_and_config() {
        let fonts = Fonts::embedded();
        let mut style = PromptStyle {
            max_columns: Some(1),
            ..PromptStyle::default()
        };
        assert_eq!(grid(&view(100), &style, &fonts, 800, 300).rows, 9);
        style.max_rows = Some(4);
        assert_eq!(grid(&view(100), &style, &fonts, 800, 300).rows, 4);
    }
}
