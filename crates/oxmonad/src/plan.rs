//! Translating core's [`Arrangement`] into river requests.
//!
//! Everything here is pure: the requests go to a [`Requests`] sink, which
//! is the river connection in production and a recorder in tests. [`Applied`]
//! remembers what river was last told, so each sequence only sends what
//! changed (proposing unchanged dimensions would make clients reconfigure).

use std::collections::BTreeMap;

use oxmonad_core::{
    Arrangement,
    Border,
    Focus,
    Layer,
    OutputId,
    Placement,
    Rect,
    ScreenId,
    Size,
    WindowId,
};

/// A node in river's render list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Node {
    /// A window.
    Window(WindowId),
    /// The status bar.
    Bar,
    /// The tab strip of a screen.
    Tabs(ScreenId),
    /// The workspace prompt.
    Prompt,
    /// The layout picker.
    LayoutPicker,
    /// The key binding help overlay.
    Help,
}

impl Node {
    /// Whether this is an overlay that takes the keyboard (prompt, layout
    /// picker or help), stacked above everything.
    #[must_use]
    pub const fn is_overlay(self) -> bool {
        matches!(self, Self::Prompt | Self::LayoutPicker | Self::Help)
    }
}

/// The requests the arrangement translates into. Window-management state
/// (dimensions, tiling, fullscreen, focus) may only be changed in a manage
/// sequence; rendering state (visibility, borders, positions, stacking) in
/// either sequence.
pub trait Requests {
    /// `river_window_v1.propose_dimensions`.
    fn propose_dimensions(&mut self, window: WindowId, size: Size);
    /// `river_window_v1.set_tiled` with all edges (`true`) or none.
    fn set_tiled(&mut self, window: WindowId, tiled: bool);
    /// `river_window_v1.fullscreen` on `output`.
    fn fullscreen(&mut self, window: WindowId, output: OutputId);
    /// `river_window_v1.exit_fullscreen`.
    fn exit_fullscreen(&mut self, window: WindowId);
    /// `river_window_v1.inform_fullscreen` (`true`) or
    /// `inform_not_fullscreen`.
    fn inform_fullscreen(&mut self, window: WindowId, fullscreen: bool);
    /// Keyboard focus for every seat.
    fn focus(&mut self, focus: Focus);
    /// `river_window_v1.show`.
    fn show(&mut self, window: WindowId);
    /// `river_window_v1.hide`.
    fn hide(&mut self, window: WindowId);
    /// `river_window_v1.set_borders` (`None` disables them).
    fn set_borders(&mut self, window: WindowId, border: Option<Border>);
    /// `river_node_v1.set_position`.
    fn set_position(&mut self, node: Node, x: i32, y: i32);
    /// `river_node_v1.place_top`.
    fn place_top(&mut self, node: Node);
}

/// What river was last told about one window.
#[derive(Debug, Clone, PartialEq, Eq)]
struct AppliedWindow {
    proposed: Option<Size>,
    /// river shows new windows until told otherwise.
    shown: bool,
    /// river draws no borders until told otherwise.
    border: Option<Border>,
    tiled: Option<bool>,
    /// The output river made the window fullscreen on.
    fullscreen: Option<OutputId>,
    /// Whether the client was told it is fullscreen.
    informed_fullscreen: bool,
}

impl Default for AppliedWindow {
    fn default() -> Self {
        Self {
            proposed: None,
            shown: true,
            border: None,
            tiled: None,
            fullscreen: None,
            informed_fullscreen: false,
        }
    }
}

/// The window-management state river was last told.
#[derive(Debug, Clone, Default)]
pub struct Applied {
    windows: BTreeMap<WindowId, AppliedWindow>,
    focus: Option<Focus>,
}

impl Applied {
    /// Forgets a closed window.
    pub fn remove(&mut self, window: WindowId) {
        self.windows.remove(&window);
    }

    /// Makes the next manage sequence resend keyboard focus (river or a
    /// layer surface changed it behind our back).
    pub fn invalidate_focus(&mut self) {
        self.focus = None;
    }

