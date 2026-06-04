mod compositor;
mod decoration;
mod layer_shell;
mod xdg_shell;

use smithay::{
    delegate_data_device,
    delegate_output,
    delegate_seat,
    input::{
        Seat,
        SeatHandler,
        SeatState,
    },
    reexports::wayland_server::{
        Resource,
        protocol::wl_surface::WlSurface,
    },
    wayland::{
        output::OutputHandler,
        selection::{
            SelectionHandler,
            data_device::{
                ClientDndGrabHandler,
                DataDeviceHandler,
                DataDeviceState,
                ServerDndGrabHandler,
                set_data_device_focus,
            },
        },
    },
};

use crate::Oxmonad;

// Wl Seat
//

impl SeatHandler for Oxmonad {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<Oxmonad> {
        &mut self.seat_state
    }

    fn cursor_image(
        &mut self,
        _seat: &Seat<Self>,
        image: smithay::input::pointer::CursorImageStatus,
    ) {
        // Remember the client's requested cursor so the TTY/DRM backend can draw
        // it. The nested winit backend ignores this and relies on the host.
        self.cursor_status = image;
    }

    fn focus_changed(&mut self, seat: &Seat<Self>, focused: Option<&WlSurface>) {
        let dh = &self.display_handle;
        let client = focused.and_then(|s| dh.get_client(s.id()).ok());
        set_data_device_focus(dh, seat, client);
    }
}

delegate_seat!(Oxmonad);

// Wl Data Device
//

impl SelectionHandler for Oxmonad {
    type SelectionUserData = ();
}

impl DataDeviceHandler for Oxmonad {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.data_device_state
    }
}

impl ClientDndGrabHandler for Oxmonad {}
impl ServerDndGrabHandler for Oxmonad {}

delegate_data_device!(Oxmonad);

// Wl Output & Xdg Output
//

impl OutputHandler for Oxmonad {}
delegate_output!(Oxmonad);
