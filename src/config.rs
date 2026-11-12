use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::application::sync::DEFAULT_BATCH_SIZE;
use crate::domain::chunker::DEFAULT_MAX_CHUNK_WORDS;
use crate::domain::errors::MidxError;

pub const CONFIG_FILE_NAME: &str = ".midx.toml";
pub const DEFAULT_DB_REL_PATH: &str = ".midx/index.db";
pub const DEFAULT_MODEL_NAME: &str = "MultilingualE5Small";

/// Persistent repository configuration stored in `.midx.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub model: String,
    pub batch_size: usize,
    pub max_chunk_words: usize,
    pub db_path: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            model: DEFAULT_MODEL_NAME.to_string(),
            batch_size: DEFAULT_BATCH_SIZE,
            max_chunk_words: DEFAULT_MAX_CHUNK_WORDS,
            db_path: DEFAULT_DB_REL_PATH.to_string(),
        }
    }
}

/// Configuration paired with the discovered repository root directory.
#[derive(Debug, Clone)]
pub struct LoadedConfig {
    pub root_dir: PathBuf,
    pub config: Config,
}

impl LoadedConfig {
    /// Searches `start_dir` and its parent directories for `.midx.toml`.
    pub fn discover(start_dir: &Path) -> Result<Self, MidxError> {
        let abs_start = if start_dir.is_absolute() {
            start_dir.to_path_buf()
        } else {
            std::env::current_dir()?.join(start_dir)
        };

        for ancestor in abs_start.ancestors() {
            let candidate = ancestor.join(CONFIG_FILE_NAME);
            if candidate.is_file() {
                let raw = fs::read_to_string(&candidate)?;
                let config: Config = toml::from_str(&raw)
                    .map_err(|err| MidxError::Config(format!("invalid {CONFIG_FILE_NAME}: {err}")))?;

                if config.batch_size == 0 {
                    return Err(MidxError::Config(
                        "batch_size must be greater than 0".to_string(),
                    ));
                }
                if config.max_chunk_words == 0 {
                    return Err(MidxError::Config(
                        "max_chunk_words must be greater than 0".to_string(),
                    ));
                }

                return Ok(Self {
                    root_dir: ancestor.to_path_buf(),
                    config,
                });
            }
        }

        Err(MidxError::NotInitialized(abs_start))
    }

    /// Resolves the SQLite database path relative to `root_dir`.
    pub fn db_abs_path(&self) -> PathBuf {
        let path = Path::new(&self.config.db_path);
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.root_dir.join(path)
        }
    }
}

/// Creates `.midx.toml` in `target_dir` if it does not already exist.
/// Returns `Ok(true)` if a new config file was written, `Ok(false)` if one already existed.
pub fn write_default_config_if_missing(target_dir: &Path) -> Result<(PathBuf, bool), MidxError> {
    let config_path = target_dir.join(CONFIG_FILE_NAME);
    if config_path.exists() {
        return Ok((config_path, false));
    }

    let serialized = toml::to_string_pretty(&Config::default())
        .map_err(|err| MidxError::Config(format!("failed to serialize default config: {err}")))?;
    fs::write(&config_path, serialized)?;
    Ok((config_path, true))
}

/// Ensures `.midx/` is ignored in `target_dir/.gitignore`.
/// Returns `Ok(true)` if `.gitignore` was created or updated.
pub fn ensure_gitignore_entry(target_dir: &Path) -> Result<bool, MidxError> {
    let gitignore_path = target_dir.join(".gitignore");
    let existing = if gitignore_path.exists() {
        fs::read_to_string(&gitignore_path)?
    } else {
        String::new()
    };

    let already_listed = existing.lines().any(|line| {
        let trimmed = line.trim();
        trimmed == ".midx/" || trimmed == ".midx" || trimmed == "/.midx/" || trimmed == "/.midx"
    });

    if already_listed {
        return Ok(false);
    }

    let mut updated = existing;
    if !updated.is_empty() && !updated.ends_with('\n') {
        updated.push('\n');
    }
    updated.push_str(".midx/\n");
    fs::write(&gitignore_path, updated)?;
    Ok(true)
}

