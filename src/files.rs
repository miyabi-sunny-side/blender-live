//! Listing the base camp for the browser and the MCP tools.
use crate::basecamp::BaseCamp;
use serde::Serialize;
use std::{io, path::Path, time::UNIX_EPOCH};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Image,
    Video,
    /// A directory of numbered PNG frames under `renders/`.
    Sequence,
    Blend,
    Script,
    Audio,
    Model,
    Other,
}

#[must_use]
pub fn kind_of(path: &str) -> Kind {
    let ext = Path::new(path)
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "webp" | "gif" | "exr" | "tga" | "bmp" => Kind::Image,
        "mp4" | "webm" | "mov" | "mkv" => Kind::Video,
        "blend" => Kind::Blend,
        "py" => Kind::Script,
        "wav" | "mp3" | "ogg" | "flac" | "m4a" => Kind::Audio,
        "pmx" | "pmd" | "vmd" | "glb" | "gltf" | "fbx" | "obj" | "stl" | "usd" | "usdz" => {
            Kind::Model
        }
        _ => Kind::Other,
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Entry {
    /// `/`-separated path relative to the base camp.
    pub path: String,
    pub kind: Kind,
    pub size: u64,
    pub modified_ms: i64,
    /// Frame count and first frame of a sequence.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frames: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cover: Option<String>,
}

/// Newest first; equal times fall back to the path so the order is stable.
pub fn newest_first(entries: &mut [Entry]) {
    entries.sort_by(|a, b| {
        b.modified_ms
            .cmp(&a.modified_ms)
            .then_with(|| a.path.cmp(&b.path))
    });
}

/// Lists one of the standard directories. `renders/` shows its files and
/// each subdirectory as one sequence; the others list files recursively.
/// Symlinks are skipped so a listing never reaches outside the camp.
pub fn list(camp: &BaseCamp, dir: &str) -> io::Result<Vec<Entry>> {
    if !crate::basecamp::DIRS.contains(&dir) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("unknown directory {dir:?}"),
        ));
    }
    let root = camp.root().join(dir);
    let mut entries = Vec::new();
    if dir == "renders" {
        for item in std::fs::read_dir(&root)? {
            let item = item?;
            let meta = item.metadata()?;
            let path = item.path();
            if meta.is_file() {
                entries.push(file_entry(camp, &path, &meta));
            } else if meta.is_dir() {
                entries.extend(sequence(camp, &path)?);
            }
        }
    } else {
        walk(camp, &root, &mut entries)?;
    }
    newest_first(&mut entries);
    Ok(entries)
}

fn file_entry(camp: &BaseCamp, path: &Path, meta: &std::fs::Metadata) -> Entry {
    let rel = camp.relative(path).unwrap_or_default();
    Entry {
        kind: kind_of(&rel),
        path: rel,
        size: meta.len(),
        modified_ms: modified_ms(meta),
        frames: None,
        cover: None,
    }
}

fn sequence(camp: &BaseCamp, dir: &Path) -> io::Result<Option<Entry>> {
    let mut frames = Vec::new();
    let mut size = 0;
    let mut modified = 0;
    for item in std::fs::read_dir(dir)? {
        let item = item?;
        let meta = item.metadata()?;
        let path = item.path();
        if meta.is_file() && kind_of(&path.to_string_lossy()) == Kind::Image {
            size += meta.len();
            modified = modified.max(modified_ms(&meta));
            frames.push(path);
        }
    }
    frames.sort();
    let Some(first) = frames.first() else {
        return Ok(None);
    };
    Ok(Some(Entry {
        path: camp.relative(dir).unwrap_or_default(),
        kind: Kind::Sequence,
        size,
        modified_ms: modified,
        frames: u32::try_from(frames.len()).ok(),
        cover: camp.relative(first),
    }))
}

fn walk(camp: &BaseCamp, dir: &Path, entries: &mut Vec<Entry>) -> io::Result<()> {
    for item in std::fs::read_dir(dir)? {
        let item = item?;
        // DirEntry::metadata does not follow symlinks, so they are skipped.
        let meta = item.metadata()?;
        if meta.is_file() {
            entries.push(file_entry(camp, &item.path(), &meta));
        } else if meta.is_dir() {
            walk(camp, &item.path(), entries)?;
        }
    }
    Ok(())
}

