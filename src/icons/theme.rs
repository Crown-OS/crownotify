//! XDG icon-theme lookup: turns an icon *name* into a file on disk.
//!
//! This follows the shape of the freedesktop icon theme spec closely enough
//! to find real icons, but deliberately not to the letter — there is no
//! `Directories` metadata parsing (`Size`/`MinSize`/`MaxSize`/`Threshold`),
//! only the size encoded in each directory's own name (`48x48`, `scalable`,
//! …). Real themes are consistent about this, and skipping the metadata
//! table means the whole scan is a couple of `read_dir` calls per theme
//! instead of an `index.theme` parse per directory.

use std::{collections::HashSet, fs, path::PathBuf};

/// Extensions accepted for an icon file, in the preference order the spec
/// asks for: a raster hit at the right size beats an SVG, which beats the
/// legacy XPM format nothing ships anymore but some themes still carry.
const EXTENSIONS: [&str; 3] = ["png", "svg", "xpm"];

/// How deep an `Inherits` chase is allowed to go before it is cut off.
///
/// Real theme chains are two or three deep (`Adwaita` -> `hicolor`). This is
/// generous headroom for that while still bounding the walk if two themes
/// name each other as parents.
const MAX_CHAIN_DEPTH: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Size {
    /// Vector icon: valid at any requested size.
    Scalable,
    /// Raster directory named e.g. `48x48`; the value is the pixel edge.
    Fixed(u32),
}

/// One size directory found under a theme, and every place to look inside it
/// for a given icon name (the directory itself, plus any context
/// subdirectories such as `apps`, `mimetypes`, `status`, …).
#[derive(Debug)]
struct SizeEntry {
    dirs: Vec<PathBuf>,
    size: Size,
}

impl SizeEntry {
    fn find(&self, name: &str) -> Option<PathBuf> {
        for dir in &self.dirs {
            for ext in EXTENSIONS {
                let candidate = dir.join(format!("{name}.{ext}"));
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
        None
    }
}

/// Every size directory belonging to one theme in the inheritance chain,
/// scanned once up front so `lookup` never walks a directory tree.
#[derive(Debug)]
struct ThemeLevel {
    sizes: Vec<SizeEntry>,
}

impl ThemeLevel {
    fn scan(theme_dirs: &[PathBuf]) -> Self {
        let mut sizes = Vec::new();
        for theme_dir in theme_dirs {
            let Ok(read) = fs::read_dir(theme_dir) else {
                continue;
            };
            for entry in read.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                    continue;
                };
                let Some(size) = parse_size_dir(name) else {
                    continue;
                };
                let mut dirs = vec![path.clone()];
                if let Ok(contexts) = fs::read_dir(&path) {
                    dirs.extend(contexts.flatten().map(|c| c.path()).filter(|p| p.is_dir()));
                }
                sizes.push(SizeEntry { dirs, size });
            }
        }
        Self { sizes }
    }

    fn best_match(&self, name: &str, size: u32) -> Option<PathBuf> {
        let candidates: Vec<(Size, PathBuf)> = self
            .sizes
            .iter()
            .filter_map(|entry| entry.find(name).map(|path| (entry.size, path)))
            .collect();
        pick_best(&candidates, size)
    }
}

/// Parses a size-directory name like `48x48` or `48x48@2` into its pixel
/// edge, or recognises `scalable`. Anything else (context directories that
/// leaked in, stray files) is not a size directory at all.
fn parse_size_dir(name: &str) -> Option<Size> {
    if name == "scalable" {
        return Some(Size::Scalable);
    }
    let name = name.split('@').next().unwrap_or(name);
    let (width, _height) = name.split_once('x')?;
    width.parse().ok().map(Size::Fixed)
}

