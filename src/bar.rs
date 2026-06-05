//! Built-in compositor status bar.

use smithay::{
    backend::renderer::{
        Color32F,
        element::{
            Kind,
            solid::{
                SolidColorBuffer,
                SolidColorRenderElement,
            },
        },
    },
    output::Output,
    utils::{
        Logical,
        Point,
        Rectangle,
        Size,
    },
    wayland::{
        compositor::with_states,
        shell::xdg::{
            ToplevelSurface,
            XdgToplevelSurfaceData,
        },
    },
};

use crate::{
    Oxmonad,
    config,
};

const TEXT_X: i32 = 10;
const GLYPH_W: i32 = 5;
const GLYPH_H: i32 = 7;
const GLYPH_SCALE: i32 = 2;
const GLYPH_GAP: i32 = 1;
const MAX_TITLE_CHARS: usize = 96;

/// Returns the client layout area after reserving space for the built-in bar.
#[must_use]
pub fn content_area(area: Rectangle<i32, Logical>) -> Rectangle<i32, Logical> {
    let bar_height = config::BAR_HEIGHT.clamp(0, area.size.h);
    Rectangle::new(
        Point::from((area.loc.x, area.loc.y + bar_height)),
        Size::from((area.size.w, area.size.h - bar_height)),
    )
}

impl Oxmonad {
    /// Builds render elements for the built-in status bar.
    #[must_use]
    pub fn bar_elements(&self, output: &Output, scale: f64) -> Vec<SolidColorRenderElement> {
        // The bar belongs to the primary monitor only.
        if !self.is_primary_output(output) {
            return Vec::new();
        }

        let Some(output_geo) = self.space.output_geometry(output) else {
            return Vec::new();
        };

        bar_elements_for_title(
            output_geo,
            self.focused_title().as_deref().unwrap_or_default(),
            scale,
        )
    }

    fn focused_title(&self) -> Option<String> {
        let window = self.focused_window()?;
        let toplevel = window.toplevel()?;
        window_title(toplevel)
    }
}

pub(crate) fn bar_elements_for_title(
    output_geo: Rectangle<i32, Logical>,
    title: &str,
    scale: f64,
) -> Vec<SolidColorRenderElement> {
    let height = config::BAR_HEIGHT.clamp(0, output_geo.size.h);
    if height == 0 {
        return Vec::new();
    }

    let bar = Rectangle::new(output_geo.loc, Size::from((output_geo.size.w, height)));
    let mut elements = Vec::new();

    let y = output_geo.loc.y + (height - GLYPH_H * GLYPH_SCALE) / 2;
    let max_width = output_geo.size.w - TEXT_X * 2;
    draw_text(
        &mut elements,
        Point::from((output_geo.loc.x + TEXT_X, y)),
        max_width,
        title,
        config::BAR_TEXT_COLOR,
        scale,
    );

    // Smithay expects render elements in front-to-back order. Keep the
    // text before the background so it is drawn on top.
    elements.push(solid_element(bar, config::BAR_BACKGROUND_COLOR, scale));

    elements
}

fn window_title(toplevel: &ToplevelSurface) -> Option<String> {
    with_states(toplevel.wl_surface(), |states| {
        states
            .data_map
            .get::<XdgToplevelSurfaceData>()?
            .lock()
            .ok()?
            .title
            .clone()
    })
}

fn draw_text(
    elements: &mut Vec<SolidColorRenderElement>,
    loc: Point<i32, Logical>,
    max_width: i32,
    text: &str,
    color: Color32F,
    scale: f64,
) {
    let advance = (GLYPH_W + GLYPH_GAP) * GLYPH_SCALE;
    let max_chars = usize::try_from((max_width / advance).max(0)).unwrap_or_default();
    let limit = MAX_TITLE_CHARS.min(max_chars);
    let mut x = loc.x;

    for ch in text.chars().take(limit) {
        draw_glyph(elements, Point::from((x, loc.y)), ch, color, scale);
        x += advance;
    }
}

fn draw_glyph(
    elements: &mut Vec<SolidColorRenderElement>,
    loc: Point<i32, Logical>,
    ch: char,
    color: Color32F,
    scale: f64,
) {
    for (row, bits) in glyph(ch).iter().enumerate() {
        for col in 0..GLYPH_W {
            if bits & (1 << (GLYPH_W - 1 - col)) == 0 {
                continue;
            }

            elements.push(solid_element(
                Rectangle::new(
                    Point::from((
                        loc.x + col * GLYPH_SCALE,
                        loc.y + i32::try_from(row).unwrap_or_default() * GLYPH_SCALE,
                    )),
                    Size::from((GLYPH_SCALE, GLYPH_SCALE)),
                ),
                color,
                scale,
            ));
        }
    }
}