fn modified_ms(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, time::Duration};

    #[test]
    fn kinds_follow_the_extension_case_insensitively() {
        for (path, kind) in [
            ("renders/a.png", Kind::Image),
            ("renders/a.JPG", Kind::Image),
            ("renders/a.webp", Kind::Image),
            ("renders/a.mp4", Kind::Video),
            ("renders/a.webm", Kind::Video),
            ("scenes/a.blend", Kind::Blend),
            ("scripts/a.py", Kind::Script),
            ("assets/a.wav", Kind::Audio),
            ("assets/a.mp3", Kind::Audio),
            ("assets/m/モカ.pmx", Kind::Model),
            ("assets/a.glb", Kind::Model),
            ("assets/a.fbx", Kind::Model),
            ("assets/a.tga", Kind::Image),
            ("assets/readme.txt", Kind::Other),
            ("assets/noext", Kind::Other),
        ] {
            assert_eq!(kind_of(path), kind, "{path}");
        }
    }

    fn entry(path: &str, modified_ms: i64) -> Entry {
        Entry {
            path: path.into(),
            kind: kind_of(path),
            size: 0,
            modified_ms,
            frames: None,
            cover: None,
        }
    }

    #[test]
    fn newest_first_breaks_ties_by_path() {
        let mut entries = vec![
            entry("renders/old.png", 1),
            entry("renders/b.png", 5),
            entry("renders/a.png", 5),
            entry("renders/new.mp4", 9),
        ];
        newest_first(&mut entries);
        let paths: Vec<_> = entries.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(
            paths,
            [
                "renders/new.mp4",
                "renders/a.png",
                "renders/b.png",
                "renders/old.png"
            ]
        );
    }

    fn touch(path: &Path, bytes: &[u8], age_secs: u64) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
        let time = std::time::SystemTime::now() - Duration::from_secs(age_secs);
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(time)
            .unwrap();
    }

    #[test]
    fn renders_show_files_and_sequences_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        let camp = BaseCamp::open(dir.path()).unwrap();
        let r = camp.root().join("renders");
        touch(&r.join("old.png"), b"png", 300);
        touch(&r.join("clip.mp4"), b"mp4!", 10);
        touch(&r.join("clip/frame_0002.png"), b"2", 20);
        touch(&r.join("clip/frame_0001.png"), b"1", 30);
        touch(&r.join("clip/notes.txt"), b"x", 400);
        std::os::unix::fs::symlink("/etc/hostname", r.join("escape.png")).unwrap();

        let entries = list(&camp, "renders").unwrap();
        let summary: Vec<_> = entries
            .iter()
            .map(|e| (e.path.as_str(), e.kind, e.frames, e.cover.as_deref()))
            .collect();
        assert_eq!(
            summary,
            [
                ("renders/clip.mp4", Kind::Video, None, None),
                (
                    "renders/clip",
                    Kind::Sequence,
                    Some(2),
                    Some("renders/clip/frame_0001.png")
                ),
                ("renders/old.png", Kind::Image, None, None),
            ]
        );
        assert_eq!(entries[0].size, 4);
    }

    #[test]
    fn other_directories_list_files_recursively() {
        let dir = tempfile::tempdir().unwrap();
        let camp = BaseCamp::open(dir.path()).unwrap();
        touch(&camp.root().join("assets/moca/宮舞モカ.pmx"), b"pmx", 50);
        touch(&camp.root().join("assets/bgm.wav"), b"wav", 5);
        let paths: Vec<_> = list(&camp, "assets")
            .unwrap()
            .into_iter()
            .map(|e| (e.path, e.kind))
            .collect();
        assert_eq!(
            paths,
            [
                ("assets/bgm.wav".to_owned(), Kind::Audio),
                ("assets/moca/宮舞モカ.pmx".to_owned(), Kind::Model),
            ]
        );
    }

    #[test]
    fn unknown_directories_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let camp = BaseCamp::open(dir.path()).unwrap();
        for name in ["..", "/etc", "other", "renders/clip"] {
            assert!(list(&camp, name).is_err(), "listed {name}");
        }
    }
}
