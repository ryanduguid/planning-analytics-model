//! `pathlib.Path` as the original engine used it.
//!
//! Messages and findings print paths the way `pathlib` holds them: separators
//! normalised, `.` parts dropped, joined with the platform separator. Sorting,
//! equality and `is_relative_to` follow `pathlib`'s case rules (case
//! insensitive on Windows), and `resolve` ports `os.path.realpath` from
//! CPython 3.15 for each platform, so a path that names no file resolves as
//! far as it can and keeps the rest.

use std::cmp::Ordering;
use std::path::PathBuf;

use crate::error::Uncaught;
use crate::py;

#[cfg(windows)]
const SEP: char = '\\';
#[cfg(not(windows))]
const SEP: char = '/';

/// A parsed path: drive, root and the remaining parts.
#[derive(Clone, Debug)]
pub struct PyPath {
    /// The string the path was made from, which `/` joins to, as
    /// `PurePath._raw_paths` is; derived paths hold their `str()`.
    raw: String,
    drv: String,
    root: String,
    tail: Vec<String>,
}

/// `os.path.splitroot` for the platform.
#[cfg(windows)]
pub fn splitroot(p: &str) -> (String, String, String) {
    let normp = p.replace('/', "\\");
    let chars: Vec<char> = p.chars().collect();
    let norm: Vec<char> = normp.chars().collect();
    let slice = |from: usize, to: usize| -> String { chars[from..to].iter().collect() };
    if norm.first() == Some(&'\\') {
        if norm.get(1) == Some(&'\\') {
            let head: String = norm.iter().take(8).collect();
            let start = if head.to_uppercase() == "\\\\?\\UNC\\" {
                8
            } else {
                2
            };
            let find = |from: usize| (from..norm.len()).find(|&i| norm[i] == '\\');
            let Some(index) = find(start.min(norm.len())) else {
                return (p.to_string(), String::new(), String::new());
            };
            let Some(index2) = find(index + 1) else {
                return (p.to_string(), String::new(), String::new());
            };
            return (
                slice(0, index2),
                slice(index2, index2 + 1),
                slice(index2 + 1, chars.len()),
            );
        }
        return (String::new(), slice(0, 1), slice(1, chars.len()));
    }
    if norm.get(1) == Some(&':') {
        if norm.get(2) == Some(&'\\') {
            return (slice(0, 2), slice(2, 3), slice(3, chars.len()));
        }
        return (slice(0, 2), String::new(), slice(2, chars.len()));
    }
    (String::new(), String::new(), p.to_string())
}

/// `os.path.splitroot` for the platform.
#[cfg(not(windows))]
pub fn splitroot(p: &str) -> (String, String, String) {
    if !p.starts_with('/') {
        (String::new(), String::new(), p.to_string())
    } else if !p[1..].starts_with('/') || p[2..].starts_with('/') {
        (String::new(), "/".into(), p[1..].to_string())
    } else {
        (String::new(), "//".into(), p[2..].to_string())
    }
}

/// `os.path.join(a, b)` for the platform.
#[cfg(windows)]
pub fn join(path: &str, p: &str) -> String {
    let seps = |c: char| c == '\\' || c == '/';
    let (mut drive, mut root, mut tail) = splitroot(path);
    let (p_drive, p_root, p_path) = splitroot(p);
    if !p_root.is_empty() {
        if !p_drive.is_empty() || drive.is_empty() {
            drive = p_drive;
        }
        root = p_root;
        tail = p_path;
    } else if !p_drive.is_empty() && p_drive != drive {
        if p_drive.to_lowercase() != drive.to_lowercase() {
            drive = p_drive;
            root = p_root;
            tail = p_path;
        } else {
            drive = p_drive;
            if tail.chars().last().is_some_and(|c| !seps(c)) {
                tail.push('\\');
            }
            tail.push_str(&p_path);
        }
    } else {
        if tail.chars().last().is_some_and(|c| !seps(c)) {
            tail.push('\\');
        }
        tail.push_str(&p_path);
    }
    if !tail.is_empty()
        && root.is_empty()
        && drive
            .chars()
            .last()
            .is_some_and(|c| !matches!(c, ':' | '\\' | '/'))
    {
        return format!("{drive}\\{tail}");
    }
    format!("{drive}{root}{tail}")
}

