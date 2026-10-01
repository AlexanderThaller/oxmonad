//! A small freedesktop icon theme lookup
//! (<https://specifications.freedesktop.org/icon-theme-spec/latest/>).
//!
//! Themes are searched in order: the user's GTK icon theme (from
//! `gtk-3.0/settings.ini` or `gtk-4.0/settings.ini`) and the themes it
//! inherits, then Adwaita, then hicolor; the pixmap directories last.

use std::{
    cmp::Reverse,
    collections::HashMap,
    env,
    fs,
    path::{
        Path,
        PathBuf,
    },
};

/// File extensions tried for every candidate, in order of preference.
const EXTENSIONS: [&str; 2] = ["svg", "png"];

/// Themes always searched after the user's.
const FALLBACK_THEMES: [&str; 2] = ["Adwaita", "hicolor"];

/// How deep an application's `IconThemePath` is searched when it is not
/// laid out as a theme.
const MAX_SCAN_DEPTH: usize = 4;

/// How a theme directory's icons may be scaled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DirKind {
    Fixed,
    Scalable,
    Threshold,
}

/// One `Directories` entry of an `index.theme`.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ThemeDir {
    path: String,
    size: u32,
    scale: u32,
    min: u32,
    max: u32,
    threshold: u32,
    kind: DirKind,
}

impl ThemeDir {
    /// The range of pixel sizes the directory's icons are good for.
    fn pixel_range(&self) -> (u32, u32) {
        let (min, max) = match self.kind {
            DirKind::Fixed => (self.size, self.size),
            DirKind::Scalable => (self.min, self.max),
            DirKind::Threshold => (
                self.size.saturating_sub(self.threshold),
                self.size + self.threshold,
            ),
        };
        (min * self.scale, max * self.scale)
    }

    /// How far the directory is from `size` pixels (0: a match).
    fn distance(&self, size: u32) -> u32 {
        let (min, max) = self.pixel_range();
        if size < min {
            min - size
        } else {
            size.saturating_sub(max)
        }
    }
}

/// A parsed `index.theme`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct ThemeIndex {
    dirs: Vec<ThemeDir>,
    inherits: Vec<String>,
}

