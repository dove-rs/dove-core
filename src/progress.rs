//! Progress tracking module for the Dove downloader
//!
//! This module provides functionality for tracking and reporting download progress,
//! including download speed, completion percentage, and estimated time remaining.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

/// Represents the current progress of a download
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Progress {
    /// Total size of the file in bytes
    total_size: u64,
    /// Number of bytes downloaded so far
    downloaded: u64,
    /// Current download speed in bytes per second
    speed: u64,
    /// Estimated time remaining in seconds
    pub eta: Option<u64>,
    /// Timestamp when the download started
    pub start_time: std::time::SystemTime,
    /// Whether the download is complete
    pub is_complete: bool,
    /// Whether the download is paused
    pub is_paused: bool,
    /// List of chunk ranges that have been completed
    pub completed_chunks: Vec<(u64, u64)>,
}

impl Progress {
    /// Creates a new Progress instance
    pub fn new(total_size: u64) -> Self {
        trace!(total_size, "Creating new Progress instance");
        Self {
            total_size,
            downloaded: 0,
            speed: 0,
            eta: None,
            start_time: std::time::SystemTime::now(),
            is_complete: false,
            is_paused: false,
            completed_chunks: Vec::new(),
        }
    }

    /// Returns the percentage of the download that has been completed
    pub fn percentage(&self) -> f64 {
        if self.total_size == 0 {
            trace!("Total size is zero, returning 0% progress");
            return 0.0;
        }
        let percentage = (self.downloaded as f64 / self.total_size as f64) * 100.0;
        trace!(
            percentage,
            downloaded = self.downloaded,
            total = self.total_size,
            "Calculated download percentage"
        );
        percentage
    }

    /// Returns the number of bytes downloaded so far
    pub fn downloaded(&self) -> u64 {
        self.downloaded
    }

    /// Returns the total size of the file in bytes
    pub fn total(&self) -> u64 {
        self.total_size
    }

    /// Returns the current download speed in bytes per second
    pub fn speed(&self) -> u64 {
        self.speed
    }

    /// Returns the estimated time remaining in seconds
    pub fn eta(&self) -> Option<u64> {
        self.eta
    }

    /// Returns the elapsed time since the download started
    pub fn elapsed(&self) -> Duration {
        self.start_time.elapsed().unwrap_or(Duration::from_secs(0))
    }

    /// Marks the download as complete
    pub fn mark_complete(&mut self) {
        trace!("Marking download as complete");
        self.is_complete = true;
        self.downloaded = self.total_size;
        self.eta = Some(0);
        debug!(
            downloaded = self.downloaded,
            total_size = self.total_size,
            "Download completed"
        );
    }

    /// Marks the download as paused
    pub fn mark_paused(&mut self, paused: bool) {
        debug!(paused, "Marking download as paused");
        self.is_paused = paused;
    }

    /// Adds a completed chunk to the progress
    pub fn add_completed_chunk(&mut self, start: u64, end: u64) {
        debug!(
            start,
            end,
            chunk_size = end - start + 1,
            "Adding completed chunk"
        );
        self.completed_chunks.push((start, end));
        // Recalculate downloaded bytes based on completed chunks
        self.downloaded = self
            .completed_chunks
            .iter()
            .map(|(start, end)| end - start + 1)
            .sum();
        debug!(
            chunks_count = self.completed_chunks.len(),
            total_downloaded = self.downloaded,
            "Updated downloaded bytes after adding chunk"
        );
    }
}

/// Tracks download progress and provides periodic updates
#[derive(Debug, Clone)]
pub struct ProgressTracker {
    /// The current progress
    progress: Arc<Mutex<Progress>>,
    /// The interval between progress updates
    update_interval: Duration,
    /// Moving average window for speed calculation (in seconds)
    speed_window: Duration,
    /// History of download speeds for moving average calculation
    /// Stores (timestamp, bytes_downloaded_in_interval_across_all_threads)
    speed_history: Arc<Mutex<Vec<(Instant, u64)>>>,
    /// Timestamp of the last speed calculation
    last_speed_calc_time: Arc<Mutex<Instant>>,
    /// Total bytes downloaded at the time of the last speed calculation
    total_bytes_at_last_speed_calc: Arc<Mutex<u64>>,
}

impl ProgressTracker {
    /// Creates a new ProgressTracker
    pub fn new(total_size: u64, update_interval: Duration) -> Self {
        debug!(
            total_size,
            update_interval_ms = update_interval.as_millis(),
            "Creating new ProgressTracker"
        );
        let progress = Progress::new(total_size);
        Self {
            progress: Arc::new(Mutex::new(progress)),
            update_interval,
            speed_window: Duration::from_secs(5), // Default, can be made configurable
            speed_history: Arc::new(Mutex::new(Vec::new())),
            last_speed_calc_time: Arc::new(Mutex::new(Instant::now())),
            total_bytes_at_last_speed_calc: Arc::new(Mutex::new(0)),
        }
    }

    /// Gets a clone of the current progress
    pub async fn progress(&self) -> Progress {
        trace!("Getting clone of current progress");
        let progress = self.progress.lock().await.clone();
        trace!(
            downloaded = progress.downloaded,
            total = progress.total_size,
            "Retrieved progress clone"
        );
        progress
    }

