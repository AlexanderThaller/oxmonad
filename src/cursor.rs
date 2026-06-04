//! Software cursor for the TTY/DRM backend.
//!
//! When running nested, the host compositor draws the cursor for us, but on a
//! bare VT oxmonad must render its own. While hovering a window the client
//! usually provides its own cursor surface (rendered directly by the DRM
//! backend); [`fallback_cursor`] draws a simple arrow for everything else — the
//! background, the bar, or before any client has set a cursor.

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
    utils::{
        Logical,
        Point,
        Rectangle,
        Size,
    },
};

/// Logical size of one cell of the arrow bitmap.
const CELL: i32 = 2;

/// Dark outline colour, so the cursor stays visible over light windows.
const OUTLINE: Color32F = Color32F::new(0.0, 0.0, 0.0, 1.0);
/// Light fill colour, so the cursor stays visible over dark windows.
const FILL: Color32F = Color32F::new(1.0, 1.0, 1.0, 1.0);

/// Classic arrow cursor bitmap. `#` is the dark outline, `.` the light fill and
/// a space is transparent. The hotspot is the top-left corner.
const ARROW: [&str; 16] = [
    "#",
    "##",
    "#.#",
    "#..#",
    "#...#",
    "#....#",
    "#.....#",
    "#......#",
    "#.......#",
    "#........#",
    "#.....#####",
    "#..#..#",
    "#.# #..#",
    "##  #..#",
    "#    #..#",
    "     ####",
];

/// Builds render elements for the fallback arrow, with its hotspot at `loc`
/// (output-relative logical coordinates).
#[must_use]
pub fn fallback_cursor(loc: Point<i32, Logical>, scale: f64) -> Vec<SolidColorRenderElement> {
    let mut elements = Vec::new();

    for (row, line) in ARROW.iter().enumerate() {
        let y = loc.y + i32::try_from(row).unwrap_or_default() * CELL;
        let cells: Vec<char> = line.chars().collect();

        // Merge consecutive same-colour cells into a single rectangle to keep
        // the per-frame element count low.
        let mut col = 0;
        while col < cells.len() {
            let cell = cells[col];
            let color = match cell {
                '#' => OUTLINE,
                '.' => FILL,
                _ => {
                    col += 1;
                    continue;
                }
            };

            let start = col;
            while col < cells.len() && cells[col] == cell {
                col += 1;
            }
            let run = i32::try_from(col - start).unwrap_or_default();
            let x = loc.x + i32::try_from(start).unwrap_or_default() * CELL;

            elements.push(solid_element(
                Rectangle::new(Point::from((x, y)), Size::from((run * CELL, CELL))),
                color,
                scale,
            ));
        }
    }

    elements
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
        Kind::Cursor,
    )
}

#[cfg(test)]
mod tests {
    use smithay::backend::renderer::element::Element;

    use super::*;

    #[test]
    fn fallback_cursor_anchors_its_hotspot_at_the_location() {
        let elements = fallback_cursor(Point::from((100, 50)), 1.0);
        assert!(!elements.is_empty());

        // The arrow's tip (the top-left outline cell) is the hotspot and must
        // sit exactly at the requested location.
        let top_left = elements
            .iter()
            .map(|element| element.geometry(1.0.into()).loc)
            .min_by_key(|loc| (loc.y, loc.x))
            .expect("cursor has elements");
        assert_eq!(top_left, Point::from((100, 50)));
    }
}
