//! Where an animation job keeps its frames and the MP4 in the making.
use crate::basecamp::BaseCamp;
use std::{
    io,
    path::{Path, PathBuf},
};

/// Directory under the base camp for jobs the temp directory has no room for.
pub const CAMP_DIR: &str = ".tmp";

/// A job's private directory, removed with everything in it when dropped.
#[derive(Debug)]
pub struct Scratch {
    dir: PathBuf,
    in_camp: bool,
}

impl Scratch {
    /// Under `run_dir`, which is in memory where the temp directory is a tmpfs.
    pub fn temp(run_dir: &Path, id: &str) -> io::Result<Self> {
        Self::create(run_dir.join(format!("job-{id}")), false)
    }

    /// Under the base camp, for when the temp directory has no room.
    pub fn camp(camp: &BaseCamp, id: &str) -> io::Result<Self> {
        let dir = camp
            .writable(&format!("{CAMP_DIR}/job-{id}"))
            .map_err(io::Error::other)?;
        Self::create(dir, true)
    }

    fn create(dir: PathBuf, in_camp: bool) -> io::Result<Self> {
        let scratch = Self { dir, in_camp };
        std::fs::create_dir_all(scratch.frames())?;
        Ok(scratch)
    }

    /// Where Blender writes the frames that wait for ffmpeg.
    #[must_use]
    pub fn frames(&self) -> PathBuf {
        self.dir.join("frames")
    }

    /// The MP4 ffmpeg writes until it is complete.
    #[must_use]
    pub fn mp4(&self) -> PathBuf {
        self.dir.join("out.mp4")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
        if self.in_camp
            && let Some(parent) = self.dir.parent()
        {
            // Only succeeds once no other job is left in it.
            let _ = std::fs::remove_dir(parent);
        }
    }
}

/// Bytes an unprivileged process can still write to the file system of `dir`.
pub fn free_bytes(dir: &Path) -> io::Result<u64> {
    let stat = rustix::fs::statvfs(dir)?;
    Ok(stat.f_bavail.saturating_mul(stat.f_frsize))
}

/// Whether the temp directory can take a job: `free` bytes are left in it.
#[must_use]
pub fn roomy(free: &io::Result<u64>, reserve: u64) -> bool {
    free.as_ref().is_ok_and(|free| *free >= reserve)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_temp_directory_is_used_while_the_reserve_is_free() {
        assert!(roomy(&Ok(64), 64), "exactly the reserve refused");
        assert!(roomy(&Ok(u64::MAX), 64), "plenty refused");
        assert!(roomy(&Ok(0), 0), "no reserve asked for");
        assert!(!roomy(&Ok(63), 64), "less than the reserve accepted");
        assert!(
            !roomy(&Err(io::Error::other("gone")), 0),
            "an unreadable directory accepted"
        );
    }

    #[test]
    fn a_scratch_directory_is_gone_once_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let camp = BaseCamp::open(&dir.path().join("camp")).unwrap();
        for scratch in [
            Scratch::temp(dir.path(), "a").unwrap(),
            Scratch::camp(&camp, "a").unwrap(),
        ] {
            let frames = scratch.frames();
            std::fs::write(frames.join("frame_0001.png"), b"png").unwrap();
            std::fs::write(scratch.mp4(), b"mp4").unwrap();
            drop(scratch);
            assert!(!frames.exists(), "{} stayed", frames.display());
        }
        assert!(!camp.root().join(CAMP_DIR).exists(), "the camp kept .tmp");
        assert!(dir.path().join("camp/renders").exists(), "too much removed");
    }

    #[test]
    fn each_scratch_directory_is_under_its_own_root() {
        let dir = tempfile::tempdir().unwrap();
        let camp = BaseCamp::open(&dir.path().join("camp")).unwrap();
        let temp = Scratch::temp(dir.path(), "b").unwrap();
        let spilled = Scratch::camp(&camp, "b").unwrap();
        assert_eq!(temp.frames(), dir.path().join("job-b/frames"));
        assert_eq!(
            spilled.frames(),
            camp.root().join(CAMP_DIR).join("job-b/frames")
        );
        assert!(
            temp.frames().is_dir() && spilled.frames().is_dir(),
            "not created"
        );
    }
}
