//! The tab strip of the tabbed layout (`XMonad.Layout.Tabbed`).

use crate::{
    Canvas,
    Color,
    Fonts,
    canvas::{
        Painter,
        to_f32,
        to_i32,
    },
};

/// One window of the tabbed layout.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tab {
    pub title: String,
    pub focused: bool,
    pub urgent: bool,
}

/// The tabs, left to right.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TabsView {
    pub tabs: Vec<Tab>,
}

/// Fill, border and text color of one tab state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TabColors {
    pub fill: Color,
    pub border: Color,
    pub text: Color,
}

/// Tab colors and metrics, in logical pixels. Defaults are xmonad's `def`
/// `Theme` (`XMonad.Layout.Decoration`) with `myTabConf`'s Fira Mono 12.
#[derive(Debug, Clone, PartialEq)]
pub struct TabStyle {
    pub active: TabColors,
    pub inactive: TabColors,
    pub urgent: TabColors,
    pub border_width: u32,
    /// Font size in logical pixels.
    pub font_size: f32,
    /// `decoHeight`.
    pub height: u32,
}

impl Default for TabStyle {
    fn default() -> Self {
        Self {
            active: TabColors {
                fill: Color::hex("#999999"),
                border: Color::hex("#FFFFFF"),
                text: Color::hex("#FFFFFF"),
            },
            inactive: TabColors {
                fill: Color::hex("#666666"),
                border: Color::hex("#BBBBBB"),
                text: Color::hex("#BFBFBF"),
            },
            // xmonad's default is the typo "##00FF00"; this is its intent.
            urgent: TabColors {
                fill: Color::hex("#FFFF00"),
                border: Color::hex("#00FF00"),
                text: Color::hex("#FF0000"),
            },
            border_width: 1,
            font_size: 14.0,
            height: 20,
        }
    }
}

/// The device-pixel span `start..end` of tab `index` of `count` across
/// `width` pixels; tabs share the width equally, the last absorbs rounding.
fn tab_span(index: usize, count: usize, width: i32) -> (i32, i32) {
    let count = to_i32(count).max(1);
    let index = to_i32(index);
    (width * index / count, width * (index + 1) / count)
}

/// The tab under logical x coordinate `x` of a strip `width` logical pixels
/// wide.
#[must_use]
pub fn tab_at(view: &TabsView, width: u32, x: f64) -> Option<usize> {
    if x < 0.0 || x >= f64::from(width) {
        return None;
    }
    let width = to_i32(width);
    (0..view.tabs.len()).find(|&index| {
        let (start, end) = tab_span(index, view.tabs.len(), width);
        x >= f64::from(start) && x < f64::from(end)
    })
}

/// Renders the strip `width` logical pixels wide at `scale`; the canvas is
/// `width * scale` × `style.height * scale` device pixels. Titles are
/// centered and ellipsized to their tab.
#[must_use]
pub fn render_tabs(
    view: &TabsView,
    style: &TabStyle,
    fonts: &Fonts,
    width: u32,
    scale: u32,
) -> Canvas {
    let mut painter = Painter::new(width, style.height, scale);
    let s = to_i32(painter.scale());
    let (canvas_width, canvas_height) = (painter.width(), painter.height());
    let border = to_i32(style.border_width) * s;
    let size = style.font_size * to_f32(s);
    painter.fill_rect(0, 0, canvas_width, canvas_height, style.inactive.fill);

    for (index, tab) in view.tabs.iter().enumerate() {
        let colors = if tab.urgent {
            style.urgent
        } else if tab.focused {
            style.active
        } else {
            style.inactive
        };
        // Spans are computed in logical pixels so hit-testing agrees.
        let (start, end) = tab_span(index, view.tabs.len(), to_i32(width));
        let (start, end) = (start * s, end * s);
        painter.fill_rect(start, 0, end - start, canvas_height, colors.fill);
        painter.stroke_rect(start, 0, end - start, canvas_height, border, colors.border);

        let room = to_f32(end - start - 2 * border) - size / 2.0;
        let title = fonts.ellipsize(&tab.title, size, room);
        let x = to_f32(start) + (to_f32(end - start) - fonts.text_width(&title, size)) / 2.0;
        painter.draw_text(
            fonts,
            &title,
            x,
            0,
            canvas_height,
            size,
            colors.text,
            (start + border, end - border),
        );
    }
    painter.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabs_split_the_width_evenly() {
        let view = TabsView {
            tabs: vec![Tab::default(); 3],
        };
        assert_eq!(tab_at(&view, 300, 0.0), Some(0));
        assert_eq!(tab_at(&view, 300, 99.9), Some(0));
        assert_eq!(tab_at(&view, 300, 100.0), Some(1));
        assert_eq!(tab_at(&view, 300, 299.0), Some(2));
        assert_eq!(tab_at(&view, 300, 300.0), None);
        assert_eq!(tab_at(&TabsView::default(), 300, 10.0), None);
    }
}
