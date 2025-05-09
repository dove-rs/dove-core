//! Core downloader module for the Dove downloader
//!
//! This module implements the core download coordination logic.

use std::path::Path;
use std::sync::Arc;

use tokio::fs;
use tokio::sync::mpsc;

use crate::chunk::ChunkManager;
use crate::config::DownloadConfig;
use crate::error::{DoveError, DoveResult};
use crate::http::HttpClient;
use crate::progress::{Progress, ProgressTracker};
use crate::storage::StorageManager;
use crate::task_manager::TaskManager;
use crate::utils::{extract_filename, get_temporary_directory, validate_url};

/// Status of a download task
#[derive(Debug, Clone, PartialEq)]
pub enum DownloadStatus {
    /// Download is in progress
    InProgress,
    /// Download is paused
    Paused,
    /// Download is completed
    Completed,
    /// Download is cancelled
    Cancelled,
    /// Download failed with an error
    Failed(String),
}

/// Core downloader engine
pub struct Downloader {
    /// Configuration for the download
    config: Arc<DownloadConfig>,
    /// Storage manager for handling files
    storage: Arc<StorageManager>,
    /// Current status of the download
    status: DownloadStatus,
    /// Progress tracker
    progress_tracker: Option<Arc<ProgressTracker>>,
    /// Cancellation channel sender
    cancel_tx: Option<mpsc::Sender<()>>,
}

impl Downloader {
    /// Creates a new downloader with the given configuration
    pub fn new(config: Arc<DownloadConfig>) -> Self {
        let storage = Arc::new(StorageManager::new().unwrap_or_else(|_| {
            // Fallback to system temp dir if custom dir fails
            StorageManager::with_temp_dir(std::env::temp_dir()).unwrap()
        }));

        Self {
            config,
            storage,
            status: DownloadStatus::InProgress,
            progress_tracker: None,
            cancel_tx: None,
        }
    }

    /// Downloads a file from the specified URL to the destination path
    pub async fn download<P, F>(
        &mut self,
        url: &str,
        destination_dir: P,
        filename: impl Into<Option<String>>,
        mut progress_callback: F,
    ) -> DoveResult<()>
    where
        P: AsRef<Path>,
        F: FnMut(Progress) + Send + Copy + Sync + 'static,
    {
        let url = validate_url(url)?;

        let filename = Into::<Option<String>>::into(filename).unwrap_or(
            extract_filename(&url)
                .ok_or_else(|| DoveError::InvalidUrl("无法从URL提取文件名".to_string()))?,
        );

        let destination = destination_dir.as_ref().join(filename);

        // Check if destination already exists
        if destination.exists() {
            if self.config.overwrite {
                fs::remove_file(&destination).await?;
            } else {
                return Err(DoveError::FileAlreadyExists(destination));
            }
        }

        // Create HTTP client
        let http_client = Arc::new(HttpClient::new(self.config.clone())?);

        let url_str = url.to_string();

        // Get file information
        let file_info = http_client.get_file_info(&url).await?;

        // Log warning if range requests not supported but multiple threads configured
        if !file_info.supports_range && self.config.threads > 1 {
            println!(
                "Server doesn't support range requests, falling back to single-threaded download"
            );
        }

        let temporary_directory_id = get_temporary_directory(url_str.as_bytes());

        // Check if we can resume a previous download
        let can_resume = self
            .storage
            .load_state(&url_str, &temporary_directory_id, &destination)
            .await
            .unwrap_or(false);

        if !can_resume {
            // Initialize new download
            self.storage
                .init_download(
                    &url_str,
                    &temporary_directory_id,
                    &destination,
                    file_info.total_size,
                    file_info.etag,
                    file_info.last_modified,
                )
                .await?;
        }

        // Check disk space
        self.storage.check_disk_space(file_info.total_size).await?;

        // Initialize progress tracker
        let progress_tracker = Arc::new(ProgressTracker::new(
            file_info.total_size,
            self.config.progress_update_interval,
        ));
        self.progress_tracker = Some(progress_tracker.clone());

        // Create cancellation channel
        let (cancel_tx, cancel_rx) = mpsc::channel::<()>(1);
        self.cancel_tx = Some(cancel_tx);

        // Determine number of chunks
        let thread_count = if file_info.supports_range {
            self.config.threads
        } else {
            1
        };

        // Create chunk manager and chunks
        let chunk_manager = ChunkManager::new(self.storage.clone());
        let chunks = chunk_manager
            .create_chunks(&temporary_directory_id, file_info.total_size, thread_count)
            .await?;

        // Execute download tasks
        let result = TaskManager::execute_tasks(
            url_str,
            chunks,
            http_client,
            self.storage.clone(),
            progress_tracker.clone(),
            cancel_rx,
            progress_callback,
        )
        .await;

        // Handle result
        match result {
            Ok(()) => {
                // Mark progress as complete
                if let Some(tracker) = &self.progress_tracker {
                    tracker.mark_complete().await;
                    let progress = tracker.progress().await;
                    progress_callback(progress);
                }

                // Finalize download (merge chunks)
                self.storage
                    .finalize_download(&temporary_directory_id)
                    .await?;

                // Update status
                self.status = DownloadStatus::Completed;

                Ok(())
            }
            Err(e) => {
                // Update status
                self.status = DownloadStatus::Failed(e.to_string());
                Err(e)
            }
        }
    }

    /// Pauses the current download
    pub async fn pause(&mut self) -> bool {
        if let Some(tracker) = &self.progress_tracker {
            tracker.mark_paused(true).await;

            // Update status
            self.status = DownloadStatus::Paused;

            true
        } else {
            false
        }
    }

    /// Resumes a previously paused download
    pub async fn resume(&mut self) -> bool {
        if let Some(tracker) = &self.progress_tracker {
            tracker.mark_paused(false).await;

            // Update status
            self.status = DownloadStatus::InProgress;

            true
        } else {
            false
        }
    }

    /// Cancels the current download
    pub fn cancel(&self) -> bool {
        if let Some(cancel_tx) = &self.cancel_tx {
            let _ = cancel_tx.try_send(());
            true
        } else {
            false
        }
    }

    /// Gets the current progress
    pub async fn progress(&self) -> Option<Progress> {
        match self.progress_tracker.as_ref() {
            Some(tracker) => Some(tracker.progress().await),
            None => None,
        }
    }

    /// Gets the current status
    pub fn status(&self) -> DownloadStatus {
        self.status.clone()
    }
}
