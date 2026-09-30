//! Loaded fonts by family, and where installed fonts were found.
//!
//! Finding an installed font scans every font file on the system, which
//! takes ~100 ms with thousands of fonts. So: the embedded Fira Mono (the
//! default everywhere) is used without looking; other families are loaded
//! from the file remembered in `$XDG_CACHE_HOME/oxmonad/fonts` (else
//! `~/.cache/oxmonad/fonts`), and only scanned for when that file is
//! unknown or gone. Delete the cache file to look a family up again, e.g.
//! after installing a better match for a generic name like `monospace`.

use std::{
    collections::{
        BTreeMap,
        HashMap,
    },
    env,
    fmt::Write as _,
    fs,
    io,
    path::{
        Path,
        PathBuf,
    },
    rc::Rc,
};

use oxmonad_ui::{
    FontFile,
    Fonts,
};

/// Loaded fonts by requested family; each family is loaded once.
#[derive(Debug)]
pub struct FontCache {
    loaded: HashMap<String, Rc<Fonts>>,
    /// Where located font files are remembered; `None` disables it.
    file: Option<PathBuf>,
}

impl Default for FontCache {
    /// A cache remembering font files at the default location.
    fn default() -> Self {
        Self::new(default_path())
    }
}

impl FontCache {
    /// A cache remembering located font files in `file`.
    #[must_use]
    pub fn new(file: Option<PathBuf>) -> Self {
        Self {
            loaded: HashMap::new(),
            file,
        }
    }

    /// The fonts for `family`.
    pub fn get(&mut self, family: &str) -> Rc<Fonts> {
        if let Some(fonts) = self.loaded.get(family) {
            return Rc::clone(fonts);
        }
        let fonts = Rc::new(self.load(family));
        if fonts.family() != family && !Fonts::is_embedded_family(family) {
            tracing::info!(family, loaded = fonts.family(), "font fallback");
        }
        self.loaded.insert(family.to_owned(), Rc::clone(&fonts));
        fonts
    }

    fn load(&self, family: &str) -> Fonts {
        if Fonts::is_embedded_family(family) {
            return Fonts::embedded();
        }
        let mut known = self.file.as_deref().map(read).unwrap_or_default();
        if let Some(file) = known.get(family).filter(|file| file.exists())
            && let Ok(fonts) = Fonts::from_file(file)
        {
            return fonts;
        }
        let Some(file) = Fonts::locate(family) else {
            return Fonts::embedded();
        };
        match Fonts::from_file(&file) {
            Ok(fonts) => {
                known.insert(family.to_owned(), file);
                if let Some(path) = &self.file
                    && let Err(err) = write(path, &known)
                {
                    tracing::debug!(%err, path = %path.display(), "cannot remember font files");
                }
                fonts
            }
            Err(err) => {
                tracing::warn!(%err, family, "cannot load the font");
                Fonts::embedded()
            }
        }
    }
}

/// `$XDG_CACHE_HOME/oxmonad/fonts`, else `~/.cache/oxmonad/fonts`.
fn default_path() -> Option<PathBuf> {
    let base = env::var_os("XDG_CACHE_HOME")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))?;
    Some(base.join("oxmonad").join("fonts"))
}

/// Reads remembered font files: one `family<TAB>index<TAB>font family<TAB>path`
/// line each; unreadable files and malformed lines are ignored.
fn read(path: &Path) -> BTreeMap<String, FontFile> {
    let Ok(text) = fs::read_to_string(path) else {
        return BTreeMap::new();
    };
    text.lines()
        .filter_map(|line| {
            let mut fields = line.splitn(4, '\t');
            let requested = fields.next()?;
            let index = fields.next()?.parse().ok()?;
            let family = fields.next()?;
            let path = fields.next().filter(|p| !p.is_empty())?;
            Some((
                requested.to_owned(),
                FontFile {
                    path: PathBuf::from(path),
                    index,
                    family: family.to_owned(),
                },
            ))
        })
        .collect()
}

/// Writes `known` to `path` (via a temporary file, so readers never see a
/// partial file).
fn write(path: &Path, known: &BTreeMap<String, FontFile>) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut text = String::new();
    for (requested, file) in known {
        if requested.contains(['\t', '\n']) || file.family.contains(['\t', '\n']) {
            continue;
        }
        let _ = writeln!(
            text,
            "{requested}\t{}\t{}\t{}",
            file.index,
            file.family,
            file.path.display()
        );
    }
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, text)?;
    fs::rename(&temporary, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EMBEDDED_TTF: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../assets/FiraMono-Regular.ttf"
    );

    fn scratch(name: &str) -> PathBuf {
        let dir = env::temp_dir().join(format!("oxmonad-font-cache-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir.join("fonts")
    }

    #[test]
    fn fira_mono_needs_no_lookup() {
        let path = scratch("embedded");
        let mut cache = FontCache::new(Some(path.clone()));
        assert_eq!(cache.get("Fira Mono").family(), "Fira Mono");
        assert_eq!(cache.get("fira mono").family(), "Fira Mono");
        // Nothing was looked up, so nothing was remembered.
        assert!(!path.exists());
    }

    #[test]
    fn remembered_files_load_without_a_scan() {
        let path = scratch("remembered");
        let mut known = BTreeMap::new();
        // A family that isn't installed anywhere: only the remembered file
        // can provide it.
        known.insert(
            "No Such Family 1234".to_owned(),
            FontFile {
                path: PathBuf::from(EMBEDDED_TTF),
                index: 0,
                family: "Remembered Mono".to_owned(),
            },
        );
        write(&path, &known).expect("write");
        let mut cache = FontCache::new(Some(path.clone()));
        assert_eq!(cache.get("No Such Family 1234").family(), "Remembered Mono");
        let _ = fs::remove_dir_all(path.parent().expect("dir"));
    }

    #[test]
    fn missing_remembered_files_fall_back() {
        let path = scratch("missing");
        let mut known = BTreeMap::new();
        known.insert(
            "No Such Family 1234".to_owned(),
            FontFile {
                path: PathBuf::from("/nonexistent/font.ttf"),
                index: 0,
                family: "Gone".to_owned(),
            },
        );
        write(&path, &known).expect("write");
        let mut cache = FontCache::new(Some(path.clone()));
        // Rescanned: not installed, so the embedded font.
        assert_eq!(cache.get("No Such Family 1234").family(), "Fira Mono");
        let _ = fs::remove_dir_all(path.parent().expect("dir"));
    }

    #[test]
    fn cache_file_round_trips_and_skips_bad_lines() {
        let path = scratch("roundtrip");
        let mut known = BTreeMap::new();
        known.insert(
            "monospace".to_owned(),
            FontFile {
                path: PathBuf::from("/usr/share/fonts/a b/Mono.ttf"),
                index: 2,
                family: "DejaVu Sans Mono".to_owned(),
            },
        );
        write(&path, &known).expect("write");
        let mut text = fs::read_to_string(&path).expect("read");
        text.push_str("garbage\nx\tnot-a-number\ty\t/z\n");
        fs::write(&path, text).expect("append");
        assert_eq!(read(&path), known);
        let _ = fs::remove_dir_all(path.parent().expect("dir"));
    }
}