fn solid_element(
    rect: Rectangle<i32, Logical>,
    color: Color32F,
    scale: f64,
) -> SolidColorRenderElement {
    let buffer = SolidColorBuffer::new(rect.size, color);
    SolidColorRenderElement::from_buffer(
        &buffer,
        rect.loc.to_physical_precise_round(scale),
        scale,
        1.0,
        Kind::Unspecified,
    )
}

#[expect(
    clippy::too_many_lines,
    reason = "A compact built-in bitmap font keeps the internal bar self-contained."
)]
fn glyph(ch: char) -> [i32; 7] {
    match ch.to_ascii_uppercase() {
        'A' => [
            0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
        'B' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110,
        ],
        'C' => [
            0b01111, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b01111,
        ],
        'D' => [
            0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11110,
        ],
        'E' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111,
        ],
        'F' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000,
        ],
        'G' => [
            0b01111, 0b10000, 0b10000, 0b10111, 0b10001, 0b10001, 0b01111,
        ],
        'H' => [
            0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
        'I' => [
            0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b11111,
        ],
        'J' => [
            0b00111, 0b00010, 0b00010, 0b00010, 0b00010, 0b10010, 0b01100,
        ],
        'K' => [
            0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001,
        ],
        'L' => [
            0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111,
        ],
        'M' => [
            0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001,
        ],
        'N' => [
            0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001, 0b10001,
        ],
        'O' => [
            0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
        'P' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000,
        ],
        'Q' => [
            0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101,
        ],
        'R' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001,
        ],
        'S' => [
            0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110,
        ],
        'T' => [
            0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100,
        ],
        'U' => [
            0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
        'V' => [
            0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100,
        ],
        'W' => [
            0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b10101, 0b01010,
        ],
        'X' => [
            0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001,
        ],
        'Y' => [
            0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100,
        ],
        'Z' => [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111,
        ],
        '0' => [
            0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110,
        ],
        '1' => [
            0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
        ],
        '2' => [
            0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111,
        ],
        '3' => [
            0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110,
        ],
        '4' => [
            0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010,
        ],
        '5' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b00001, 0b00001, 0b11110,
        ],
        '6' => [
            0b01110, 0b10000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110,
        ],
        '7' => [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000,
        ],
        '8' => [
            0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110,
        ],
        '9' => [
            0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00001, 0b01110,
        ],
        ' ' => [0, 0, 0, 0, 0, 0, 0],
        '-' => [0, 0, 0, 0b11111, 0, 0, 0],
        '_' => [0, 0, 0, 0, 0, 0, 0b11111],
        '.' => [0, 0, 0, 0, 0, 0b01100, 0b01100],
        ':' => [0, 0b01100, 0b01100, 0, 0b01100, 0b01100, 0],
        '/' => [
            0b00001, 0b00010, 0b00010, 0b00100, 0b01000, 0b01000, 0b10000,
        ],
        '+' => [0, 0b00100, 0b00100, 0b11111, 0b00100, 0b00100, 0],
        '?' => [0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0, 0b00100],
        _ => glyph('?'),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smithay::{
        backend::renderer::element::Element,
        utils::Physical,
    };

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Logical> {
        Rectangle::new(Point::from((x, y)), Size::from((w, h)))
    }

    fn physical_rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Physical> {
        Rectangle::new(Point::from((x, y)), Size::from((w, h)))
    }

    #[test]
    fn content_area_reserves_bar_height() {
        assert_eq!(content_area(rect(0, 0, 1000, 1000)), rect(0, 24, 1000, 976));
    }

    #[test]
    fn content_area_clamps_bar_to_area_height() {
        assert_eq!(content_area(rect(0, 0, 1000, 10)), rect(0, 10, 1000, 0));
    }

    #[test]
    fn text_elements_are_front_of_background() {
        let mut elements = Vec::new();
        draw_text(
            &mut elements,
            Point::from((0, 0)),
            100,
            "A",
            config::BAR_TEXT_COLOR,
            1.0,
        );
        elements.push(solid_element(
            rect(0, 0, 100, config::BAR_HEIGHT),
            config::BAR_BACKGROUND_COLOR,
            1.0,
        ));

        assert!(elements.len() > 1);
        assert_eq!(
            elements
                .first()
                .expect("text element exists")
                .geometry(1.0.into())
                .loc,
            Point::from((2, 0))
        );
        assert_eq!(
            elements
                .last()
                .expect("background element exists")
                .geometry(1.0.into()),
            physical_rect(0, 0, 100, config::BAR_HEIGHT)
        );
    }

    #[test]
    fn snapshot_bar_with_focused_title() {
        let elements = bar_elements_for_title(rect(0, 0, 1000, 1000), "oxmonad-snapshot", 1.0);

        crate::snapshot::assert_snapshot("built_in_bar_title", 1000, 1000, &elements);
    }
}
