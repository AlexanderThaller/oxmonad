//! Helpers for launching child processes connected to this compositor.

/// Spawns `command` as a detached child process.
///
/// The child inherits `WAYLAND_DISPLAY` from our environment (set during
/// startup), so it connects back to this compositor.
pub fn spawn(command: &str) {
    if command.trim().is_empty() {
        return;
    }

    if let Err(err) = std::process::Command::new(command).spawn() {
        tracing::warn!(%command, %err, "failed to spawn command");
    }
}

/// Runs `command` through `sh -c` as a detached child.
///
/// Used for configurable commands that may need shell features such as pipes,
/// arguments, or redirection. The shell inherits `WAYLAND_DISPLAY`.
pub fn spawn_shell(command: &str, description: &str) {
    if command.trim().is_empty() {
        return;
    }

    if let Err(err) = std::process::Command::new("sh")
        .arg("-c")
        .arg(command)
        .spawn()
    {
        tracing::warn!(%command, %description, %err, "failed to spawn shell command");
    }
}
