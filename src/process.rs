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

/// Session variables to publish to the wider user session, so clients and user
/// services can find this compositor and identify the session.
const SESSION_VARS: [&str; 3] = ["WAYLAND_DISPLAY", "XDG_CURRENT_DESKTOP", "XDG_SESSION_TYPE"];

/// Publishes our session environment to the systemd user manager and the D-Bus
/// activation environment, so user services and desktop portals started later
/// can connect to this compositor.
///
/// Best-effort and only meaningful when oxmonad *is* the session (the TTY
/// backend); it must not be called from the nested backend, where it would
/// clobber the host session's variables. Missing `systemctl` / D-Bus (or any
/// failure) is logged, not fatal. Only variables actually present in our
/// environment are published.
pub fn import_environment() {
    let vars: Vec<&str> = SESSION_VARS
        .into_iter()
        .filter(|var| std::env::var_os(var).is_some())
        .collect();
    if vars.is_empty() {
        return;
    }

    let mut systemctl_args = vec!["--user", "import-environment"];
    systemctl_args.extend_from_slice(&vars);
    run_best_effort("systemctl", &systemctl_args);

    let mut dbus_args = vec!["--systemd"];
    dbus_args.extend_from_slice(&vars);
    run_best_effort("dbus-update-activation-environment", &dbus_args);
}

/// Runs `program` with `args` to completion, logging (but not propagating) any
/// failure.
fn run_best_effort(program: &str, args: &[&str]) {
    match std::process::Command::new(program).args(args).status() {
        Ok(status) if status.success() => {}
        Ok(status) => tracing::warn!(%program, ?status, "environment import command failed"),
        Err(err) => tracing::warn!(%program, %err, "could not run environment import command"),
    }
}
