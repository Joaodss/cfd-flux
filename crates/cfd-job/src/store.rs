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

/// Result directories of many jobs under one root: `<root>/<job id>/` (the server's results).
/// Local disk for now; an object store (S3/MinIO) would make this a trait.
#[derive(Debug, Clone)]
pub struct LocalStore {
    root: PathBuf,
}

impl LocalStore {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(&root)
            .with_context(|| format!("cannot create {}", root.display()))?;
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Job ids become directory names: only letters, digits, `-` and `_` are accepted.
    fn path(&self, id: &str) -> Result<PathBuf> {
        let ok = !id.is_empty()
            && id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        if !ok {
            bail!("invalid job id {id:?}");
        }
        Ok(self.root.join(id))
    }

    /// A new, empty directory for a job (an existing non-empty one is an error).
    pub fn create(&self, id: &str) -> Result<ResultDir> {
        ResultDir::create(&self.path(id)?, false)
    }

    /// The directory of an existing job, if there is one.
    pub fn open(&self, id: &str) -> Option<ResultDir> {
        let p = self.path(id).ok()?;
        p.is_dir().then(|| ResultDir::open(&p))
    }

    /// Deletes a job's results (nothing to do if there are none).
    pub fn delete(&self, id: &str) -> Result<()> {
        let p = self.path(id)?;
        if p.exists() {
            std::fs::remove_dir_all(&p)
                .with_context(|| format!("cannot remove {}", p.display()))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_ids_cannot_escape_the_root() {
        let root = std::env::temp_dir().join(format!("cfd-job-store-{}", std::process::id()));
        let store = LocalStore::new(&root).unwrap();
        for bad in ["", "..", "../x", "a/b", r"a\b", "c:", "x y"] {
            assert!(store.create(bad).is_err(), "{bad:?}");
            assert!(store.open(bad).is_none(), "{bad:?}");
            assert!(store.delete(bad).is_err(), "{bad:?}");
        }
        let dir = store.create("0192-abc_D").unwrap();
        dir.write("meta.json", "{}").unwrap();
        assert!(store.open("0192-abc_D").is_some());
        assert!(store.create("0192-abc_D").is_err(), "not empty");
        store.delete("0192-abc_D").unwrap();
        assert!(store.open("0192-abc_D").is_none());
        let _ = std::fs::remove_dir_all(&root);
    }
}
