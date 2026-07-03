use smithay::{
    backend::renderer::utils::on_commit_buffer_handler,
    delegate_compositor,
    delegate_shm,
    reexports::wayland_server::{
        Client,
        protocol::{
            wl_buffer,
            wl_surface::WlSurface,
        },
    },
    wayland::{
        buffer::BufferHandler,
        compositor::{
            CompositorClientState,
            CompositorHandler,
            CompositorState,
            get_parent,
            is_sync_subsurface,
        },
        shm::{
            ShmHandler,
            ShmState,
        },
    },
};

use super::xdg_shell;
use crate::{
    Oxmonad,
    grabs::resize_grab,
    state::ClientState,
};

impl CompositorHandler for Oxmonad {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        &client
            .get_data::<ClientState>()
            .expect("every client is initialized with ClientState")
            .compositor_state
    }

    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);
        if !is_sync_subsurface(surface) {
            let mut root = surface.clone();
            while let Some(parent) = get_parent(&root) {
                root = parent;
            }
            if let Some(window) = self.space.elements().find(|w| {
                w.toplevel()
                    .expect("managed windows are always wayland toplevels")
                    .wl_surface()
                    == &root
            }) {
                window.on_commit();
            }
        }

        xdg_shell::handle_commit(&mut self.popups, &self.space, surface);
        resize_grab::handle_commit(&mut self.space, surface);
        super::layer_shell::handle_commit(self, surface);
        // Manage rules (float/sink) run once the client has committed its
        // title/app_id.
        self.apply_pending_manage(surface);
    }
}

impl BufferHandler for Oxmonad {
    fn buffer_destroyed(&mut self, _buffer: &wl_buffer::WlBuffer) {}
}

impl ShmHandler for Oxmonad {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

delegate_compositor!(Oxmonad);
delegate_shm!(Oxmonad);
