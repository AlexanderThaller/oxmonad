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

pub(crate) const TEXT_X: i32 = 10;
pub(crate) const GLYPH_W: i32 = 5;
pub(crate) const GLYPH_H: i32 = 7;
pub(crate) const GLYPH_SCALE: i32 = 2;
pub(crate) const GLYPH_GAP: i32 = 1;
const MAX_TITLE_CHARS: usize = 96;

/// Horizontal advance of one glyph cell, in logical pixels.
pub(crate) const fn glyph_advance() -> i32 {
    (GLYPH_W + GLYPH_GAP) * GLYPH_SCALE
}

/// Returns the client layout area after reserving space for the built-in bar.
#[must_use]
pub fn content_area(area: Rectangle<i32, Logical>) -> Rectangle<i32, Logical> {
    let bar_height = config::BAR_HEIGHT.clamp(0, area.size.h);
    Rectangle::new(
        Point::from((area.loc.x, area.loc.y + bar_height)),
        Size::from((area.size.w, area.size.h - bar_height)),
    )
}

/// One workspace tag rendered in the bar: its display name and whether it is
/// the workspace shown on that bar's monitor.
pub(crate) struct WorkspaceTag<'a> {
    pub name: &'a str,
    pub shown: bool,
}

impl Oxmonad {
    /// Shows or hides the built-in bar, retiling so clients reclaim (or give
    /// up) the strip it occupies — xmonad's toggle-struts key.
    pub fn toggle_bar(&mut self) {
        self.bar_visible = !self.bar_visible;
        self.relayout();
    }

    /// Builds every compositor-drawn overlay for `output`, front-to-back: the
    /// workspace prompt (if open, on the active monitor), the built-in bar and
    /// the window borders.
    #[must_use]
    pub fn overlay_elements(&self, output: &Output, scale: f64) -> Vec<SolidColorRenderElement> {
        let mut elements = Vec::new();
        if let Some(prompt) = &self.prompt
            && self
                .monitors
                .get(self.active_monitor)
                .is_some_and(|monitor| monitor.output.name() == output.name())
            && let Some(output_geo) = self.space.output_geometry(output)
        {
            elements.extend(crate::prompt::prompt_elements(prompt, output_geo, scale));
        }
        elements.extend(self.bar_elements(output, scale));
        elements.extend(self.tab_elements(output, scale));
        elements.extend(self.border_elements(scale));
        elements
    }

    /// Builds render elements for the built-in status bar: the workspace tags
    /// followed by the focused window's title.
    #[must_use]
    pub fn bar_elements(&self, output: &Output, scale: f64) -> Vec<SolidColorRenderElement> {
        // The bar belongs to the primary monitor only, and can be toggled off.
        if !self.bar_visible || !self.is_primary_output(output) {
            return Vec::new();
        }

        let Some(output_geo) = self.space.output_geometry(output) else {
            return Vec::new();
        };

        // Highlight the workspace currently displayed on this bar's monitor.
        let shown = self
            .monitors
            .iter()
            .find(|monitor| monitor.output.name() == output.name())
            .map(|monitor| monitor.workspace);
        let tags: Vec<WorkspaceTag<'_>> = self
            .workspaces
            .iter()
            .enumerate()
            .map(|(index, workspace)| WorkspaceTag {
                name: &workspace.name,
                shown: shown == Some(index),
            })
            .collect();

        // The current layout name for this monitor's workspace (xmobar's
        // default `ppOrder` shows it between the workspaces and the title).
        let layout_name = self
            .monitors
            .iter()
            .find(|monitor| monitor.output.name() == output.name())
            .and_then(|monitor| self.workspaces.get(monitor.workspace))
            .map_or("", |workspace| workspace.layout.name());

        let title = self.focused_title().unwrap_or_default();
        render_bar(output_geo, &tags, layout_name, &title, scale)
    }

    /// Builds the tab-strip render elements for a monitor showing a `Tabbed`
    /// workspace with more than one window; empty otherwise.
    #[must_use]
    pub fn tab_elements(&self, output: &Output, scale: f64) -> Vec<SolidColorRenderElement> {
        let Some(monitor) = self
            .monitors
            .iter()
            .find(|monitor| monitor.output.name() == output.name())
        else {
            return Vec::new();
        };
        let Some(workspace) = self.workspaces.get(monitor.workspace) else {
            return Vec::new();
        };
        if !workspace.layout.is_tabbed() {
            return Vec::new();
        }
        let windows = self.tiling_order(workspace);
        if windows.len() < 2 {
            return Vec::new();
        }
        let Some(area) = self.tiling_area(monitor) else {
            return Vec::new();
        };
        let focused = self.focus_index_within(&windows);
        let titles: Vec<String> = windows
            .iter()
            .map(|window| window.toplevel().and_then(window_title).unwrap_or_default())
            .collect();
        render_tabs(area, &titles, focused, scale)
    }

    fn focused_title(&self) -> Option<String> {
        let window = self.focused_window()?;
        let toplevel = window.toplevel()?;
        window_title(toplevel)
    }
}

/// Pixels between adjacent workspace tags (and before the window title).
const TAG_GAP: i32 = GLYPH_W * GLYPH_SCALE;

