//! The WM's own keyboard, used while the prompt has keyboard focus. Keys
//! are interpreted with the compositor's keymap by the pure-Rust `kbvm`.

use std::{
    fs::File,
    os::{
        fd::OwnedFd,
        unix::fs::FileExt,
    },
};

use kbvm::{
    Components,
    GroupIndex,
    Keycode,
    ModifierMask,
    lookup::LookupTable,
    xkb::{
        Context,
        diagnostic::Diagnostic,
    },
};
use oxmonad_core::prompt::PromptKey;
use wayland_client::protocol::wl_keyboard::WlKeyboard;

use crate::keys;

/// A `wl_keyboard` and its keymap state.
pub struct Keyboard {
    proxy: WlKeyboard,
    table: Option<LookupTable>,
    components: Components,
}

impl std::fmt::Debug for Keyboard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Keyboard")
            .field("proxy", &self.proxy)
            .field("has_keymap", &self.table.is_some())
            .finish_non_exhaustive()
    }
}

impl Keyboard {
    /// Wraps a new `wl_keyboard`.
    #[must_use]
    pub fn new(keyboard: WlKeyboard) -> Self {
        Self {
            proxy: keyboard,
            table: None,
            components: Components::default(),
        }
    }

    /// Releases the `wl_keyboard`.
    pub fn release(self) {
        self.proxy.release();
    }

    /// Loads the keymap river sent (`wl_keyboard.keymap`). The fd may be
    /// shared with other clients, so it is read with `pread`.
    pub fn set_keymap(&mut self, fd: &OwnedFd, size: u32) {
        let file = match fd.try_clone() {
            Ok(fd) => File::from(fd),
            Err(err) => {
                tracing::warn!(%err, "cannot read the keymap");
                return;
            }
        };
        let mut text = vec![0; usize::try_from(size).unwrap_or(0)];
        if let Err(err) = file.read_exact_at(&mut text, 0) {
            tracing::warn!(%err, "cannot read the keymap");
            return;
        }
        while text.last() == Some(&0) {
            text.pop();
        }
        let mut diagnostics: Vec<Diagnostic> = Vec::new();
        match Context::default().keymap_from_bytes(&mut diagnostics, None, &text) {
            Ok(keymap) => self.table = Some(keymap.to_builder().build_lookup_table()),
            Err(err) => tracing::warn!(%err, "cannot parse the keymap"),
        }
    }

    /// `wl_keyboard.modifiers`.
    pub fn set_modifiers(&mut self, depressed: u32, latched: u32, locked: u32, group: u32) {
        self.components.mods_pressed = ModifierMask(depressed);
        self.components.mods_latched = ModifierMask(latched);
        self.components.mods_locked = ModifierMask(locked);
        self.components.group_locked = GroupIndex(group);
        self.components.update_effective();
    }

    /// The prompt key for a pressed evdev key code.
    #[must_use]
    pub fn prompt_key(&self, key: u32) -> Option<PromptKey> {
        let table = self.table.as_ref()?;
        let mods = self.components.mods;
        let ctrl = mods.0 & ModifierMask::CONTROL.0 != 0;
        let shift = mods.0 & ModifierMask::SHIFT.0 != 0;
        table
            .lookup(self.components.group, mods, Keycode::from_evdev(key))
            .into_iter()
            .find_map(|props| keys::prompt_key(props.keysym(), ctrl, shift))
    }
}