/// `os.path.join(a, b)` for the platform.
#[cfg(not(windows))]
pub fn join(path: &str, p: &str) -> String {
    if p.starts_with('/') || path.is_empty() {
        p.to_string()
    } else if path.ends_with('/') {
        format!("{path}{p}")
    } else {
        format!("{path}/{p}")
    }
}

impl PyPath {
    /// `Path(text)`.
    pub fn new(text: &str) -> PyPath {
        if text.is_empty() {
            return PyPath {
                raw: String::new(),
                drv: String::new(),
                root: String::new(),
                tail: Vec::new(),
            };
        }
        let path = if cfg!(windows) {
            text.replace('/', "\\")
        } else {
            text.to_string()
        };
        let (drv, mut root, rel) = splitroot(&path);
        if cfg!(windows) && root.is_empty() && drv.starts_with('\\') && !drv.ends_with('\\') {
            let parts: Vec<&str> = drv.split('\\').collect();
            if (parts.len() == 4 && !"?.".contains(parts[2])) || parts.len() == 6 {
                root = "\\".into();
            }
        }
        let tail = rel
            .split(SEP)
            .filter(|x| !x.is_empty() && *x != ".")
            .map(str::to_string)
            .collect();
        PyPath {
            raw: text.to_string(),
            drv,
            root,
            tail,
        }
    }

    /// `self / part`.
    pub fn join(&self, part: &str) -> PyPath {
        PyPath::new(&join(&self.raw, part))
    }

    /// `str(path)`.
    pub fn display(&self) -> String {
        let sep = SEP.to_string();
        if !self.drv.is_empty() || !self.root.is_empty() {
            return format!("{}{}{}", self.drv, self.root, self.tail.join(&sep));
        }
        if self.tail.is_empty() {
            return ".".into();
        }
        if cfg!(windows) && !splitroot(&self.tail[0]).0.is_empty() {
            return format!(".{sep}{}", self.tail.join(&sep));
        }
        self.tail.join(&sep)
    }

    /// The filesystem path to operate on.
    pub fn fs(&self) -> PathBuf {
        PathBuf::from(self.display())
    }

    /// `path.name`.
    pub fn name(&self) -> &str {
        self.tail.last().map_or("", String::as_str)
    }

    /// `path.stem`.
    pub fn stem(&self) -> &str {
        let name = self.name();
        if let Some(i) = name.rfind('.') {
            let stem = &name[..i];
            if !stem.trim_start_matches('.').is_empty() {
                return stem;
            }
        }
        name
    }

    /// `path.suffix`.
    pub fn suffix(&self) -> &str {
        let name = self.name().trim_start_matches('.');
        match name.rfind('.') {
            Some(i) => &name[i..],
            None => "",
        }
    }

    /// `path.parent`.
    pub fn parent(&self) -> PyPath {
        let mut out = self.clone();
        if out.tail.pop().is_some() {
            out.raw = out.display();
        }
        out
    }

    /// `path.as_posix()`.
    pub fn as_posix(&self) -> String {
        self.display().replace(SEP, "/")
    }

    fn normcase(&self) -> String {
        if cfg!(windows) {
            py::lower(&self.display())
        } else {
            self.display()
        }
    }

    /// `path.is_relative_to(other)`.
    pub fn is_relative_to(&self, other: &PyPath) -> bool {
        self.relative_tail(other).is_some()
    }

    /// The parts of `self` below `other`, where `other == self` or is one of its parents.
    fn relative_tail(&self, other: &PyPath) -> Option<&[String]> {
        let target = other.normcase();
        let mut ancestor = self.clone();
        loop {
            if ancestor.normcase() == target {
                return Some(&self.tail[ancestor.tail.len()..]);
            }
            if ancestor.tail.is_empty() {
                return None;
            }
            ancestor.tail.pop();
        }
    }

    /// `path.relative_to(other).as_posix()`, or None where Python raised ValueError.
    pub fn relative_to_posix(&self, other: &PyPath) -> Option<String> {
        let tail = self.relative_tail(other)?;
        Some(if tail.is_empty() {
            ".".into()
        } else {
            tail.join("/")
        })
    }

