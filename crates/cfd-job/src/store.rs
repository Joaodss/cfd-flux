//! Where a run's results go: one directory in the layout of `docs/03-data-model.md` §6.2.

use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

/// Name of the frame file with the given index, inside `frames/`.
pub fn frame_file_name(index: u32) -> String {
    format!("{index:06}.bin")
}

/// The directory of one run.
#[derive(Debug, Clone)]
pub struct ResultDir {
    path: PathBuf,
}

impl ResultDir {
    /// Prepares `path` for a run. A non-empty directory is an error unless `overwrite` is set; then
    /// only what a run writes into subdirectories (`frames/`, `png/`) is removed, and the files are
    /// overwritten as the run writes them.
    pub fn create(path: &Path, overwrite: bool) -> Result<Self> {
        if path.exists() {
            let non_empty = std::fs::read_dir(path)?.next().is_some();
            if non_empty && !overwrite {
                bail!(
                    "{} is not empty; use --force to overwrite a previous run",
                    path.display()
                );
            }
            for sub in ["frames", "png"] {
                let p = path.join(sub);
                if p.exists() {
                    std::fs::remove_dir_all(&p)
                        .with_context(|| format!("cannot remove {}", p.display()))?;
                }
            }
        }
        std::fs::create_dir_all(path)
            .with_context(|| format!("cannot create {}", path.display()))?;
        Ok(Self {
            path: path.to_path_buf(),
        })
    }

    /// An existing result directory, for reading.
    pub fn open(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Writes (or replaces) a file at the top of the directory.
    pub fn write(&self, name: &str, contents: impl AsRef<[u8]>) -> Result<()> {
        let p = self.path.join(name);
        std::fs::write(&p, contents).with_context(|| format!("cannot write {}", p.display()))
    }

    /// Creates a file at the top of the directory, for streaming writes.
    pub fn create_file(&self, name: &str) -> Result<BufWriter<File>> {
        let p = self.path.join(name);
        Ok(BufWriter::new(File::create(&p).with_context(|| {
            format!("cannot create {}", p.display())
        })?))
    }

    pub fn frames_dir(&self) -> PathBuf {
        self.path.join("frames")
    }

    pub fn frame_path(&self, index: u32) -> PathBuf {
        self.frames_dir().join(frame_file_name(index))
    }
}
