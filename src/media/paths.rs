use std::io;
use std::path::{Path, PathBuf};

/// Locations of generated files for one media item: `{data_dir}/derived/{id}/`.
#[derive(Debug, Clone)]
pub struct DerivedPaths {
    dir: PathBuf,
}

impl DerivedPaths {
    pub fn new(data_dir: &Path, media_id: &str) -> Self {
        Self {
            dir: data_dir.join("derived").join(media_id),
        }
    }

    pub fn dir(&self) -> PathBuf {
        self.dir.clone()
    }

    pub fn poster(&self) -> PathBuf {
        self.dir.join("poster.jpg")
    }

    pub fn preview(&self) -> PathBuf {
        self.dir.join("preview.mp4")
    }

    pub fn thumb(&self) -> PathBuf {
        self.dir.join("thumb.jpg")
    }

    pub fn rendition(&self, height: u32) -> PathBuf {
        self.dir.join(format!("{height}p.mp4"))
    }

    /// Per-job temp output so one job's cleanup never touches another's file.
    pub fn rendition_tmp(&self, height: u32, job_id: i64) -> PathBuf {
        self.dir.join(format!("{height}p.{job_id}.tmp.mp4"))
    }

    /// Removes every temp output of a rung (jobs unknown); missing files are fine.
    pub fn remove_rendition_tmps(&self, height: u32) {
        let (prefix, suffix) = (format!("{height}p."), ".tmp.mp4");
        let Ok(rd) = std::fs::read_dir(&self.dir) else {
            return;
        };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with(&prefix) && name.ends_with(suffix) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }

    /// Removes the derived directory; a missing directory is not an error.
    pub fn remove_all(&self) -> io::Result<()> {
        match std::fs::remove_dir_all(&self.dir) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            other => other,
        }
    }
}
