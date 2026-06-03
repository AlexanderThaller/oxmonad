# oxmonad development tasks.
#
# oxmonad currently runs *nested*: it opens a window inside your existing X11 or
# Wayland session (via Smithay's winit backend), so you can develop and test it
# without leaving your current desktop. Nothing is spawned on startup; launch a
# terminal inside the window with META+Shift+Enter.

# Show available recipes.
@_default:
    @just --list

# Run oxmonad in a nested window.
run *ARGS:
    RUST_LOG="${RUST_LOG:-info}" cargo run -- {{ ARGS }}

# Run with verbose Smithay/winit logging.
run-debug *ARGS:
    RUST_LOG="${RUST_LOG:-debug}" cargo run -- {{ ARGS }}

# Build the debug binary.
build:
    cargo build

# Build the optimized `deploy` profile.
build-deploy:
    cargo build --profile deploy

# Type-check without producing a binary.
check:
    cargo check

# Run clippy with the project lints.
clippy:
    cargo clippy --all-targets

# Format the code (needs the nightly rustfmt for the unstable options in .rustfmt.toml).
fmt:
    cargo fmt