    /// Emits the requests of a manage sequence that bring river from the
    /// applied state to `arrangement`.
    pub fn manage(&mut self, arrangement: &Arrangement, out: &mut impl Requests) {
        for placement in &arrangement.windows {
            let window = placement.window;
            let applied = self.windows.entry(window).or_default();
            // A virtual screen's fullscreen window is placed like a float
            // covering the screen; only unsplit outputs use river's
            // fullscreen, which covers the whole output.
            let on_output = (placement.layer == Layer::Fullscreen).then_some(placement.output);
            if applied.fullscreen != on_output {
                if let Some(output) = on_output {
                    out.fullscreen(window, output);
                } else {
                    out.exit_fullscreen(window);
                    // river forgets the dimensions while fullscreen.
                    applied.proposed = None;
                }
                applied.fullscreen = on_output;
            }
            let informed = placement.layer.is_fullscreen();
            if applied.informed_fullscreen != informed {
                out.inform_fullscreen(window, informed);
                applied.informed_fullscreen = informed;
            }
            if on_output.is_none() {
                let size = Size::new(placement.rect.w.max(1), placement.rect.h.max(1));
                if applied.proposed != Some(size) {
                    out.propose_dimensions(window, size);
                    applied.proposed = Some(size);
                }
                let tiled = placement.layer == Layer::Tiled;
                if applied.tiled != Some(tiled) {
                    out.set_tiled(window, tiled);
                    applied.tiled = Some(tiled);
                }
            }
            if !applied.shown {
                out.show(window);
                applied.shown = true;
            }
            if applied.border != placement.border {
                out.set_borders(window, placement.border);
                applied.border = placement.border;
            }
        }
        for &window in &arrangement.hidden {
            let applied = self.windows.entry(window).or_default();
            if applied.shown {
                out.hide(window);
                applied.shown = false;
            }
        }
        if self.focus != Some(arrangement.focus) {
            out.focus(arrangement.focus);
            self.focus = Some(arrangement.focus);
        }
    }
}

/// A WM-drawn surface and where it goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShellPlacement {
    /// Which surface.
    pub node: Node,
    /// Its logical rectangle.
    pub rect: Rect,
}

/// Emits the positions and stacking of a render sequence: windows bottom
/// to top, then the bar and tab strips, then fullscreen windows (which
/// cover the bar, also those fullscreen on a virtual screen) and the prompt
/// or layout picker on top of everything. `dimensions` are the window sizes
/// river reported.
pub fn render(
    arrangement: &Arrangement,
    dimensions: impl Fn(WindowId) -> Option<Size>,
    shells: &[ShellPlacement],
    out: &mut impl Requests,
) {
    let (fullscreen, normal): (Vec<&Placement>, Vec<&Placement>) = arrangement
        .windows
        .iter()
        .partition(|p| p.layer.is_fullscreen());
    for placement in normal {
        let (x, y) = window_position(placement, dimensions(placement.window));
        let node = Node::Window(placement.window);
        out.set_position(node, x, y);
        out.place_top(node);
    }
    let (prompt, bars): (Vec<&ShellPlacement>, Vec<&ShellPlacement>) =
        shells.iter().partition(|s| s.node.is_overlay());
    for shell in bars {
        out.set_position(shell.node, shell.rect.x, shell.rect.y);
        out.place_top(shell.node);
    }
    for placement in fullscreen {
        let node = Node::Window(placement.window);
        // river positions windows it made fullscreen itself.
        if placement.layer == Layer::VirtualFullscreen {
            out.set_position(node, placement.rect.x, placement.rect.y);
        }
        out.place_top(node);
    }
    for shell in prompt {
        out.set_position(shell.node, shell.rect.x, shell.rect.y);
        out.place_top(shell.node);
    }
}

/// Where a window's content goes. Tiled windows smaller than their slot
/// (terminals snap to their cell grid) are centred in it; larger ones stay
/// at the slot's top-left corner. Floating windows keep their position.
#[must_use]
pub fn window_position(placement: &Placement, actual: Option<Size>) -> (i32, i32) {
    let rect = placement.rect;
    match (placement.layer, actual) {
        (Layer::Tiled, Some(size)) => (
            rect.x + ((rect.w - size.w) / 2).max(0),
            rect.y + ((rect.h - size.h) / 2).max(0),
        ),
        _ => (rect.x, rect.y),
    }
}

/// A river border color component: an 8-bit straight channel scaled to
/// the 32-bit premultiplied range.
#[must_use]
pub fn border_channel(value: u8, alpha: u8) -> u32 {
    let premultiplied = u32::from(value) * u32::from(alpha) / 255;
    premultiplied * 0x0101_0101
}

#[cfg(test)]
mod tests {
    use oxmonad_core::{
        ScreenArrangement,
        config::Color,
    };

    use super::*;

    /// Records requests as strings.
    #[derive(Debug, Default)]
    struct Recorder(Vec<String>);

