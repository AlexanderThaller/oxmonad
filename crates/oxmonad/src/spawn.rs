//! Running user commands.

use std::{
    os::unix::process::CommandExt,
    process::{
        Command,
        Stdio,
    },
};

use rustix::process::{
    WaitOptions,
    wait,
};

use crate::restart;

/// Spawned children that have not been reaped yet (their pids).
///
/// Children spawned before a restart stay children of the re-executed
/// process, which does not know them: [`Self::reap`] therefore reaps any
/// exited child, not only those in the list.
#[derive(Debug, Default)]
pub struct Children(Vec<u32>);

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
            .env_remove(restart::ENV)
            .process_group(0)
            .spawn();
        match spawned {
            Ok(child) => {
                tracing::debug!(%command, pid = child.id(), "spawned");
                self.0.push(child.id());
            }
            Err(err) => tracing::warn!(%command, %err, "failed to spawn"),
        }
    }

    /// Reaps every child that has exited.
    pub fn reap(&mut self) {
        // Stops when no exited child is left (`ECHILD`: no child at all).
        while let Ok(Some((pid, status))) = wait(WaitOptions::NOHANG) {
            let pid = pid.as_raw_nonzero().get().unsigned_abs();
            tracing::debug!(pid, ?status, "child exited");
            self.0.retain(|&child| child != pid);
        }
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
