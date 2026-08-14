use std::path::{Path, PathBuf};
use thiserror::Error;

use crate::manifest::ParseError;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("manifest not configured; pass --manifest or set SUMI_MANIFEST")]
    ManifestNotConfigured,

    #[error("invalid manifest: {0}")]
    InvalidManifest(String),

    #[error("invalid selection: {0}")]
    InvalidSelection(String),

    #[error("invalid state: {0}")]
    InvalidState(String),

    #[error("invalid manifest at '{}': {source}", path.display())]
    ParseManifest {
        path: PathBuf,
        #[source]
        source: ParseError,
    },

    #[error("could not parse JSON at '{}': {source}", path.display())]
    ParseJson {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("could not serialize selection: {0}")]
    SerializeSelection(#[source] serde_json::Error),

    #[error("could not {operation} '{}': {source}", path.display())]
    Filesystem {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

impl AppError {
    pub fn fs(operation: &'static str, path: impl AsRef<Path>, source: std::io::Error) -> Self {
        Self::Filesystem {
            operation,
            path: path.as_ref().to_path_buf(),
            source,
        }
    }
}