pub(crate) fn render_bar(
    output_geo: Rectangle<i32, Logical>,
    tags: &[WorkspaceTag<'_>],
    layout_name: &str,
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
    let advance = (GLYPH_W + GLYPH_GAP) * GLYPH_SCALE;
    let right_edge = output_geo.loc.x + output_geo.size.w - TEXT_X;
    let mut x = output_geo.loc.x + TEXT_X;

    // Workspace tags; the one shown on this monitor is highlighted and
    // wrapped in angle brackets, matching the xmonad `ppCurrent` format.
    for tag in tags {
        let (text, color) = if tag.shown {
            (
                format!("<{}>", tag.name),
                config::BAR_WORKSPACE_ACTIVE_COLOR,
            )
        } else {
            (tag.name.to_string(), config::BAR_TEXT_COLOR)
        };
        let chars = i32::try_from(text.chars().count()).unwrap_or(0);
        draw_text(
            &mut elements,
            Point::from((x, y)),
            (right_edge - x).max(0),
            &text,
            color,
            scale,
        );
        x += chars * advance + TAG_GAP;
    }

    // The current layout name (xmobar's `ppLayout`, default foreground color).
    let layout_chars = i32::try_from(layout_name.chars().count()).unwrap_or(0);
    draw_text(
        &mut elements,
        Point::from((x, y)),
        (right_edge - x).max(0),
        layout_name,
        config::BAR_TEXT_COLOR,
        scale,
    );
    x += layout_chars * advance + TAG_GAP;

    // A wider gap, then the focused window title (`ppTitle` green).
    x += TAG_GAP;
    draw_text(
        &mut elements,
        Point::from((x, y)),
        (right_edge - x).max(0),
        title,
        config::BAR_TITLE_COLOR,
        scale,
    );

    // Smithay expects render elements in front-to-back order. Keep the
    // text before the background so it is drawn on top.
    elements.push(solid_element(bar, config::BAR_BACKGROUND_COLOR, scale));

    elements
}

/// Renders the tabbed-layout strip: one equal-width tab per window title, the
/// focused one highlighted, drawn along the top of `area`.
///
/// Tab widths follow xmonad's `Tabbed` decoration: window `i` spans
/// `[area.x + area.w*i/n, area.x + area.w*(i+1)/n)`.
pub(crate) fn render_tabs(
    area: Rectangle<i32, Logical>,
    titles: &[String],
    focused: usize,
    scale: f64,
) -> Vec<SolidColorRenderElement> {
    use crate::layout::{
        TAB_ACTIVE_BG,
        TAB_ACTIVE_TEXT,
        TAB_BAR_HEIGHT,
        TAB_INACTIVE_BG,
        TAB_INACTIVE_TEXT,
    };

    let count = i32::try_from(titles.len()).unwrap_or(0);
    if count < 2 {
        return Vec::new();
    }

    let mut elements = Vec::new();
    let text_y = area.loc.y + (TAB_BAR_HEIGHT - GLYPH_H * GLYPH_SCALE) / 2;
    let pad = GLYPH_SCALE * 2;

    for (index, title) in titles.iter().enumerate() {
        let i = i32::try_from(index).unwrap_or(0);
        let x0 = area.loc.x + area.size.w * i / count;
        let x1 = area.loc.x + area.size.w * (i + 1) / count;
        let width = (x1 - x0).max(0);
        let (background, text_color) = if index == focused {
            (TAB_ACTIVE_BG, TAB_ACTIVE_TEXT)
        } else {
            (TAB_INACTIVE_BG, TAB_INACTIVE_TEXT)
        };

        // Text first so it renders on top of its tab background.
        draw_text(
            &mut elements,
            Point::from((x0 + pad, text_y)),
            (width - 2 * pad).max(0),
            title,
            text_color,
            scale,
        );
        elements.push(solid_element(
            Rectangle::new(
                Point::from((x0, area.loc.y)),
                Size::from((width, TAB_BAR_HEIGHT)),
            ),
            background,
            scale,
        ));
    }

    elements
}

pub(crate) fn window_title(toplevel: &ToplevelSurface) -> Option<String> {
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

pub(crate) fn draw_text(
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

pub(crate) fn solid_element(
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
        '<' => [
            0b00010, 0b00100, 0b01000, 0b10000, 0b01000, 0b00100, 0b00010,
        ],
        '>' => [
            0b01000, 0b00100, 0b00010, 0b00001, 0b00010, 0b00100, 0b01000,
        ],
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
    fn snapshot_bar_with_workspaces_and_title() {
        let tags = [
            WorkspaceTag {
                name: "1",
                shown: true,
            },
            WorkspaceTag {
                name: "2",
                shown: false,
            },
            WorkspaceTag {
                name: "web",
                shown: false,
            },
        ];
        let elements = render_bar(
            rect(0, 0, 1000, 1000),
            &tags,
            "Tall",
            "oxmonad-snapshot",
            1.0,
        );

        crate::snapshot::assert_snapshot("built_in_bar", 1000, 1000, &elements);
    }

    #[test]
    fn snapshot_tabbed_bar() {
        let titles = [
            "left".to_string(),
            "middle".to_string(),
            "right".to_string(),
        ];
        // Three equal tabs across the top, the middle one focused.
        let elements = render_tabs(rect(0, 0, 1000, 1000), &titles, 1, 1.0);

        crate::snapshot::assert_snapshot("tabbed_bar", 1000, 1000, &elements);
    }
}
