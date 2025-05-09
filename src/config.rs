//! Configuration module for the Dove downloader
//!
//! This module provides configuration options for the downloader,
//! including thread count, chunk size, timeout settings, and retry strategies.

use std::time::Duration;

/// Configuration for the download process
#[derive(Debug, Clone)]
pub struct DownloadConfig {
    /// Number of concurrent download threads
    pub threads: usize,
    /// Size of each download chunk in bytes
    pub chunk_size: usize,
    /// Total timeout for the download
    pub timeout: Duration,
    /// Connection timeout
    pub connection_timeout: Duration,
    /// Read timeout for download operations
    pub read_timeout: Duration,
    /// Number of retry attempts for failed chunks
    pub retry_count: usize,
    /// Delay between retry attempts
    pub retry_delay: Duration,
    /// Whether to follow redirects
    pub follow_redirects: bool,
    /// User agent string
    pub user_agent: String,
    /// Whether to verify SSL certificates
    pub verify_ssl: bool,
    /// Buffer size for writing to disk
    pub buffer_size: usize,
    /// Progress update interval
    pub progress_update_interval: Duration,
    /// Whether to overwrite existing files
    pub overwrite: bool,
}

impl Default for DownloadConfig {
    fn default() -> Self {
        Self {
            threads: 4,
            chunk_size: 1024 * 1024, // 1MB
            timeout: Duration::from_secs(60),
            connection_timeout: Duration::from_secs(30),
            read_timeout: Duration::from_secs(30),
            retry_count: 3,
            retry_delay: Duration::from_secs(5),
            follow_redirects: true,
            user_agent: format!("dove-downloader/{}", env!("CARGO_PKG_VERSION")),
            verify_ssl: true,
            buffer_size: 8192, // 8KB
            progress_update_interval: Duration::from_millis(100),
            overwrite: false,
        }
    }
}

impl DownloadConfig {
    /// Creates a new download configuration with default values
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the number of concurrent download threads
    pub fn with_threads(mut self, threads: usize) -> Self {
        self.threads = threads.max(1); // Ensure at least one thread
        self
    }

    /// Sets the size of each download chunk in bytes
    pub fn with_chunk_size(mut self, chunk_size: usize) -> Self {
        self.chunk_size = chunk_size.max(1024); // Ensure reasonable minimum chunk size
        self
    }

    /// Sets whether to overwrite existing files
    pub fn with_overwrite(mut self, overwrite: bool) -> Self {
        self.overwrite = overwrite;
        self
    }

    /// Sets the connection timeout
    pub fn with_connection_timeout(mut self, timeout: Duration) -> Self {
        self.connection_timeout = timeout;
        self
    }

    /// Sets the read timeout for download operations
    pub fn with_read_timeout(mut self, timeout: Duration) -> Self {
        self.read_timeout = timeout;
        self
    }

    /// Sets the number of retry attempts for failed chunks
    pub fn with_retry_count(mut self, count: usize) -> Self {
        self.retry_count = count;
        self
    }

    /// Sets the delay between retry attempts
    pub fn with_retry_delay(mut self, delay: Duration) -> Self {
        self.retry_delay = delay;
        self
    }

    /// Sets whether to follow redirects
    pub fn with_follow_redirects(mut self, follow: bool) -> Self {
        self.follow_redirects = follow;
        self
    }

    /// Sets the user agent string
    pub fn with_user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.user_agent = user_agent.into();
        self
    }

    /// Sets whether to verify SSL certificates
    pub fn with_verify_ssl(mut self, verify: bool) -> Self {
        self.verify_ssl = verify;
        self
    }

    /// Sets the buffer size for writing to disk
    pub fn with_buffer_size(mut self, size: usize) -> Self {
        self.buffer_size = size.max(1024); // Ensure reasonable minimum buffer size
        self
    }

    /// Sets the progress update interval
    pub fn with_progress_update_interval(mut self, interval: Duration) -> Self {
        self.progress_update_interval = interval;
        self
    }
}
