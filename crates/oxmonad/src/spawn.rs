//! Running user commands.

use std::{
    os::unix::process::CommandExt,
    process::{
        Child,
        Command,
        Stdio,
    },
};

/// Spawned children that have not been reaped yet.
#[derive(Debug, Default)]
pub struct Children(Vec<Child>);

impl Children {
    /// Runs `command` with `sh -c` in its own process group, so it
    /// survives oxmonad restarts and does not get the terminal's or
    /// river's signals meant for oxmonad. The child is reaped by
    /// [`Self::reap`] on `SIGCHLD`.
    pub fn spawn(&mut self, command: &str) {
        if command.trim().is_empty() {
            return;
        }
        let spawned = Command::new("sh")
            .arg("-c")
            .arg(command)
            .stdin(Stdio::null())
            .process_group(0)
            .spawn();
        match spawned {
            Ok(child) => {
                tracing::debug!(%command, pid = child.id(), "spawned");
                self.0.push(child);
            }
            Err(err) => tracing::warn!(%command, %err, "failed to spawn"),
        }
    }

    /// Reaps every child that has exited.
    pub fn reap(&mut self) {
        self.0.retain_mut(|child| match child.try_wait() {
            Ok(Some(status)) => {
                tracing::debug!(pid = child.id(), %status, "child exited");
                false
            }
            Ok(None) => true,
            Err(err) => {
                tracing::warn!(pid = child.id(), %err, "cannot wait for child");
                false
            }
        });
    }

    /// The number of children still running.
    #[cfg(test)]
    fn len(&self) -> usize {
        self.0.len()
    }
}

#[cfg(test)]
mod tests {
    use std::time::{
        Duration,
        Instant,
    };

    use super::*;

    #[test]
    fn spawns_and_reaps() {
        let mut children = Children::default();
        children.spawn("   ");
        assert_eq!(children.len(), 0);
        children.spawn("exit 0");
        assert_eq!(children.len(), 1);
        let deadline = Instant::now() + Duration::from_secs(5);
        while children.len() > 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
            children.reap();
        }
        assert_eq!(children.len(), 0);
    }
}