    /// `path.is_file()`.
    pub fn is_file(&self) -> bool {
        std::fs::metadata(self.fs()).is_ok_and(|m| m.is_file())
    }

    /// `path.is_dir()`.
    pub fn is_dir(&self) -> bool {
        std::fs::metadata(self.fs()).is_ok_and(|m| m.is_dir())
    }

    /// `path.resolve()`.
    pub fn resolve(&self) -> Result<PyPath, Uncaught> {
        Ok(PyPath::new(&realpath(&self.display())?))
    }

    fn parts_normcase(&self) -> Vec<String> {
        self.normcase().split(SEP).map(str::to_string).collect()
    }
}

impl PartialEq for PyPath {
    fn eq(&self, other: &PyPath) -> bool {
        self.normcase() == other.normcase()
    }
}

impl Eq for PyPath {}

impl PartialOrd for PyPath {
    fn partial_cmp(&self, other: &PyPath) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PyPath {
    fn cmp(&self, other: &PyPath) -> Ordering {
        self.parts_normcase().cmp(&other.parts_normcase())
    }
}

impl std::fmt::Display for PyPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.display())
    }
}

/// The key `sorted()` and set membership use for a path, so callers can
/// sort or compare many paths without reparsing.
pub fn sort_key(path: &PyPath) -> Vec<String> {
    path.parts_normcase()
}

/// `str(path)` folded the way a `pathlib` set or dict compares it.
pub fn eq_key(path: &PyPath) -> String {
    path.normcase()
}

/// A name from the file system as text. A name that is not Unicode text is
/// refused rather than read with replacement characters, which could name a
/// different file.
pub fn fs_text(name: &std::ffi::OsStr) -> Result<String, Uncaught> {
    match name.to_str() {
        Some(text) => Ok(text.to_string()),
        None => Err(Uncaught::new(
            "UnicodeError",
            format!(
                "a file name is not UTF-8 text: {}",
                py::repr_str(&name.to_string_lossy())
            ),
        )),
    }
}

fn cwd() -> Result<String, Uncaught> {
    match std::env::current_dir() {
        Ok(path) => fs_text(path.as_os_str()),
        Err(_) => Ok(String::new()),
    }
}

#[cfg(not(windows))]
fn normpath(path: &str) -> String {
    if path.is_empty() {
        return ".".into();
    }
    let (_, initial, rest) = splitroot(path);
    let mut comps: Vec<&str> = Vec::new();
    for comp in rest.split('/') {
        if comp.is_empty() || comp == "." {
            continue;
        }
        if comp != ".." || (initial.is_empty() && comps.is_empty()) || comps.last() == Some(&"..") {
            comps.push(comp);
        } else if !comps.is_empty() {
            comps.pop();
        }
    }
    let out = format!("{initial}{}", comps.join("/"));
    if out.is_empty() { ".".into() } else { out }
}

