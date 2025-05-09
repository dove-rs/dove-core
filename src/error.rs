//! Error handling module for the Dove downloader
//!
//! This module defines custom error types for various failure scenarios
//! that may occur during the download process.

use std::io;
use std::path::PathBuf;

use thiserror::Error;

pub type DoveResult<T> = std::result::Result<T, DoveError>;

/// Represents errors that can occur during the download process
#[derive(Error, Debug)]
pub enum DoveError {
    /// Error occurred during HTTP request/response
    #[error("HTTP error: {0}")]
    HttpError(#[from] reqwest::Error),

    /// Error occurred during I/O operations
    #[error("I/O error: {0}")]
    IoError(#[from] io::Error),

    /// Invalid URL provided
    #[error("Invalid URL: {0}")]
    InvalidUrl(String),

    /// Server returned an error status code
    #[error("Server error: {status} {message}")]
    ServerError { status: u16, message: String },

    /// Download was cancelled by the user
    #[error("Download cancelled")]
    Cancelled,

    /// Download was paused by the user
    #[error("Download paused")]
    Paused,

    /// Failed to create destination directory
    #[error("Failed to create directory at {path}: {error}")]
    DirectoryCreationFailed { path: PathBuf, error: io::Error },

    /// Failed to parse Content-Length header
    #[error("Failed to determine file size: {0}")]
    FileSizeUnknown(String),

    /// Server doesn't support range requests (required for resumable downloads)
    #[error("Server doesn't support range requests")]
    RangeRequestsNotSupported,

    /// Failed to parse range header
    #[error("Failed to parse range header: {0}")]
    RangeParseError(String),

    /// Checksum verification failed
    #[error("Checksum verification failed: expected {expected}, got {actual}")]
    ChecksumMismatch { expected: String, actual: String },

    /// Timeout occurred during download
    #[error("Download timed out after {0:?}")]
    Timeout(std::time::Duration),

    /// Maximum retry count exceeded
    #[error("Maximum retry count exceeded ({0})")]
    MaxRetriesExceeded(usize),

    /// Failed to serialize or deserialize progress data
    #[error("Failed to {action} progress data: {message}")]
    ProgressDataError { action: String, message: String },

    /// Destination file already exists and overwrite is not allowed
    #[error("Destination file already exists: {0}")]
    FileAlreadyExists(PathBuf),

    /// Insufficient disk space
    #[error("Insufficient disk space: need {needed} bytes, available {available} bytes")]
    InsufficientDiskSpace { needed: u64, available: u64 },

    /// Other unexpected errors
    #[error("Unexpected error: {0}")]
    Other(String),
}

impl DoveError {
    /// Returns true if the error is transient and the operation might succeed if retried
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::HttpError(e) => e.is_timeout() || e.is_connect(),
            Self::IoError(e) => matches!(
                e.kind(),
                io::ErrorKind::ConnectionReset
                    | io::ErrorKind::ConnectionAborted
                    | io::ErrorKind::TimedOut
                    | io::ErrorKind::Interrupted
            ),
            Self::ServerError { status, .. } => *status >= 500 && *status < 600,
            Self::Timeout(_) => true,
            _ => false,
        }
    }

    /// Returns true if the error is related to user cancellation
    pub fn is_cancelled(&self) -> bool {
        matches!(self, Self::Cancelled)
    }

    /// Returns true if the error is related to user pausing
    pub fn is_paused(&self) -> bool {
        matches!(self, Self::Paused)
    }

    /// Creates a new server error
    pub fn server_error(status: u16, message: impl Into<String>) -> Self {
        Self::ServerError {
            status,
            message: message.into(),
        }
    }

    /// Creates a new progress data error
    pub fn progress_data_error(action: impl Into<String>, message: impl Into<String>) -> Self {
        Self::ProgressDataError {
            action: action.into(),
            message: message.into(),
        }
    }
}
