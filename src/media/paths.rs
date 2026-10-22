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

    /// Removes the derived directory; a missing directory is not an error.
    pub fn remove_all(&self) -> io::Result<()> {
        match std::fs::remove_dir_all(&self.dir) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            other => other,
        }
    }
}
