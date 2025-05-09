//! Dove Core - A multi-threaded file downloader with resume capability and progress tracking
//!
//! This crate provides functionality for downloading files with the following features:
//! - Multi-threaded downloading for improved performance
//! - Resume capability for interrupted downloads
//! - Real-time progress tracking and persistence
//! - Error handling and retry mechanisms
//! - Simple and intuitive API

use std::path::Path;
use std::sync::Arc;

use tokio::sync::Mutex;

mod chunk;
mod config;
mod downloader;
mod error;
mod http;
mod progress;
mod storage;
mod task_manager;
mod utils;

pub use config::DownloadConfig;
pub use downloader::Downloader;
pub use error::{DoveError, DoveResult};
pub use progress::{Progress, ProgressTracker};
pub use utils::{format_duration, format_size, format_speed};

#[macro_use]
extern crate tracing;

/// The main entry point for the Dove downloader library.
///
/// # Examples
///
/// ```
/// use dove_core::{Dove, DoveResult, DownloadConfig};
/// use std::path::Path;
///
/// #[tokio::main]
/// async fn main() -> DoveResult<()> {
///     let url = "https://example.com/large-file.zip";
///     let destination_dir = Path::new("./downloads");
///     
///     let config = DownloadConfig::new()
///         .with_threads(4)
///         .with_chunk_size(1024 * 1024) // 1MB chunks
///         .with_retry_count(3);
///     
///     let mut dove = Dove::new(config);
///     
///     // Start download with progress callback
///     dove.download(url, destination_dir, None, |progress| {
///         println!("Downloaded: {}%, Speed: {} MB/s",
///             progress.percentage(),
///             progress.speed() / (1024.0 * 1024.0));
///     }).await?;
///     
///     Ok(())
/// }
/// ```
pub struct Dove {
    downloader: Arc<Mutex<Downloader>>,
}

impl Dove {
    /// Creates a new Dove downloader instance with the specified configuration.
    pub fn new(config: DownloadConfig) -> Self {
        let downloader = Arc::new(Mutex::new(Downloader::new(Arc::new(config))));
        Self { downloader }
    }

    /// Downloads a file from the specified URL to the destination path.
    ///
    /// The progress_callback is called periodically with the current download progress.
    pub async fn download<P, F>(
        &self,
        url: &str,
        destination_dir: P,
        filename: impl Into<Option<String>>,
        progress_callback: F,
    ) -> DoveResult<()>
    where
        P: AsRef<Path>,
        F: FnMut(Progress) + Send + Copy + Sync + 'static,
    {
        let downloader = self.downloader.clone();
        let mut downloader = downloader.lock().await;
        downloader
            .download(url, destination_dir, filename, progress_callback)
            .await
    }

    /// Pauses the current download, if any.
    ///
    /// Returns true if a download was paused, false otherwise.
    pub async fn pause(&self) -> bool {
        let mut downloader = self.downloader.lock().await;
        downloader.pause().await
    }

    /// Resumes a previously paused download, if any.
    ///
    /// Returns true if a download was resumed, false otherwise.
    pub async fn resume(&self) -> bool {
        let mut downloader = self.downloader.lock().await;
        downloader.resume().await
    }

    /// Cancels the current download, if any.
    ///
    /// Returns true if a download was cancelled, false otherwise.
    pub async fn cancel(&self) -> bool {
        let downloader = self.downloader.lock().await;
        downloader.cancel()
    }
}

impl Default for Dove {
    fn default() -> Self {
        Dove::new(DownloadConfig::default())
    }
}
