//! Restarting in place (`restart`, xmonad's `Mod+q`).
//!
//! The running oxmonad checks that the binary it was started from still
//! works (`--check-config`), saves [`Wm::snapshot`](oxmonad_core::Wm) as
//! JSON in `$XDG_RUNTIME_DIR`, stops window management and re-executes
//! that binary in the same process with [`ENV`] naming the file. A binary
//! in the Nix store never changes, so one started from there is looked up
//! in `PATH` instead (see [`Exe::binary`]). river
//! keeps running and keeps the windows; it only sees the window manager
//! disconnect and a new one bind, and announces every window to it again.
//! The new process reads the snapshot back with [`take`].
//!
//! Every file descriptor oxmonad opens is close-on-exec, so the new
//! process starts clean; spawned children stay its children.

use std::{
    env,
    ffi::{
        OsStr,
        OsString,
    },
    fs::{
        self,
        OpenOptions,
    },
    io::{
        self,
        Write,
    },
    os::unix::{
        fs::{
            OpenOptionsExt,
            PermissionsExt,
        },
        process::CommandExt,
    },
    path::{
        Path,
        PathBuf,
    },
    process::Command,
};

use oxmonad_core::Snapshot;

/// The environment variable naming the snapshot file to restore.
pub const ENV: &str = "OXMONAD_RESTORE";

/// Binaries below this directory are never replaced in place.
const NIX_STORE: &str = "/nix/store";

/// How this process was started, to start the same way again.
#[derive(Debug, Clone)]
pub struct Exe {
    /// The binary, resolved at startup: a rebuild replaces the file at this
    /// path, and the restart picks up the new one.
    path: PathBuf,
    /// `argv[0]`.
    arg0: OsString,
    /// The other arguments.
    args: Vec<OsString>,
}

impl Exe {
    /// The running binary and its arguments.
    pub fn current() -> io::Result<Self> {
        let path = env::current_exe()?;
        // Linux marks a replaced binary `… (deleted)`.
        let path = match path.to_str().and_then(|p| p.strip_suffix(" (deleted)")) {
            Some(original) => PathBuf::from(original),
            None => path,
        };
        let mut argv = env::args_os();
        let first = argv.next().unwrap_or_else(|| path.clone().into_os_string());
        Ok(Self {
            path,
            arg0: first,
            args: argv.collect(),
        })
    }

    /// The binary a restart runs: the one this process was started from,
    /// or, if that is in the Nix store, the first binary of the same name
    /// in `PATH` (on NixOS `/run/current-system/sw/bin` or the user's
    /// profile, which a rebuild points at the new version). Falls back to
    /// the started one when `PATH` has none.
    #[must_use]
    pub fn binary(&self) -> PathBuf {
        resolve(&self.path, env::var_os("PATH").as_deref())
    }

    /// Runs [`Self::binary`] with `--check-config`: whether it starts and
    /// accepts the configuration. Blocks until it exits and returns the
    /// checked binary, for [`Self::exec`].
    pub fn check(&self) -> Result<PathBuf, String> {
        let binary = self.binary();
        let output = Command::new(&binary)
            .arg0(&self.arg0)
            .args(&self.args)
            .arg("--check-config")
            .env_remove(ENV)
            .output()
            .map_err(|err| format!("cannot run {}: {err}", binary.display()))?;
        if output.status.success() {
            Ok(binary)
        } else {
            Err(format!(
                "{} --check-config failed ({}): {}",
                binary.display(),
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            ))
        }
    }

    /// Replaces this process with `binary` (from [`Self::check`]),
    /// restoring `snapshot`. Only returns if that fails.
    pub fn exec(&self, binary: &Path, snapshot: &Path) -> io::Error {
        Command::new(binary)
            .arg0(&self.arg0)
            .args(&self.args)
            .env(ENV, snapshot)
            .exec()
    }
}

/// The binary to restart from `started`, given the `PATH` value
/// `path_var` (see [`Exe::binary`]).
fn resolve(started: &Path, path_var: Option<&OsStr>) -> PathBuf {
    let (true, Some(name), Some(path_var)) = (
        started.starts_with(NIX_STORE),
        started.file_name(),
        path_var,
    ) else {
        return started.to_owned();
    };
    env::split_paths(path_var)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join(name))
        .find(|candidate| is_executable(candidate))
        .unwrap_or_else(|| started.to_owned())
}

