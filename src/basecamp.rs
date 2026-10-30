//! The base camp directory and the only paths the file tools may touch.
use std::{
    fmt, io,
    path::{Component, Path, PathBuf},
};

/// Directories created under every base camp.
pub const DIRS: [&str; 4] = ["scenes", "assets", "scripts", "renders"];

#[derive(Debug, PartialEq, Eq)]
pub enum PathError {
    Empty,
    Absolute,
    Parent,
    Escapes,
    NotFound,
}

impl fmt::Display for PathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Empty => "path is empty",
            Self::Absolute => "absolute paths are refused; use a path inside the base camp",
            Self::Parent => "`..` is refused; use a path inside the base camp",
            Self::Escapes => "path leaves the base camp through a symlink",
            Self::NotFound => "no such file in the base camp",
        })
    }
}

impl std::error::Error for PathError {}

/// Checks a client path without touching the file system and returns it
/// normalized: `/`-separated, no `.` segments, never absolute or upward.
pub fn lexical(rel: &str) -> Result<PathBuf, PathError> {
    if rel.contains('\0') {
        return Err(PathError::NotFound);
    }
    let mut out = PathBuf::new();
    for component in Path::new(rel).components() {
        match component {
            Component::Normal(part) => out.push(part),
            Component::CurDir => {}
            Component::ParentDir => return Err(PathError::Parent),
            Component::RootDir | Component::Prefix(_) => return Err(PathError::Absolute),
        }
    }
    if out.as_os_str().is_empty() {
        return Err(PathError::Empty);
    }
    Ok(out)
}

#[derive(Clone, Debug)]
pub struct BaseCamp {
    root: PathBuf,
}