/// Chooses among the icon files found in a single theme for one requested
/// size: an exact match, else the smallest that is still larger (shrinking
/// reads better than the blur of stretching), else the largest that is
/// smaller, else a scalable icon, which fits any size by construction.
fn pick_best(candidates: &[(Size, PathBuf)], target: u32) -> Option<PathBuf> {
    if let Some((_, path)) = candidates
        .iter()
        .find(|(size, _)| matches!(size, Size::Fixed(edge) if *edge == target))
    {
        return Some(path.clone());
    }

    let smallest_larger = candidates
        .iter()
        .filter_map(|(size, path)| match size {
            Size::Fixed(edge) if *edge > target => Some((*edge, path)),
            _ => None,
        })
        .min_by_key(|(edge, _)| *edge);
    if let Some((_, path)) = smallest_larger {
        return Some(path.clone());
    }

    let largest_smaller = candidates
        .iter()
        .filter_map(|(size, path)| match size {
            Size::Fixed(edge) if *edge < target => Some((*edge, path)),
            _ => None,
        })
        .max_by_key(|(edge, _)| *edge);
    if let Some((_, path)) = largest_smaller {
        return Some(path.clone());
    }

    candidates
        .iter()
        .find(|(size, _)| matches!(size, Size::Scalable))
        .map(|(_, path)| path.clone())
}

/// Reads the `Inherits` line out of a theme's `index.theme`, if one of the
/// base directories has that theme and the file parses.
///
/// This only looks for a line starting `Inherits=` rather than a proper INI
/// parse of the `[Icon Theme]` section — every real theme file puts the key
/// there once, and a stray `Inherits=` elsewhere would be a theme bug this
/// lookup doesn't need to defend against.
fn read_inherits(theme: &str, base_dirs: &[PathBuf]) -> Vec<String> {
    for base in base_dirs {
        let index = base.join(theme).join("index.theme");
        let Ok(contents) = fs::read_to_string(&index) else {
            continue;
        };
        if let Some(line) = contents
            .lines()
            .map(str::trim)
            .find_map(|l| l.strip_prefix("Inherits="))
        {
            return line
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect();
        }
    }
    Vec::new()
}

/// Walks `Inherits` depth-first from `start`, recording every theme visited
/// exactly once. `visited` doubles as the cycle guard: a theme that names an
/// ancestor (or itself) as a parent simply stops being explored again rather
/// than recursing forever.
fn walk_chain(
    theme: &str,
    base_dirs: &[PathBuf],
    visited: &mut HashSet<String>,
    chain: &mut Vec<String>,
    depth: usize,
) {
    if depth > MAX_CHAIN_DEPTH || visited.contains(theme) {
        return;
    }
    visited.insert(theme.to_string());
    chain.push(theme.to_string());
    for parent in read_inherits(theme, base_dirs) {
        walk_chain(&parent, base_dirs, visited, chain, depth + 1);
    }
}

fn icon_base_dirs() -> Vec<PathBuf> {
    crate::util::xdg::data_dirs("icons")
}

/// Reads the icon theme name GTK 3 apps were last configured with, if the
/// settings file exists and parses. This is the "cheap" half of theme
/// detection: one small file read, no GSettings/D-Bus round trip.
fn gtk_icon_theme_name() -> Option<String> {
    let config_home = crate::util::xdg::config_home()?;
    let contents = fs::read_to_string(config_home.join("gtk-3.0/settings.ini")).ok()?;
    contents
        .lines()
        .map(str::trim)
        .find_map(|l| l.strip_prefix("gtk-icon-theme-name="))
        .map(str::to_string)
        .filter(|s| !s.is_empty())
}

/// Resolves an icon *name* (as sent in a `Notify` hint, never a path) to a
/// file on disk, honouring the user's icon theme and its `Inherits` chain
/// down to `hicolor`, and finally the flat `/usr/share/pixmaps` directory
/// legacy apps still drop icons into.
///
/// Every directory this needs is scanned once, at construction, so repeated
/// [`lookup`](Self::lookup) calls only ever do a bounded number of
/// `Path::is_file` checks — no `read_dir` — which is what keeps this safe to
/// call from a paint path. A small result cache on top of that means a name
/// that keeps missing (or keeps hitting) does not even pay for those checks
/// twice.
#[derive(Debug)]
pub struct IconTheme {
    chain: Vec<ThemeLevel>,
    pixmaps: PathBuf,
    cache: std::cell::RefCell<std::collections::HashMap<(String, u32), Option<PathBuf>>>,
}