/// Parses an `index.theme` file.
fn parse_index(content: &str) -> ThemeIndex {
    let mut sections: HashMap<&str, HashMap<&str, &str>> = HashMap::new();
    let mut current = "";
    for line in content.lines().map(str::trim) {
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            current = name;
        } else if let Some((key, value)) = line.split_once('=') {
            sections
                .entry(current)
                .or_default()
                .insert(key.trim(), value.trim());
        }
    }
    let list = |value: Option<&&str>| -> Vec<String> {
        value
            .map(|v| {
                v.split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    };
    let header = sections.get("Icon Theme");
    let mut names = list(header.and_then(|h| h.get("Directories")));
    names.extend(list(header.and_then(|h| h.get("ScaledDirectories"))));
    let dirs = names
        .into_iter()
        .filter_map(|path| {
            let section = sections.get(path.as_str())?;
            let number = |key: &str| section.get(key).and_then(|v| v.parse::<u32>().ok());
            let size = number("Size")?;
            let kind = match section.get("Type").copied() {
                Some("Fixed") => DirKind::Fixed,
                Some("Scalable") => DirKind::Scalable,
                _ => DirKind::Threshold,
            };
            Some(ThemeDir {
                size,
                scale: number("Scale").unwrap_or(1).max(1),
                min: number("MinSize").unwrap_or(size),
                max: number("MaxSize").unwrap_or(size),
                threshold: number("Threshold").unwrap_or(2),
                kind,
                path,
            })
        })
        .collect();
    ThemeIndex {
        dirs,
        inherits: list(header.and_then(|h| h.get("Inherits"))),
    }
}

/// The `gtk-icon-theme-name` of a GTK `settings.ini`.
#[must_use]
pub fn parse_gtk_icon_theme(content: &str) -> Option<String> {
    content.lines().find_map(|line| {
        let (key, value) = line.split_once('=')?;
        (key.trim() == "gtk-icon-theme-name")
            .then(|| value.trim().trim_matches('"').to_owned())
            .filter(|name| !name.is_empty())
    })
}

/// The size hinted by an icon path's directories (`22x22`, `scalable`).
fn size_hint(path: &Path) -> Option<u32> {
    path.components().rev().skip(1).find_map(|c| {
        let c = c.as_os_str().to_str()?;
        if c == "scalable" {
            return Some(u32::MAX);
        }
        let (w, h) = c.split_once('x')?;
        let w = w.parse::<u32>().ok()?;
        h.split('@').next()?.parse::<u32>().ok().map(|h| w.max(h))
    })
}

/// Looks up icons by name in the icon themes.
#[derive(Debug)]
pub struct IconThemes {
    /// Base directories that contain themes (`…/icons`).
    bases: Vec<PathBuf>,
    /// Directories with unthemed icons (`/usr/share/pixmaps`).
    pixmaps: Vec<PathBuf>,
    /// The user's theme, searched before the fallbacks.
    user_theme: Option<String>,
    /// Theme search order, computed on first use.
    chain: Option<Vec<String>>,
    /// Parsed `index.theme`s by base list and theme name.
    indexes: HashMap<(Vec<PathBuf>, String), Option<ThemeIndex>>,
}

impl IconThemes {
    pub fn new(bases: Vec<PathBuf>, pixmaps: Vec<PathBuf>, user_theme: Option<String>) -> Self {
        Self {
            bases,
            pixmaps,
            user_theme,
            chain: None,
            indexes: HashMap::new(),
        }
    }

    /// The standard directories and the user's GTK icon theme.
    pub fn from_env() -> Self {
        let var = |name: &str| {
            env::var_os(name)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        let home = var("HOME");
        let data_home =
            var("XDG_DATA_HOME").or_else(|| home.as_ref().map(|h| h.join(".local/share")));
        let config_home =
            var("XDG_CONFIG_HOME").or_else(|| home.as_ref().map(|h| h.join(".config")));
        let data_dirs: Vec<PathBuf> = env::var("XDG_DATA_DIRS")
            .ok()
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "/usr/local/share:/usr/share".to_owned())
            .split(':')
            .filter(|d| !d.is_empty())
            .map(PathBuf::from)
            .collect();
        let mut bases = Vec::new();
        bases.extend(data_home.iter().map(|d| d.join("icons")));
        bases.extend(home.iter().map(|h| h.join(".icons")));
        bases.extend(data_dirs.iter().map(|d| d.join("icons")));
        let mut pixmaps: Vec<PathBuf> = data_dirs.iter().map(|d| d.join("pixmaps")).collect();
        let usr_pixmaps = PathBuf::from("/usr/share/pixmaps");
        if !pixmaps.contains(&usr_pixmaps) {
            pixmaps.push(usr_pixmaps);
        }
        let user_theme = config_home.and_then(|config| {
            ["gtk-3.0", "gtk-4.0"].iter().find_map(|gtk| {
                let content = fs::read_to_string(config.join(gtk).join("settings.ini")).ok()?;
                parse_gtk_icon_theme(&content)
            })
        });
        Self::new(bases, pixmaps, user_theme)
    }

    /// The `index.theme` of `theme` in the first of `bases` that has one.
    fn index(&mut self, bases: &[PathBuf], theme: &str) -> Option<ThemeIndex> {
        self.indexes
            .entry((bases.to_vec(), theme.to_owned()))
            .or_insert_with(|| {
                bases.iter().find_map(|base| {
                    let content = fs::read_to_string(base.join(theme).join("index.theme")).ok()?;
                    Some(parse_index(&content))
                })
            })
            .clone()
    }

    /// Theme names in search order: the user's theme and its parents,
    /// then the fallbacks.
    fn chain(&mut self) -> Vec<String> {
        if let Some(chain) = &self.chain {
            return chain.clone();
        }
        let mut chain: Vec<String> = Vec::new();
        let mut pending: Vec<String> = self.user_theme.iter().cloned().collect();
        let bases = self.bases.clone();
        while let Some(theme) = pending.pop() {
            if chain.contains(&theme) || FALLBACK_THEMES.contains(&theme.as_str()) {
                continue;
            }
            if let Some(index) = self.index(&bases, &theme) {
                pending.extend(index.inherits.into_iter().rev());
            }
            chain.push(theme);
        }
        chain.extend(FALLBACK_THEMES.iter().map(|t| (*t).to_owned()));
        self.chain = Some(chain.clone());
        chain
    }

    /// Finds `name` in `theme` under `bases`: an exact size match, else
    /// the closest size.
    fn find_in_theme(
        &mut self,
        bases: &[PathBuf],
        theme: &str,
        name: &str,
        size: u32,
    ) -> Option<PathBuf> {
        let index = self.index(bases, theme)?;
        let mut best: Option<(u32, u32, PathBuf)> = None;
        for dir in &index.dirs {
            let distance = dir.distance(size);
            if best
                .as_ref()
                .is_some_and(|(d, s, _)| (*d, Reverse(*s)) <= (distance, Reverse(dir.size)))
            {
                continue;
            }
            let found = bases.iter().find_map(|base| {
                EXTENSIONS.iter().find_map(|ext| {
                    let path = base
                        .join(theme)
                        .join(&dir.path)
                        .join(format!("{name}.{ext}"));
                    path.is_file().then_some(path)
                })
            });
            if let Some(path) = found {
                if distance == 0 {
                    return Some(path);
                }
                best = Some((distance, dir.size, path));
            }
        }
        best.map(|(_, _, path)| path)
    }

    /// `name.{svg,png}` directly in one of `dirs`.
    fn find_flat(dirs: &[PathBuf], name: &str) -> Option<PathBuf> {
        dirs.iter().find_map(|dir| {
            EXTENSIONS.iter().find_map(|ext| {
                let path = dir.join(format!("{name}.{ext}"));
                path.is_file().then_some(path)
            })
        })
    }

    /// Searches `root` recursively for `name.{svg,png}`, preferring the
    /// candidate whose directory hints at the size closest to `size`.
    fn scan(root: &Path, name: &str, size: u32) -> Option<PathBuf> {
        let mut found = Vec::new();
        let mut pending = vec![(root.to_path_buf(), 0)];
        while let Some((dir, depth)) = pending.pop() {
            let Ok(entries) = fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.filter_map(Result::ok) {
                let path = entry.path();
                if path.is_dir() {
                    if depth < MAX_SCAN_DEPTH {
                        pending.push((path, depth + 1));
                    }
                } else if path.file_stem().and_then(|s| s.to_str()) == Some(name)
                    && path
                        .extension()
                        .and_then(|e| e.to_str())
                        .is_some_and(|e| EXTENSIONS.contains(&e))
                {
                    found.push(path);
                }
            }
        }
        found.sort();
        found.into_iter().min_by_key(|path| {
            let hint = size_hint(path).unwrap_or(0);
            let is_svg = path.extension().is_some_and(|e| e == "svg");
            match hint {
                u32::MAX => (0, 0),
                hint if hint >= size => (1, hint - size),
                hint => (2 + u32::from(!is_svg), size - hint),
            }
        })
    }

    /// The file for icon `name` at `size` pixels. `name` may be an
    /// absolute path; `theme_path` is an item's `IconThemePath`, searched
    /// first.
    pub fn find(&mut self, name: &str, theme_path: Option<&Path>, size: u32) -> Option<PathBuf> {
        let as_path = Path::new(name);
        if as_path.is_absolute() {
            return as_path.is_file().then(|| as_path.to_path_buf());
        }
        let chain = self.chain();
        if let Some(root) = theme_path.filter(|p| p.is_dir()) {
            let roots = [root.to_path_buf()];
            if let Some(path) = Self::find_flat(&roots, name) {
                return Some(path);
            }
            for theme in &chain {
                if let Some(path) = self.find_in_theme(&roots, theme, name, size) {
                    return Some(path);
                }
            }
            if let Some(path) = Self::scan(root, name, size) {
                return Some(path);
            }
        }
        let bases = self.bases.clone();
        for theme in &chain {
            if let Some(path) = self.find_in_theme(&bases, theme, name, size) {
                return Some(path);
            }
        }
        Self::find_flat(&self.pixmaps, name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const INDEX: &str = "\
[Icon Theme]
Name=Test
Inherits=Parent, hicolor
Directories=16x16/apps,48x48/apps,scalable/apps
ScaledDirectories=16x16@2/apps

[16x16/apps]
Size=16
Type=Fixed

[16x16@2/apps]
Size=16
Scale=2
Type=Fixed

[48x48/apps]
Size=48
Type=Threshold

[scalable/apps]
Size=48
MinSize=8
MaxSize=512
Type=Scalable
";

    #[test]
    fn parses_index_theme() {
        let index = parse_index(INDEX);
        assert_eq!(index.inherits, vec!["Parent", "hicolor"]);
        assert_eq!(index.dirs.len(), 4);
        let scaled = index
            .dirs
            .iter()
            .find(|d| d.path == "16x16@2/apps")
            .expect("scaled dir");
        assert_eq!(scaled.pixel_range(), (32, 32));
        assert_eq!(scaled.distance(32), 0);
        assert_eq!(index.dirs[1].pixel_range(), (46, 50));
        assert_eq!(index.dirs[2].distance(600), 88);
    }

    #[test]
    fn parses_gtk_settings() {
        let ini = "[Settings]\ngtk-theme-name=Foo\ngtk-icon-theme-name = \"Papirus-Dark\"\n";
        assert_eq!(parse_gtk_icon_theme(ini).as_deref(), Some("Papirus-Dark"));
        assert_eq!(parse_gtk_icon_theme("[Settings]\n"), None);
    }

    #[test]
    fn size_hints() {
        assert_eq!(size_hint(Path::new("/a/22x22/apps/x.png")), Some(22));
        assert_eq!(size_hint(Path::new("/a/16x16@2/apps/x.png")), Some(16));
        assert_eq!(
            size_hint(Path::new("/a/scalable/apps/x.svg")),
            Some(u32::MAX)
        );
        assert_eq!(size_hint(Path::new("/a/x.svg")), None);
    }

    fn touch(path: &Path) {
        fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        fs::write(path, b"").expect("write");
    }

    #[test]
    fn looks_up_icons_in_themes() {
        let root = std::env::temp_dir().join(format!("oxmonad-tray-theme-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let base = root.join("icons");
        fs::create_dir_all(base.join("Test")).expect("mkdir");
        fs::write(base.join("Test/index.theme"), INDEX).expect("write");
        fs::create_dir_all(base.join("Parent")).expect("mkdir");
        fs::write(
            base.join("Parent/index.theme"),
            "[Icon Theme]\nDirectories=22x22/apps\n\n[22x22/apps]\nSize=22\nType=Fixed\n",
        )
        .expect("write");
        fs::create_dir_all(base.join("hicolor")).expect("mkdir");
        fs::write(
            base.join("hicolor/index.theme"),
            "[Icon Theme]\nDirectories=32x32/apps\n\n[32x32/apps]\nSize=32\nType=Fixed\n",
        )
        .expect("write");
        touch(&base.join("Test/16x16/apps/both.png"));
        touch(&base.join("Test/48x48/apps/both.png"));
        touch(&base.join("Test/16x16@2/apps/both.png"));
        touch(&base.join("Parent/22x22/apps/parent.png"));
        touch(&base.join("hicolor/32x32/apps/color.png"));
        touch(&root.join("pixmaps/legacy.png"));
        touch(&root.join("app/hicolor/32x32/apps/own.svg"));
        touch(&root.join("app/flat.png"));
        touch(&root.join("app/deep/24x24/status/scan.png"));
        touch(&root.join("app/deep/64x64/status/scan.png"));

        let mut themes = IconThemes::new(
            vec![base.clone()],
            vec![root.join("pixmaps")],
            Some("Test".to_owned()),
        );
        assert_eq!(themes.chain(), vec!["Test", "Parent", "Adwaita", "hicolor"]);
        // The scale-2 directory matches 32 pixels exactly.
        assert_eq!(
            themes.find("both", None, 32),
            Some(base.join("Test/16x16@2/apps/both.png"))
        );
        // The threshold directory covers 46..=50.
        assert_eq!(
            themes.find("both", None, 47),
            Some(base.join("Test/48x48/apps/both.png"))
        );
        assert_eq!(
            themes.find("parent", None, 44),
            Some(base.join("Parent/22x22/apps/parent.png"))
        );
        assert_eq!(
            themes.find("color", None, 44),
            Some(base.join("hicolor/32x32/apps/color.png"))
        );
        assert_eq!(
            themes.find("legacy", None, 44),
            Some(root.join("pixmaps/legacy.png"))
        );
        let app = root.join("app");
        assert_eq!(
            themes.find("own", Some(&app), 44),
            Some(app.join("hicolor/32x32/apps/own.svg"))
        );
        assert_eq!(
            themes.find("flat", Some(&app), 44),
            Some(app.join("flat.png"))
        );
        assert_eq!(
            themes.find("scan", Some(&app), 44),
            Some(app.join("deep/64x64/status/scan.png"))
        );
        let absolute = root.join("pixmaps/legacy.png");
        assert_eq!(
            themes.find(absolute.to_str().expect("utf-8"), None, 44),
            Some(absolute.clone())
        );
        assert_eq!(themes.find("missing", Some(&app), 44), None);
        fs::remove_dir_all(&root).expect("cleanup");
    }
}