/// `posixpath.realpath(path)`, non-strict.
#[cfg(not(windows))]
pub fn realpath(filename: &str) -> Result<String, Uncaught> {
    use std::collections::HashMap;
    let _ = normpath;
    enum Part {
        Name(String),
        Resolved,
    }
    let mut rest: Vec<Part> = filename
        .trim_end_matches('/')
        .split('/')
        .rev()
        .map(|s| Part::Name(s.to_string()))
        .collect();
    let mut part_count = rest.len();
    let mut path = if filename.starts_with('/') {
        "/".to_string()
    } else {
        cwd()?
    };
    let mut seen: HashMap<String, Option<String>> = HashMap::new();
    while part_count > 0 {
        let name = match rest.pop().expect("a pending part") {
            Part::Resolved => {
                if let Some(Part::Name(link)) = rest.pop() {
                    seen.insert(link, Some(path.clone()));
                }
                continue;
            }
            Part::Name(name) => name,
        };
        part_count -= 1;
        if name.is_empty() || name == "." {
            continue;
        }
        if name == ".." {
            let cut = path.rfind('/').expect("an absolute path");
            path.truncate(cut);
            if path.is_empty() {
                path = "/".into();
            }
            continue;
        }
        let newpath = if path == "/" {
            format!("/{name}")
        } else {
            format!("{path}/{name}")
        };
        if newpath.contains('\0') {
            // os.lstat raises ValueError, which realpath does not catch.
            return Err(Uncaught::new("ValueError", "embedded null byte"));
        }
        let target = match std::fs::symlink_metadata(&newpath) {
            Err(_) => {
                path = newpath;
                continue;
            }
            Ok(meta) if !meta.file_type().is_symlink() => {
                path = newpath;
                continue;
            }
            Ok(_) => {
                if let Some(cached) = seen.get(&newpath) {
                    match cached {
                        Some(resolved) => path = resolved.clone(),
                        None => path = newpath,
                    }
                    continue;
                }
                match std::fs::read_link(&newpath) {
                    Ok(target) => fs_text(target.as_os_str())?,
                    Err(_) => {
                        path = newpath;
                        continue;
                    }
                }
            }
        };
        if target.starts_with('/') {
            path = "/".into();
        }
        seen.insert(newpath.clone(), None);
        rest.push(Part::Name(newpath));
        rest.push(Part::Resolved);
        let parts: Vec<&str> = target.split('/').collect();
        part_count += parts.len();
        for part in parts.into_iter().rev() {
            rest.push(Part::Name(part.to_string()));
        }
    }
    Ok(path)
}