impl IconTheme {
    /// Detects the theme the rest of the desktop is using: `$XDG_ICON_THEME`
    /// if set, else the GTK 3 setting if cheaply readable, else `hicolor`.
    pub fn detect() -> Self {
        let name = std::env::var("XDG_ICON_THEME")
            .ok()
            .filter(|s| !s.is_empty())
            .or_else(gtk_icon_theme_name)
            .unwrap_or_else(|| "hicolor".to_string());
        Self::with_name(name)
    }

    /// Builds a theme lookup for an explicitly named theme, falling back to
    /// `hicolor` through the same `Inherits` walk as any other theme.
    pub fn with_name(name: impl Into<String>) -> Self {
        Self::build(
            name.into(),
            icon_base_dirs(),
            PathBuf::from("/usr/share/pixmaps"),
        )
    }

    /// Resolves `name` to the best-matching icon file for `size`, or `None`
    /// if no theme in the chain and no pixmap directory has it.
    pub fn lookup(&self, name: &str, size: u32) -> Option<PathBuf> {
        let key = (name.to_string(), size);
        if let Some(cached) = self.cache.borrow().get(&key) {
            return cached.clone();
        }
        let resolved = self.lookup_uncached(name, size);
        self.cache.borrow_mut().insert(key, resolved.clone());
        resolved
    }