    /// Gets a reference to the progress Arc
    pub fn progress_ref(&self) -> Arc<Mutex<Progress>> {
        trace!("Getting reference to progress Arc");
        self.progress.clone()
    }

    /// Updates the progress with the given number of bytes
    pub async fn update(&self, bytes_just_downloaded_by_this_call: u64) -> bool {
        let now = Instant::now();
        let mut should_notify_callback = false;

        trace!(
            bytes = bytes_just_downloaded_by_this_call,
            "Updating progress with new bytes"
        );

        // --- Update total downloaded bytes ---
        {
            let mut progress_guard = self.progress.lock().await;
            let prev_downloaded = progress_guard.downloaded;
            progress_guard.downloaded += bytes_just_downloaded_by_this_call;

            debug!(
                prev_downloaded,
                new_downloaded = progress_guard.downloaded,
                bytes_added = bytes_just_downloaded_by_this_call,
                total = progress_guard.total_size,
                percentage = progress_guard.percentage(),
                "Updated downloaded bytes"
            );

            if !progress_guard.is_complete && progress_guard.downloaded >= progress_guard.total_size
            {
                info!("Download size threshold reached, marking as complete");
                progress_guard.mark_complete();
            }
        }

        // --- Speed and ETA calculation (at interval) ---
        let mut last_calc_time_guard = self.last_speed_calc_time.lock().await;
        if now - *last_calc_time_guard >= self.update_interval {
            trace!("Update interval reached, recalculating speed and ETA");

            let mut progress_guard = self.progress.lock().await;
            let current_total_downloaded = progress_guard.downloaded;
            let mut total_bytes_at_last_calc_guard =
                self.total_bytes_at_last_speed_calc.lock().await;

            let bytes_downloaded_this_interval =
                current_total_downloaded.saturating_sub(*total_bytes_at_last_calc_guard);

            debug!(
                current_total = current_total_downloaded,
                last_total = *total_bytes_at_last_calc_guard,
                interval_bytes = bytes_downloaded_this_interval,
                "Calculated bytes downloaded in this interval"
            );

            let mut history_guard = self.speed_history.lock().await;
            history_guard.push((now, bytes_downloaded_this_interval));
            trace!(
                history_size = history_guard.len(),
                "Added entry to speed history"
            );

            // Prune history
            let cutoff_time = now
                .checked_sub(self.speed_window)
                .unwrap_or(Instant::now() - self.speed_window); // Handle potential underflow for old Instants if speed_window is large
            let before_prune = history_guard.len();
            history_guard.retain(|(t, _)| *t >= cutoff_time);
            trace!(
                before_prune,
                after_prune = history_guard.len(),
                removed = before_prune - history_guard.len(),
                "Pruned speed history"
            );

            // Calculate speed over the window
            let total_bytes_in_window: u64 = history_guard.iter().map(|(_, b)| *b).sum();
            let window_start_time = history_guard.first().map(|(t, _)| *t).unwrap_or(now);
            let duration_in_window_secs = now.duration_since(window_start_time).as_secs();

            let current_speed = if duration_in_window_secs > 0 {
                total_bytes_in_window / duration_in_window_secs
            } else {
                warn!("Duration in window is zero, setting speed to 0");
                0
            };

            debug!(
                total_bytes_in_window,
                duration_secs = duration_in_window_secs,
                speed = current_speed,
                "Calculated current download speed"
            );
            progress_guard.speed = current_speed;

            // Calculate ETA
            if progress_guard.is_complete {
                debug!("Download is complete, setting ETA to 0");
                progress_guard.eta = Some(0);
            } else if current_speed > 0 {
                let remaining_bytes = if progress_guard.total_size > current_total_downloaded {
                    progress_guard.total_size - current_total_downloaded
                } else {
                    0
                };
                let eta = remaining_bytes as f64 / current_speed as f64;
                debug!(
                    remaining_bytes,
                    speed = current_speed,
                    eta,
                    "Calculated ETA"
                );
                progress_guard.eta = Some(eta.round() as u64);
            } else {
                debug!("Speed is zero, cannot calculate ETA");
                progress_guard.eta = None;
            }

            // Update state for next calculation
            *last_calc_time_guard = now;
            *total_bytes_at_last_calc_guard = current_total_downloaded;

            should_notify_callback = true;
            debug!(
                should_notify = should_notify_callback,
                "Set callback notification flag"
            );
        }

        should_notify_callback
    }

    /// Marks the download as complete
    pub async fn mark_complete(&self) {
        trace!("Marking download as complete via ProgressTracker");
        let mut progress = self.progress.lock().await;
        progress.mark_complete();
    }

    /// Marks the download as paused
    pub async fn mark_paused(&self, paused: bool) {
        trace!(paused, "Marking download as paused via ProgressTracker");
        let mut progress = self.progress.lock().await;
        progress.mark_paused(paused);
    }

    /// Adds a completed chunk to the progress
    pub async fn add_completed_chunk(&self, start: u64, end: u64) {
        debug!(
            start,
            end,
            size = end - start + 1,
            "Adding completed chunk via ProgressTracker"
        );
        let mut progress = self.progress.lock().await;
        progress.add_completed_chunk(start, end);
    }
}
