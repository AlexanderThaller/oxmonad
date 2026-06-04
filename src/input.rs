use smithay::{
    backend::input::{
        AbsolutePositionEvent,
        Axis,
        AxisSource,
        ButtonState,
        Event,
        InputBackend,
        InputEvent,
        KeyState,
        KeyboardKeyEvent,
        PointerAxisEvent,
        PointerButtonEvent,
        PointerMotionEvent,
    },
    input::{
        keyboard::FilterResult,
        pointer::{
            AxisFrame,
            ButtonEvent,
            MotionEvent,
            RelativeMotionEvent,
        },
    },
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{
        Logical,
        Point,
        SERIAL_COUNTER,
    },
};

use crate::{
    keybindings::{
        self,
        Action,
    },
    state::Oxmonad,
};

impl Oxmonad {
    #[expect(
        clippy::too_many_lines,
        reason = "A single match dispatching every libinput/winit event kind; splitting it would \
                  only scatter the per-event handling without making it clearer."
    )]
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "InputEvent has a long tail of touch/tablet/gesture/switch variants this \
                  compositor does not yet handle; they are all intentionally ignored."
    )]
    pub fn process_input_event<I: InputBackend>(&mut self, event: InputEvent<I>) {
        match event {
            InputEvent::Keyboard { event, .. } => {
                let serial = SERIAL_COUNTER.next_serial();
                let time = Event::time_msec(&event);
                let key_state = event.state();

                // `input` runs the filter for every key event. We intercept
                // presses that match a keybinding (so they don't reach the
                // focused client) and forward everything else.
                let action = self
                    .seat
                    .get_keyboard()
                    .expect("the seat always has a keyboard; one is added during startup")
                    .input::<Action, _>(
                        self,
                        event.key_code(),
                        key_state,
                        serial,
                        time,
                        |_state, modifiers, handle| {
                            if key_state == KeyState::Pressed
                                && let Some(action) =
                                    keybindings::action_for(modifiers, handle.modified_sym())
                            {
                                return FilterResult::Intercept(action);
                            }
                            FilterResult::Forward
                        },
                    );

                if let Some(action) = action {
                    self.handle_keybinding(action);
                }
            }
            InputEvent::PointerMotion { event, .. } => {
                let serial = SERIAL_COUNTER.next_serial();
                let pointer = self
                    .seat
                    .get_pointer()
                    .expect("the seat always has a pointer; one is added during startup");

                // libinput (the TTY backend) reports relative deltas; accumulate
                // them onto the current location and keep the cursor on-screen.
                let location =
                    self.clamp_pointer_location(pointer.current_location() + event.delta());
                let under = self.surface_under(location);

                pointer.motion(
                    self,
                    under.clone(),
                    &MotionEvent {
                        location,
                        serial,
                        time: event.time_msec(),
                    },
                );
                pointer.relative_motion(
                    self,
                    under,
                    &RelativeMotionEvent {
                        delta: event.delta(),
                        delta_unaccel: event.delta_unaccel(),
                        utime: event.time(),
                    },
                );
                pointer.frame(self);
            }
            InputEvent::PointerMotionAbsolute { event, .. } => {
                let output = self
                    .space
                    .outputs()
                    .next()
                    .expect("at least one output is always mapped");

                let output_geo = self
                    .space
                    .output_geometry(output)
                    .expect("a mapped output always has geometry");

                let pos = event.position_transformed(output_geo.size) + output_geo.loc.to_f64();

                let serial = SERIAL_COUNTER.next_serial();

                let pointer = self
                    .seat
                    .get_pointer()
                    .expect("the seat always has a pointer; one is added during startup");

                let under = self.surface_under(pos);

                pointer.motion(
                    self,
                    under,
                    &MotionEvent {
                        location: pos,
                        serial,
                        time: event.time_msec(),
                    },
                );
                pointer.frame(self);
            }
            InputEvent::PointerButton { event, .. } => {
                let pointer = self
                    .seat
                    .get_pointer()
                    .expect("the seat always has a pointer; one is added during startup");
                let keyboard = self
                    .seat
                    .get_keyboard()
                    .expect("the seat always has a keyboard; one is added during startup");

                let serial = SERIAL_COUNTER.next_serial();

                let button = event.button_code();

                let button_state = event.state();

                if ButtonState::Pressed == button_state && !pointer.is_grabbed() {
                    if let Some((window, _loc)) = self
                        .space
                        .element_under(pointer.current_location())
                        .map(|(w, l)| (w.clone(), l))
                    {
                        self.space.raise_element(&window, true);
                        keyboard.set_focus(
                            self,
                            Some(
                                window
                                    .toplevel()
                                    .expect("managed windows are always wayland toplevels")
                                    .wl_surface()
                                    .clone(),
                            ),
                            serial,
                        );
                        self.space.elements().for_each(|window| {
                            window
                                .toplevel()
                                .expect("managed windows are always wayland toplevels")
                                .send_pending_configure();
                        });
                    } else {
                        self.space.elements().for_each(|window| {
                            window.set_activated(false);
                            window
                                .toplevel()
                                .expect("managed windows are always wayland toplevels")
                                .send_pending_configure();
                        });
                        keyboard.set_focus(self, Option::<WlSurface>::None, serial);
                    }
                }

                pointer.button(
                    self,
                    &ButtonEvent {
                        button,
                        state: button_state,
                        serial,
                        time: event.time_msec(),
                    },
                );
                pointer.frame(self);
            }
            InputEvent::PointerAxis { event, .. } => {
                let source = event.source();

                let horizontal_amount = event.amount(Axis::Horizontal).unwrap_or_else(|| {
                    event.amount_v120(Axis::Horizontal).unwrap_or(0.0) * 15.0 / 120.
                });
                let vertical_amount = event.amount(Axis::Vertical).unwrap_or_else(|| {
                    event.amount_v120(Axis::Vertical).unwrap_or(0.0) * 15.0 / 120.
                });
                let horizontal_amount_discrete = event.amount_v120(Axis::Horizontal);
                let vertical_amount_discrete = event.amount_v120(Axis::Vertical);

                let mut frame = AxisFrame::new(event.time_msec()).source(source);
                if horizontal_amount != 0.0 {
                    frame = frame.value(Axis::Horizontal, horizontal_amount);
                    if let Some(discrete) = horizontal_amount_discrete {
                        #[expect(
                            clippy::cast_possible_truncation,
                            reason = "v120 scroll steps are small integer multiples of 120 that \
                                      always fit in an i32."
                        )]
                        let discrete = discrete as i32;
                        frame = frame.v120(Axis::Horizontal, discrete);
                    }
                }
                if vertical_amount != 0.0 {
                    frame = frame.value(Axis::Vertical, vertical_amount);
                    if let Some(discrete) = vertical_amount_discrete {
                        #[expect(
                            clippy::cast_possible_truncation,
                            reason = "v120 scroll steps are small integer multiples of 120 that \
                                      always fit in an i32."
                        )]
                        let discrete = discrete as i32;
                        frame = frame.v120(Axis::Vertical, discrete);
                    }
                }

                if source == AxisSource::Finger {
                    if event.amount(Axis::Horizontal) == Some(0.0) {
                        frame = frame.stop(Axis::Horizontal);
                    }
                    if event.amount(Axis::Vertical) == Some(0.0) {
                        frame = frame.stop(Axis::Vertical);
                    }
                }

                let pointer = self
                    .seat
                    .get_pointer()
                    .expect("the seat always has a pointer; one is added during startup");
                pointer.axis(self, frame);
                pointer.frame(self);
            }
            _ => {}
        }
    }

    /// Clamps a pointer position to the bounding box of all mapped outputs, so
    /// relative motion can't drive the cursor off-screen.
    fn clamp_pointer_location(&self, pos: Point<f64, Logical>) -> Point<f64, Logical> {
        let mut bounds: Option<(f64, f64, f64, f64)> = None;
        for output in self.space.outputs() {
            let Some(geo) = self.space.output_geometry(output) else {
                continue;
            };
            let min_x = f64::from(geo.loc.x);
            let min_y = f64::from(geo.loc.y);
            let max_x = f64::from(geo.loc.x + geo.size.w);
            let max_y = f64::from(geo.loc.y + geo.size.h);
            bounds = Some(match bounds {
                Some((bmin_x, bmin_y, bmax_x, bmax_y)) => (
                    bmin_x.min(min_x),
                    bmin_y.min(min_y),
                    bmax_x.max(max_x),
                    bmax_y.max(max_y),
                ),
                None => (min_x, min_y, max_x, max_y),
            });
        }

        let Some((min_x, min_y, max_x, max_y)) = bounds else {
            return pos;
        };
        // Subtract one so the cursor stays within the last row/column of pixels.
        Point::from((
            pos.x.clamp(min_x, (max_x - 1.0).max(min_x)),
            pos.y.clamp(min_y, (max_y - 1.0).max(min_y)),
        ))
    }
}