impl BaseCamp {
    /// Creates the standard directories and pins the canonical root.
    pub fn open(path: &Path) -> io::Result<Self> {
        for dir in DIRS {
            std::fs::create_dir_all(path.join(dir))?;
        }
        Ok(Self {
            root: path.canonicalize()?,
        })
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Resolves a file or directory that must already exist inside the camp.
    pub fn existing(&self, rel: &str) -> Result<PathBuf, PathError> {
        let path = self
            .root
            .join(lexical(rel)?)
            .canonicalize()
            .map_err(|_| PathError::NotFound)?;
        self.inside(path)
    }

    /// Resolves a path to write. Missing parent directories are created once
    /// the nearest existing ancestor is known to be inside the camp.
    pub fn writable(&self, rel: &str) -> Result<PathBuf, PathError> {
        let target = self.root.join(lexical(rel)?);
        // Every existing ancestor, and the target itself if present, must
        // resolve inside the camp; the missing rest has no symlinks yet.
        let mut existing = target.as_path();
        while existing.symlink_metadata().is_err() {
            existing = existing.parent().ok_or(PathError::Escapes)?;
        }
        self.inside(existing.canonicalize().map_err(|_| PathError::Escapes)?)?;
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|_| PathError::NotFound)?;
        }
        Ok(target)
    }

    fn inside(&self, canonical: PathBuf) -> Result<PathBuf, PathError> {
        if canonical.starts_with(&self.root) {
            Ok(canonical)
        } else {
            Err(PathError::Escapes)
        }
    }

    /// The `/`-separated path of `abs` relative to the camp root.
    #[must_use]
    pub fn relative(&self, abs: &Path) -> Option<String> {
        let rel = abs.strip_prefix(&self.root).ok()?;
        let parts: Vec<_> = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect();
        (!parts.is_empty()).then(|| parts.join("/"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexical_normalizes_plain_relative_paths() {
        for (input, expected) in [
            ("renders/a.png", "renders/a.png"),
            ("./scenes//cube.blend", "scenes/cube.blend"),
            ("assets/モカ/宮舞モカ.pmx", "assets/モカ/宮舞モカ.pmx"),
            ("scripts/", "scripts"),
        ] {
            assert_eq!(lexical(input), Ok(PathBuf::from(expected)), "{input}");
        }
    }

    #[test]
    fn lexical_refuses_paths_that_leave_the_camp() {
        for (input, error) in [
            ("", PathError::Empty),
            (".", PathError::Empty),
            ("/etc/passwd", PathError::Absolute),
            ("../outside.blend", PathError::Parent),
            ("renders/../../x", PathError::Parent),
            ("renders/..", PathError::Parent),
        ] {
            assert_eq!(lexical(input), Err(error), "{input}");
        }
    }

    #[test]
    fn lexical_refuses_nul_bytes() {
        assert!(lexical("renders/a\0.png").is_err(), "NUL accepted");
    }

    fn camp() -> (tempfile::TempDir, BaseCamp) {
        let dir = tempfile::tempdir().unwrap();
        let camp = BaseCamp::open(&dir.path().join("camp")).unwrap();
        (dir, camp)
    }

    #[test]
    fn open_creates_the_standard_directories() {
        let (_dir, camp) = camp();
        for name in DIRS {
            assert!(camp.root().join(name).is_dir(), "{name} missing");
        }
    }

    #[test]
    fn existing_resolves_files_inside_and_reports_missing_ones() {
        let (_dir, camp) = camp();
        std::fs::write(camp.root().join("scenes/a.blend"), b"x").unwrap();
        assert_eq!(
            camp.existing("scenes/a.blend"),
            Ok(camp.root().join("scenes/a.blend"))
        );
        assert_eq!(camp.existing("scenes/b.blend"), Err(PathError::NotFound));
    }

    #[test]
    fn symlinks_out_of_the_camp_are_refused_for_reads_and_writes() {
        let (dir, camp) = camp();
        let outside = dir.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("secret.txt"), b"x").unwrap();
        std::os::unix::fs::symlink(&outside, camp.root().join("assets/link")).unwrap();
        std::os::unix::fs::symlink(
            outside.join("secret.txt"),
            camp.root().join("scripts/file.py"),
        )
        .unwrap();

        assert_eq!(
            camp.existing("assets/link/secret.txt"),
            Err(PathError::Escapes)
        );
        assert_eq!(camp.existing("scripts/file.py"), Err(PathError::Escapes));
        assert_eq!(
            camp.writable("assets/link/new.png"),
            Err(PathError::Escapes)
        );
        assert_eq!(camp.writable("scripts/file.py"), Err(PathError::Escapes));
    }

    #[test]
    fn symlinks_that_stay_inside_are_allowed() {
        let (_dir, camp) = camp();
        std::fs::write(camp.root().join("assets/model.pmx"), b"x").unwrap();
        std::os::unix::fs::symlink(
            camp.root().join("assets/model.pmx"),
            camp.root().join("assets/alias.pmx"),
        )
        .unwrap();
        assert_eq!(
            camp.existing("assets/alias.pmx"),
            Ok(camp.root().join("assets/model.pmx"))
        );
    }

    #[test]
    fn writable_creates_missing_parents_inside_the_camp() {
        let (_dir, camp) = camp();
        let path = camp.writable("renders/clip/frame_0001.png").unwrap();
        assert_eq!(path, camp.root().join("renders/clip/frame_0001.png"));
        assert!(camp.root().join("renders/clip").is_dir(), "parent missing");
    }

    #[test]
    fn writable_refuses_lexical_escapes() {
        let (_dir, camp) = camp();
        assert_eq!(camp.writable("../x.blend"), Err(PathError::Parent));
        assert_eq!(camp.writable("/tmp/x.blend"), Err(PathError::Absolute));
    }

    #[test]
    fn relative_reports_paths_inside_the_camp_only() {
        let (dir, camp) = camp();
        assert_eq!(
            camp.relative(&camp.root().join("renders/a.png")),
            Some("renders/a.png".to_owned())
        );
        assert_eq!(camp.relative(dir.path()), None);
    }
}
