//! Locating and loading `config.toml`.

use std::{
    env,
    fmt,
    fs,
    io,
    path::{
        Path,
        PathBuf,
    },
};

use oxmonad_core::config::{
    Config,
    ConfigError,
};

/// The default config path: `$XDG_CONFIG_HOME/oxmonad/config.toml`, else
/// `~/.config/oxmonad/config.toml`.
#[must_use]
pub fn default_path() -> Option<PathBuf> {
    let base = env::var_os("XDG_CONFIG_HOME")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(base.join("oxmonad").join("config.toml"))
}

/// Why a config file could not be loaded.
#[derive(Debug)]
pub enum LoadError {
    /// The file exists but cannot be read.
    Io(PathBuf, io::Error),
    /// The file is invalid.
    Invalid(PathBuf, ConfigError),
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(path, err) => write!(f, "cannot read {}: {err}", path.display()),
            Self::Invalid(path, err) => write!(f, "invalid config {}: {err}", path.display()),
        }
    }
}

impl std::error::Error for LoadError {}

/// Loads the config at `path` merged over the defaults. A missing file
/// yields the defaults.
pub fn load(path: Option<&Path>) -> Result<Config, LoadError> {
    let Some(path) = path else {
        return Ok(Config::defaults());
    };
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            tracing::info!(path = %path.display(), "no config file, using defaults");
            return Ok(Config::defaults());
        }
        Err(err) => return Err(LoadError::Io(path.to_owned(), err)),
    };
    Config::from_toml(&text).map_err(|err| LoadError::Invalid(path.to_owned(), err))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_is_defaults() {
        let config = load(Some(Path::new("/nonexistent/oxmonad/config.toml"))).expect("defaults");
        assert_eq!(config, Config::defaults());
        assert_eq!(load(None).expect("defaults"), Config::defaults());
    }

    #[test]
    fn invalid_file_is_an_error() {
        let path = env::temp_dir().join(format!("oxmonad-config-{}.toml", std::process::id()));
        fs::write(&path, "terminal = 3\n").expect("write");
        let result = load(Some(&path));
        fs::remove_file(&path).expect("cleanup");
        assert!(matches!(result, Err(LoadError::Invalid(..))));
    }
}