/// Whether `path` is (or links to) a file with an execute bit set.
fn is_executable(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

/// Writes `snapshot` to a private file in `$XDG_RUNTIME_DIR` and returns
/// its path.
pub fn save(snapshot: &Snapshot) -> io::Result<PathBuf> {
    let dir = env::var_os("XDG_RUNTIME_DIR")
        .filter(|dir| !dir.is_empty())
        .map_or_else(env::temp_dir, PathBuf::from);
    save_in(&dir, snapshot)
}

fn save_in(dir: &Path, snapshot: &Snapshot) -> io::Result<PathBuf> {
    let path = dir.join(format!("oxmonad-restart-{}.json", std::process::id()));
    let json = serde_json::to_vec(snapshot).map_err(io::Error::other)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&path)?;
    file.write_all(&json)?;
    Ok(path)
}

/// Removes a snapshot file that will not be restored.
pub fn discard(path: &Path) {
    if let Err(err) = fs::remove_file(path) {
        tracing::warn!(path = %path.display(), %err, "cannot remove the restart snapshot");
    }
}

/// The snapshot the previous process left in [`ENV`], if any. The file is
/// removed; an unreadable or invalid snapshot is logged and ignored.
pub fn take() -> Option<Snapshot> {
    let path = PathBuf::from(env::var_os(ENV).filter(|p| !p.is_empty())?);
    let read = fs::read(&path);
    discard(&path);
    let parsed = read
        .map_err(|err| err.to_string())
        .and_then(|bytes| serde_json::from_slice(&bytes).map_err(|err| err.to_string()));
    match parsed {
        Ok(snapshot) => Some(snapshot),
        Err(err) => {
            tracing::warn!(path = %path.display(), %err, "ignoring the restart snapshot");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use oxmonad_core::{
        Wm,
        config::Config,
    };

    use super::*;

    #[test]
    fn saved_snapshots_read_back() {
        let wm = Wm::new(Config::defaults());
        let snapshot = wm.snapshot(|w| Some(w.0.to_string()));
        let dir = env::temp_dir().join(format!("oxmonad-restart-test-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("temp dir");
        let path = save_in(&dir, &snapshot).expect("saved");
        let json = fs::read(&path).expect("readable");
        discard(&path);
        assert!(!path.exists());
        fs::remove_dir(&dir).expect("temp dir removed");
        let read: Snapshot = serde_json::from_slice(&json).expect("valid JSON");
        assert_eq!(read, snapshot);
    }

    #[test]
    fn nix_store_binaries_restart_from_path() {
        let dir = env::temp_dir().join(format!("oxmonad-resolve-test-{}", std::process::id()));
        let (empty, bin) = (dir.join("empty"), dir.join("bin"));
        fs::create_dir_all(&empty).expect("temp dir");
        fs::create_dir_all(&bin).expect("temp dir");
        let installed = bin.join("oxmonad");
        fs::write(&installed, "").expect("written");
        let not_executable = empty.join("oxmonad");
        fs::write(&not_executable, "").expect("written");
        fs::set_permissions(&installed, fs::Permissions::from_mode(0o755)).expect("chmod");
        let path_var = env::join_paths([&empty, &bin]).expect("PATH");

        let store = Path::new("/nix/store/0000-oxmonad-0.1.0/bin/oxmonad");
        assert_eq!(resolve(store, Some(&path_var)), installed);
        // Nothing in PATH, or no PATH: the started binary.
        let only_empty = env::join_paths([&empty]).expect("PATH");
        assert_eq!(resolve(store, Some(&only_empty)), store);
        assert_eq!(resolve(store, None), store);
        // Elsewhere the started path is kept: a rebuild replaces that file.
        let local = Path::new("/usr/local/bin/oxmonad");
        assert_eq!(resolve(local, Some(&path_var)), local);

        fs::remove_dir_all(&dir).expect("temp dir removed");
    }
}