    impl Requests for Recorder {
        fn propose_dimensions(&mut self, window: WindowId, size: Size) {
            self.0
                .push(format!("propose {} {}x{}", window.0, size.w, size.h));
        }
        fn set_tiled(&mut self, window: WindowId, tiled: bool) {
            self.0.push(format!("tiled {} {tiled}", window.0));
        }
        fn fullscreen(&mut self, window: WindowId, output: OutputId) {
            self.0.push(format!("fullscreen {} {}", window.0, output.0));
        }
        fn exit_fullscreen(&mut self, window: WindowId) {
            self.0.push(format!("exit-fullscreen {}", window.0));
        }
        fn inform_fullscreen(&mut self, window: WindowId, fullscreen: bool) {
            self.0
                .push(format!("inform-fullscreen {} {fullscreen}", window.0));
        }
        fn focus(&mut self, focus: Focus) {
            self.0.push(format!("focus {focus:?}"));
        }
        fn show(&mut self, window: WindowId) {
            self.0.push(format!("show {}", window.0));
        }
        fn hide(&mut self, window: WindowId) {
            self.0.push(format!("hide {}", window.0));
        }
        fn set_borders(&mut self, window: WindowId, border: Option<Border>) {
            self.0.push(format!(
                "borders {} {}",
                window.0,
                border.map_or(0, |b| b.width)
            ));
        }
        fn set_position(&mut self, node: Node, x: i32, y: i32) {
            self.0.push(format!("position {node:?} {x},{y}"));
        }
        fn place_top(&mut self, node: Node) {
            self.0.push(format!("top {node:?}"));
        }
    }

    const RED: Color = Color::rgb(255, 0, 0);

    fn placement(id: u64, rect: Rect, layer: Layer) -> Placement {
        Placement {
            window: WindowId(id),
            screen: ScreenId(1),
            output: OutputId(1),
            rect,
            layer,
            border: (!layer.is_fullscreen()).then_some(Border {
                width: 1,
                color: RED,
            }),
        }
    }

    fn arrangement(windows: Vec<Placement>, hidden: Vec<u64>, focus: Focus) -> Arrangement {
        Arrangement {
            windows,
            hidden: hidden.into_iter().map(WindowId).collect(),
            focus,
            screens: vec![ScreenArrangement {
                screen: ScreenId(1),
                output: OutputId(1),
                tab_strip: None,
            }],
            bar: None,
            prompt: None,
            layout_picker: None,
            help: None,
        }
    }

    #[test]
    fn manage_sends_everything_once() {
        let arr = arrangement(
            vec![
                placement(1, Rect::new(1, 25, 638, 694), Layer::Tiled),
                placement(2, Rect::new(641, 25, 638, 694), Layer::Tiled),
                placement(3, Rect::new(100, 100, 300, 200), Layer::Floating),
            ],
            vec![4],
            Focus::Window(WindowId(3)),
        );
        let mut applied = Applied::default();
        let mut out = Recorder::default();
        applied.manage(&arr, &mut out);
        assert_eq!(
            out.0,
            [
                "propose 1 638x694",
                "tiled 1 true",
                "borders 1 1",
                "propose 2 638x694",
                "tiled 2 true",
                "borders 2 1",
                "propose 3 300x200",
                "tiled 3 false",
                "borders 3 1",
                "hide 4",
                "focus Window(WindowId(3))",
            ]
        );

        let mut again = Recorder::default();
        applied.manage(&arr, &mut again);
        assert!(again.0.is_empty(), "{:?}", again.0);
    }

    #[test]
    fn manage_sends_only_changes() {
        let mut applied = Applied::default();
        let first = arrangement(
            vec![placement(1, Rect::new(0, 0, 100, 100), Layer::Tiled)],
            vec![2],
            Focus::Window(WindowId(1)),
        );
        applied.manage(&first, &mut Recorder::default());

        // Window 2 becomes visible, window 1 gets smaller.
        let second = arrangement(
            vec![
                placement(1, Rect::new(0, 0, 50, 100), Layer::Tiled),
                placement(2, Rect::new(50, 0, 50, 100), Layer::Tiled),
            ],
            vec![],
            Focus::Window(WindowId(2)),
        );
        let mut out = Recorder::default();
        applied.manage(&second, &mut out);
        assert_eq!(
            out.0,
            [
                "propose 1 50x100",
                "propose 2 50x100",
                "tiled 2 true",
                "show 2",
                "borders 2 1",
                "focus Window(WindowId(2))",
            ]
        );
    }

    #[test]
    fn fullscreen_round_trip() {
        let mut applied = Applied::default();
        let tiled = arrangement(
            vec![placement(1, Rect::new(0, 0, 100, 100), Layer::Tiled)],
            vec![],
            Focus::Window(WindowId(1)),
        );
        applied.manage(&tiled, &mut Recorder::default());

        let full = arrangement(
            vec![placement(1, Rect::new(0, 0, 200, 200), Layer::Fullscreen)],
            vec![],
            Focus::Window(WindowId(1)),
        );
        let mut out = Recorder::default();
        applied.manage(&full, &mut out);
        assert_eq!(
            out.0,
            ["fullscreen 1 1", "inform-fullscreen 1 true", "borders 1 0"]
        );

        let mut out = Recorder::default();
        applied.manage(&tiled, &mut out);
        assert_eq!(
            out.0,
            [
                "exit-fullscreen 1",
                "inform-fullscreen 1 false",
                "propose 1 100x100",
                "borders 1 1"
            ]
        );
    }