#[cfg(windows)]
fn normpath(path: &str) -> String {
    let path = path.replace('/', "\\");
    let (drive, root, rest) = splitroot(&path);
    let prefix = format!("{drive}{root}");
    let mut comps: Vec<&str> = rest.split('\\').collect();
    let mut i = 0;
    while i < comps.len() {
        if comps[i].is_empty() || comps[i] == "." {
            comps.remove(i);
        } else if comps[i] == ".." {
            if i > 0 && comps[i - 1] != ".." {
                comps.drain(i - 1..=i);
                i -= 1;
            } else if i == 0 && !root.is_empty() {
                comps.remove(i);
            } else {
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    if prefix.is_empty() && comps.is_empty() {
        comps.push(".");
    }
    format!("{prefix}{}", comps.join("\\"))
}

#[cfg(windows)]
fn isabs(s: &str) -> bool {
    let head: String = s.chars().take(3).collect::<String>().replace('/', "\\");
    head.get(1..).is_some_and(|h| h.starts_with(":\\")) || head.starts_with("\\\\")
}

#[cfg(windows)]
fn split(p: &str) -> (String, String) {
    let (d, r, rest) = splitroot(p);
    let cut = rest
        .char_indices()
        .rev()
        .find(|(_, c)| *c == '\\' || *c == '/')
        .map_or(0, |(i, c)| i + c.len_utf8());
    let (head, tail) = rest.split_at(cut);
    (
        format!("{d}{r}{}", head.trim_end_matches(['\\', '/'])),
        tail.to_string(),
    )
}

/// `_getfinalpathname(path)`: the final path, or the Windows error code.
#[cfg(windows)]
fn final_path(path: &str) -> Result<Result<String, i32>, Uncaught> {
    match std::fs::canonicalize(path) {
        Ok(p) => Ok(Ok(fs_text(p.as_os_str())?)),
        Err(e) => Ok(Err(e.raw_os_error().unwrap_or(0))),
    }
}

#[cfg(windows)]
fn readlink_deep(mut path: String) -> Result<String, Uncaught> {
    let mut seen = std::collections::HashSet::new();
    while seen.insert(path.to_lowercase()) {
        let old = path.clone();
        match std::fs::read_link(&path) {
            Ok(target) => {
                let target = fs_text(target.as_os_str())?;
                if !isabs(&target) {
                    let is_link =
                        std::fs::symlink_metadata(&old).is_ok_and(|m| m.file_type().is_symlink());
                    if !is_link {
                        path = old;
                        break;
                    }
                    path = normpath(&join(&split(&old).0, &target));
                } else {
                    path = target;
                }
            }
            Err(_) => break,
        }
    }
    Ok(path)
}

#[cfg(windows)]
fn final_path_nonstrict(mut path: String) -> Result<String, Uncaught> {
    let mut tail = String::new();
    while !path.is_empty() {
        match final_path(&path)? {
            Ok(found) => {
                return Ok(if tail.is_empty() {
                    found
                } else {
                    join(&found, &tail)
                });
            }
            Err(_) => {
                let new_path = readlink_deep(path.clone())?;
                if new_path != path {
                    return Ok(if tail.is_empty() {
                        new_path
                    } else {
                        join(&new_path, &tail)
                    });
                }
                let (head, name) = split(&path);
                path = head;
                if !path.is_empty() && name.is_empty() {
                    return Ok(format!("{path}{tail}"));
                }
                tail = if tail.is_empty() {
                    name
                } else {
                    join(&name, &tail)
                };
            }
        }
    }
    Ok(tail)
}

/// `ntpath.realpath(path)`, non-strict.
#[cfg(windows)]
pub fn realpath(path: &str) -> Result<String, Uncaught> {
    let mut path = normpath(path);
    if py::lower(&path) == "nul" {
        return Ok("\\\\.\\NUL".into());
    }
    let prefix = "\\\\?\\";
    let had_prefix = path.starts_with(prefix);
    if !had_prefix && !isabs(&path) {
        path = join(&cwd()?, &path);
    }
    if path.contains('\0') {
        // _getfinalpathname raises ValueError; realpath keeps the absolute path.
        return Ok(normpath(&path));
    }
    let initial = match final_path(&path)? {
        Ok(found) => {
            path = found;
            0
        }
        Err(code) => {
            path = final_path_nonstrict(path)?;
            code
        }
    };
    if !had_prefix && path.starts_with(prefix) {
        let spath = match path.strip_prefix("\\\\?\\UNC\\") {
            Some(rest) => format!("\\\\{rest}"),
            None => path[prefix.len()..].to_string(),
        };
        match final_path(&spath)? {
            Ok(found) if found == path => path = spath,
            Err(code) if code == initial => path = spath,
            _ => {}
        }
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_follow_pathlib() {
        let p = PyPath::new("data/a.CSV");
        assert_eq!(p.suffix(), ".CSV");
        assert_eq!(p.stem(), "a");
        assert_eq!(PyPath::new(".csv").suffix(), "");
        assert_eq!(PyPath::new(".csv").stem(), ".csv");
        assert_eq!(PyPath::new("a.").suffix(), ".");
        assert_eq!(PyPath::new("..a.csv").stem(), "..a");
        assert_eq!(PyPath::new("").display(), ".");
        assert_eq!(PyPath::new("./a//b/").as_posix(), "a/b");
    }

    #[test]
    fn sorting_compares_parts() {
        let mut paths = [PyPath::new("a-b/x"), PyPath::new("a/x")];
        paths.sort();
        assert_eq!(paths[0].as_posix(), "a/x");
    }

    #[cfg(windows)]
    #[test]
    fn windows_paths_follow_ntpath() {
        assert_eq!(PyPath::new("model/").display(), "model");
        assert_eq!(PyPath::new("C:/x/../y").display(), "C:\\x\\..\\y");
        assert_eq!(join("C:\\m", "D:x"), "D:x");
        assert_eq!(join("C:\\m", "c:x"), "c:\\m\\x");
        assert_eq!(join("C:\\m", "\\x"), "C:\\x");
        assert_eq!(
            PyPath::new("//server/share/x").display(),
            "\\\\server\\share\\x"
        );
        assert!(PyPath::new("C:\\A\\b").is_relative_to(&PyPath::new("c:\\a")));
        assert_eq!(normpath("C:\\a\\..\\..\\b"), "C:\\b");
    }

    #[cfg(not(windows))]
    #[test]
    fn posix_paths_follow_posixpath() {
        assert_eq!(PyPath::new("//x/y").display(), "//x/y");
        assert_eq!(PyPath::new("///x").display(), "/x");
        assert_eq!(join("/m", "/x"), "/x");
        assert!(!PyPath::new("/A/b").is_relative_to(&PyPath::new("/a")));
        assert_eq!(realpath("/no/such/../path").unwrap(), "/no/path");
    }
}
