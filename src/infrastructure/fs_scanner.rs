use std::fmt::Write as _;
use std::fs::{self, File};
use std::io::{BufReader, Read};
use std::path::Path;
use std::time::UNIX_EPOCH;

use sha2::{Digest, Sha256};
use walkdir::{DirEntry, WalkDir};

use crate::application::ports::FileScanner;
use crate::domain::errors::MidxError;
use crate::domain::models::{ScannedFile, Sha256Hash};

/// Filesystem scanner backed by `walkdir` and `sha2`.
#[derive(Debug, Default, Clone, Copy)]
pub struct FsScanner;

impl FsScanner {
    /// Creates a new `FsScanner`.
    pub fn new() -> Self {
        Self
    }

    fn is_hidden(entry: &DirEntry) -> bool {
        if entry.depth() == 0 {
            return false;
        }
        entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with('.'))
    }

    fn is_markdown(entry: &DirEntry) -> bool {
        entry.file_type().is_file()
            && entry
                .path()
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
    }

    fn hash_file(path: &Path) -> Result<Sha256Hash, MidxError> {
        let file = File::open(path)?;
        let mut reader = BufReader::new(file);
        let mut hasher = Sha256::new();
        let mut buf = [0u8; 8192];

        loop {
            let read = reader.read(&mut buf)?;
            if read == 0 {
                break;
            }
            hasher.update(&buf[..read]);
        }

        let digest = hasher.finalize();
        let mut hex = String::with_capacity(digest.len() * 2);
        for byte in digest {
            let _ = write!(&mut hex, "{byte:02x}");
        }
        Ok(Sha256Hash::new(hex))
    }

    fn read_mtime_millis(path: &Path) -> Result<i64, MidxError> {
        let meta = fs::metadata(path)?;
        let modified = meta.modified()?;
        let duration = modified.duration_since(UNIX_EPOCH).map_err(|err| {
            MidxError::Scanner(format!("invalid mtime for '{}': {err}", path.display()))
        })?;
        i64::try_from(duration.as_millis())
            .map_err(|_| MidxError::Scanner(format!("mtime overflow for '{}'", path.display())))
    }
}

impl FileScanner for FsScanner {
    fn scan(&self, root: &Path) -> Result<Vec<ScannedFile>, MidxError> {
        let mut files = Vec::new();

        let walker = WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|entry| !Self::is_hidden(entry));

        for entry_res in walker {
            let entry = entry_res.map_err(|err| MidxError::Scanner(err.to_string()))?;
            if !Self::is_markdown(&entry) {
                continue;
            }

            let abs_path = entry.path();
            let rel_path = abs_path
                .strip_prefix(root)
                .map_err(|err| MidxError::Scanner(err.to_string()))?;

            let normalized_path = rel_path
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");

            let hash = Self::hash_file(abs_path)?;
            let mtime = Self::read_mtime_millis(abs_path)?;

            files.push(ScannedFile {
                path: normalized_path,
                hash,
                mtime,
            });
        }

        files.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(files)
    }

    fn read_content(&self, root: &Path, relative_path: &str) -> Result<String, MidxError> {
        let full_path = root.join(relative_path);
        Ok(fs::read_to_string(full_path)?)
    }
}