    fn lookup_uncached(&self, name: &str, size: u32) -> Option<PathBuf> {
        for level in &self.chain {
            if let Some(path) = level.best_match(name, size) {
                return Some(path);
            }
        }
        for ext in EXTENSIONS {
            let candidate = self.pixmaps.join(format!("{name}.{ext}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        None
    }

    /// Shared constructor so tests can point the scan at a synthetic theme
    /// tree instead of the real XDG directories.
    fn build(name: String, base_dirs: Vec<PathBuf>, pixmaps: PathBuf) -> Self {
        let mut chain_names = Vec::new();
        let mut visited = HashSet::new();
        walk_chain(&name, &base_dirs, &mut visited, &mut chain_names, 0);
        if !visited.contains("hicolor") {
            chain_names.push("hicolor".to_string());
        }

        let chain = chain_names
            .into_iter()
            .map(|theme| {
                let dirs: Vec<PathBuf> = base_dirs.iter().map(|base| base.join(&theme)).collect();
                ThemeLevel::scan(&dirs)
            })
            .collect();

        Self {
            chain,
            pixmaps,
            cache: std::cell::RefCell::new(std::collections::HashMap::new()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File};
    use std::path::Path;

    /// Lays out `{root}/icons/{theme}/{size}/apps/{name}.{ext}` the way a
    /// real theme would, creating an empty file (contents are never read by
    /// the lookup, only existence and extension matter).
    fn place_icon(root: &Path, theme: &str, size_dir: &str, name: &str, ext: &str) {
        let dir = root.join("icons").join(theme).join(size_dir).join("apps");
        fs::create_dir_all(&dir).expect("create icon dir");
        File::create(dir.join(format!("{name}.{ext}"))).expect("create icon file");
    }

    fn write_index_theme(root: &Path, theme: &str, inherits: &str) {
        let dir = root.join("icons").join(theme);
        fs::create_dir_all(&dir).expect("create theme dir");
        fs::write(
            dir.join("index.theme"),
            format!("[Icon Theme]\nName={theme}\nInherits={inherits}\n"),
        )
        .expect("write index.theme");
    }

    fn theme_at(root: &Path, name: &str) -> IconTheme {
        IconTheme::build(
            name.to_string(),
            vec![root.join("icons")],
            root.join("pixmaps"),
        )
    }

    #[test]
    fn an_exact_size_match_wins_over_every_other_candidate() {
        let dir = tempfile::tempdir().expect("tempdir");
        place_icon(dir.path(), "hicolor", "32x32", "mail", "png");
        place_icon(dir.path(), "hicolor", "48x48", "mail", "png");
        place_icon(dir.path(), "hicolor", "64x64", "mail", "png");

        let theme = theme_at(dir.path(), "hicolor");
        let found = theme.lookup("mail", 48).expect("mail should resolve");
        assert!(
            found.ends_with("48x48/apps/mail.png"),
            "expected the exact 48x48 match, got {found:?}"
        );
    }

    #[test]
    fn a_larger_icon_is_preferred_over_a_smaller_one_absent_an_exact_match() {
        let dir = tempfile::tempdir().expect("tempdir");
        place_icon(dir.path(), "hicolor", "16x16", "mail", "png");
        place_icon(dir.path(), "hicolor", "64x64", "mail", "png");

        let theme = theme_at(dir.path(), "hicolor");
        let found = theme.lookup("mail", 32).expect("mail should resolve");
        assert!(
            found.ends_with("64x64/apps/mail.png"),
            "expected the next-larger 64x64 icon over the smaller 16x16, got {found:?}"
        );
    }

    #[test]
    fn the_largest_smaller_icon_is_used_when_nothing_bigger_exists() {
        let dir = tempfile::tempdir().expect("tempdir");
        place_icon(dir.path(), "hicolor", "16x16", "mail", "png");
        place_icon(dir.path(), "hicolor", "32x32", "mail", "png");

        let theme = theme_at(dir.path(), "hicolor");
        let found = theme.lookup("mail", 64).expect("mail should resolve");
        assert!(
            found.ends_with("32x32/apps/mail.png"),
            "expected the largest available (32x32) icon, got {found:?}"
        );
    }

    #[test]
    fn a_scalable_directory_matches_any_requested_size() {
        let dir = tempfile::tempdir().expect("tempdir");
        place_icon(dir.path(), "hicolor", "scalable", "mail", "svg");

        let theme = theme_at(dir.path(), "hicolor");
        for size in [16, 48, 512] {
            let found = theme.lookup("mail", size);
            assert!(found.is_some(), "scalable icon should match size {size}");
        }
    }

    #[test]
    fn a_theme_missing_the_icon_falls_back_to_hicolor() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_index_theme(dir.path(), "custom", "hicolor");
        place_icon(dir.path(), "hicolor", "48x48", "mail", "png");

        let theme = theme_at(dir.path(), "custom");
        let found = theme
            .lookup("mail", 48)
            .expect("hicolor fallback should resolve mail");
        assert!(
            found.to_string_lossy().contains("hicolor"),
            "expected the hicolor fallback to be used, got {found:?}"
        );
    }

    #[test]
    fn an_inheritance_cycle_terminates_instead_of_looping_forever() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_index_theme(dir.path(), "a", "b");
        write_index_theme(dir.path(), "b", "a");
        place_icon(dir.path(), "b", "48x48", "mail", "png");

        let theme = theme_at(dir.path(), "a");
        let found = theme.lookup("mail", 48);
        assert!(
            found.is_some(),
            "the cyclic chain should still resolve icons it can reach"
        );
    }

    #[test]
    fn an_unknown_icon_name_resolves_to_none() {
        let dir = tempfile::tempdir().expect("tempdir");
        place_icon(dir.path(), "hicolor", "48x48", "mail", "png");

        let theme = theme_at(dir.path(), "hicolor");
        assert_eq!(theme.lookup("does-not-exist", 48), None);
    }

    #[test]
    fn a_lookup_result_is_cached_so_a_second_call_does_not_need_to_hit_the_theme_again() {
        let dir = tempfile::tempdir().expect("tempdir");
        place_icon(dir.path(), "hicolor", "48x48", "mail", "png");

        let theme = theme_at(dir.path(), "hicolor");
        let first = theme.lookup("mail", 48);
        // Remove the underlying file: if the second call re-scanned the
        // filesystem instead of using the cache, it would now come back
        // empty instead of matching the first result.
        fs::remove_file(first.clone().expect("first lookup should hit")).expect("remove icon");
        let second = theme.lookup("mail", 48);
        assert_eq!(
            first, second,
            "the cached path should be returned unchanged"
        );
    }

    #[test]
    fn the_pixmaps_fallback_is_used_when_no_theme_has_the_icon() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::create_dir_all(dir.path().join("pixmaps")).expect("create pixmaps dir");
        File::create(dir.path().join("pixmaps/legacy-app.png")).expect("create pixmap");

        let theme = theme_at(dir.path(), "hicolor");
        let found = theme
            .lookup("legacy-app", 48)
            .expect("pixmap fallback should resolve");
        assert!(found.ends_with("pixmaps/legacy-app.png"), "got {found:?}");
    }
}
