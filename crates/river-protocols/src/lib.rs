//! Client-side bindings for the river 0.4 protocols oxmonad speaks.
//!
//! The XML files under `protocols/` are copied verbatim from river v0.4.8
//! (MIT licensed, © Isaac Freund). They are stable: river guarantees window
//! managers written against 0.4 keep working with 1.0 and later.

#![expect(unused_imports, reason = "generated code")]

macro_rules! protocol {
    ($module:ident, $path:literal $(, $dep:ident)*) => {
        pub mod $module {
            use wayland_client;
            use wayland_client::protocol::*;
            $(use crate::$dep::*;)*

            pub mod __interfaces {
                use wayland_client::protocol::__interfaces::*;
                $(use crate::$dep::__interfaces::*;)*
                wayland_scanner::generate_interfaces!($path);
            }
            use self::__interfaces::*;

            wayland_scanner::generate_client_code!($path);
        }
    };
}

protocol!(
    window_management,
    "protocols/river-window-management-v1.xml"
);
protocol!(
    xkb_bindings,
    "protocols/river-xkb-bindings-v1.xml",
    window_management
);
protocol!(
    layer_shell,
    "protocols/river-layer-shell-v1.xml",
    window_management
);