    #[test]
    fn virtual_fullscreen_is_a_float_the_client_is_told_about() {
        let mut applied = Applied::default();
        let tiled = arrangement(
            vec![placement(1, Rect::new(960, 24, 960, 1056), Layer::Tiled)],
            vec![],
            Focus::Window(WindowId(1)),
        );
        applied.manage(&tiled, &mut Recorder::default());
        let full = arrangement(
            vec![placement(
                1,
                Rect::new(960, 0, 960, 1080),
                Layer::VirtualFullscreen,
            )],
            vec![],
            Focus::Window(WindowId(1)),
        );
        let mut out = Recorder::default();
        applied.manage(&full, &mut out);
        assert_eq!(
            out.0,
            [
                "inform-fullscreen 1 true",
                "propose 1 960x1080",
                "tiled 1 false",
                "borders 1 0"
            ]
        );
        // Unsplitting turns it into a river fullscreen window.
        let whole = arrangement(
            vec![placement(1, Rect::new(0, 0, 1920, 1080), Layer::Fullscreen)],
            vec![],
            Focus::Window(WindowId(1)),
        );
        let mut out = Recorder::default();
        applied.manage(&whole, &mut out);
        assert_eq!(out.0, ["fullscreen 1 1"]);
        let mut out = Recorder::default();
        applied.manage(&full, &mut out);
        assert_eq!(out.0, ["exit-fullscreen 1", "propose 1 960x1080"]);

        let mut out = Recorder::default();
        render(&full, |_| Some(Size::new(960, 1080)), &[], &mut out);
        assert_eq!(
            out.0,
            [
                "position Window(WindowId(1)) 960,0",
                "top Window(WindowId(1))"
            ]
        );
    }

    #[test]
    fn focus_is_resent_after_invalidation() {
        let mut applied = Applied::default();
        let arr = arrangement(vec![], vec![], Focus::None);
        applied.manage(&arr, &mut Recorder::default());
        applied.invalidate_focus();
        let mut out = Recorder::default();
        applied.manage(&arr, &mut out);
        assert_eq!(out.0, ["focus None"]);
    }

    #[test]
    fn prompt_focus() {
        let mut applied = Applied::default();
        let mut out = Recorder::default();
        applied.manage(&arrangement(vec![], vec![], Focus::Prompt), &mut out);
        assert_eq!(out.0, ["focus Prompt"]);
    }

    #[test]
    fn render_stacks_shells_between_layers() {
        let arr = arrangement(
            vec![
                placement(1, Rect::new(0, 24, 640, 696), Layer::Tiled),
                placement(2, Rect::new(100, 100, 300, 200), Layer::Floating),
                placement(3, Rect::new(0, 0, 1280, 720), Layer::Fullscreen),
            ],
            vec![],
            Focus::Window(WindowId(3)),
        );
        let shells = [
            ShellPlacement {
                node: Node::Prompt,
                rect: Rect::new(0, 692, 1280, 28),
            },
            ShellPlacement {
                node: Node::Bar,
                rect: Rect::new(0, 0, 1280, 24),
            },
        ];
        let mut out = Recorder::default();
        render(
            &arr,
            |w| (w == WindowId(1)).then_some(Size::new(630, 690)),
            &shells,
            &mut out,
        );
        assert_eq!(
            out.0,
            [
                "position Window(WindowId(1)) 5,27",
                "top Window(WindowId(1))",
                "position Window(WindowId(2)) 100,100",
                "top Window(WindowId(2))",
                "position Bar 0,0",
                "top Bar",
                "top Window(WindowId(3))",
                "position Prompt 0,692",
                "top Prompt",
            ]
        );
    }

    #[test]
    fn centring() {
        let tiled = placement(1, Rect::new(10, 20, 100, 50), Layer::Tiled);
        assert_eq!(window_position(&tiled, None), (10, 20));
        assert_eq!(window_position(&tiled, Some(Size::new(100, 50))), (10, 20));
        assert_eq!(window_position(&tiled, Some(Size::new(90, 41))), (15, 24));
        // Too large: pinned to the slot's corner.
        assert_eq!(window_position(&tiled, Some(Size::new(200, 60))), (10, 20));
        let float = placement(2, Rect::new(10, 20, 100, 50), Layer::Floating);
        assert_eq!(window_position(&float, Some(Size::new(50, 25))), (10, 20));
    }

    #[test]
    fn border_channels() {
        assert_eq!(border_channel(0xff, 0xff), 0xffff_ffff);
        assert_eq!(border_channel(0, 0xff), 0);
        assert_eq!(border_channel(0xff, 0), 0);
        assert_eq!(border_channel(0x80, 0xff), 0x8080_8080);
    }
}
