use std::sync::Arc;

use futures::stream::StreamExt;
use reqwest::header::{ETAG, LAST_MODIFIED, RANGE};
use reqwest::Client;
use tokio::sync::Mutex;
use url::Url;

use crate::chunk::Chunk;
use crate::config::DownloadConfig;
use crate::error::{DoveError, DoveResult};
use crate::progress::{Progress, ProgressTracker};
use crate::storage::StorageManager;
use crate::utils::{calculate_backoff_duration, create_client, parse_content_length};

/// Handles HTTP operations for downloading
pub struct HttpClient {
    client: Client,
    config: Arc<DownloadConfig>,
}

impl HttpClient {
    pub fn new(config: Arc<DownloadConfig>) -> DoveResult<Self> {
        debug!("Creating new HTTP client with config: {:?}", config);
        let client = create_client(config.as_ref())?;
        debug!("HTTP client created successfully");

        Ok(Self { client, config })
    }

    /// Sends a HEAD request to get file information
    pub async fn get_file_info(&self, url: &Url) -> DoveResult<FileInfo> {
        debug!(url = %url, "Sending HEAD request to get file information");

        let head_resp = self.client.head(url.clone()).send().await.map_err(|e| {
            error!(error = %e, url = %url, "Failed to send HEAD request");
            DoveError::HttpError(e)
        })?;

        debug!(status = %head_resp.status(), "HEAD response received");

        if !head_resp.status().is_success() {
            warn!(
                status = %head_resp.status(),
                url = %url,
                "HEAD request failed with non-success status"
            );
            return Err(DoveError::server_error(
                head_resp.status().as_u16(),
                head_resp.status().to_string(),
            ));
        }

        // Get content length
        let total_size = parse_content_length(&head_resp).ok_or_else(|| {
            warn!(url = %url, "Content-Length header missing in HEAD response");
            DoveError::FileSizeUnknown("Content-Length header missing".to_string())
        })?;
        debug!(
            size = total_size,
            "Content length parsed from HEAD response"
        );

        // Check if range requests are supported
        let supports_range = head_resp
            .headers()
            .get("accept-ranges")
            .and_then(|v| v.to_str().ok())
            .map(|v| v.contains("bytes"))
            .unwrap_or(false);

        debug!(
            supports_range = supports_range,
            "Range request support detected"
        );

        // Get ETag and Last-Modified for resuming
        let etag = head_resp
            .headers()
            .get(ETAG)
            .and_then(|v| v.to_str().ok())
            .map(String::from);

        let last_modified = head_resp
            .headers()
            .get(LAST_MODIFIED)
            .and_then(|v| v.to_str().ok())
            .map(String::from);

        debug!(
            ?etag,
            ?last_modified,
            "Retrieved ETag and Last-Modified headers"
        );

        info!(
            url = %url,
            size = total_size,
            supports_range = supports_range,
            "File information retrieved successfully"
        );

        Ok(FileInfo {
            total_size,
            supports_range,
            etag,
            last_modified,
        })
    }

    pub async fn download_with_retry<F>(
        &self,
        url: &str,
        chunk: &mut Chunk,
        storage: Arc<Mutex<Arc<StorageManager>>>,
        processor: Arc<ProgressTracker>,
        progress_callback: F,
    ) -> DoveResult<()>
    where
        F: FnMut(Progress) + Send + Copy + Sync + 'static,
    {
        let base_delay = self.config.retry_delay;
        debug!(
            url = url,
            chunk_index = chunk.index,
            retry_count = self.config.retry_count,
            retry_delay = ?base_delay,
            "Starting download with retry for chunk"
        );

        loop {
            debug!(
                url = url,
                chunk_index = chunk.index,
                retries = chunk.retries,
                "Attempting to download chunk"
            );

            match self
                .download_chunk(
                    url,
                    chunk,
                    storage.clone(),
                    processor.clone(),
                    progress_callback,
                )
                .await
            {
                Ok(_) => {
                    info!(
                        url = url,
                        chunk_index = chunk.index,
                        "Chunk downloaded successfully"
                    );
                    return Ok(());
                }
                Err(e) if e.is_retryable() && chunk.retries < self.config.retry_count => {
                    chunk.increment_retry();
                    let backoff = calculate_backoff_duration(chunk.retries, base_delay);
                    warn!(
                        error = ?e,
                        url = url,
                        chunk_index = chunk.index,
                        retries = chunk.retries,
                        backoff = ?backoff,
                        "Download failed with retryable error, will retry after backoff"
                    );
                    tokio::time::sleep(backoff).await;
                    continue;
                }
                Err(e) => {
                    error!(
                        error = ?e,
                        url = url,
                        chunk_index = chunk.index,
                        retries = chunk.retries,
                        "Download failed with non-retryable error or max retries exceeded"
                    );
                    return Err(e);
                }
            }
        }
    }

    /// Downloads a single chunk of a file
    pub async fn download_chunk<F>(
        &self,
        url: &str,
        chunk: &Chunk,
        storage: Arc<Mutex<Arc<StorageManager>>>,
        processor: Arc<ProgressTracker>,
        mut progress_callback: F,
    ) -> DoveResult<()>
    where
        F: FnMut(Progress) + Send + Copy + Sync + 'static,
    {
        trace!("Downloading chunk {}", chunk.index);

        // Create request with range header
        let mut req = self.client.get(url);

        // Add range header
        let range_header = format!("bytes={}-{}", chunk.start, chunk.end);
        req = req.header(RANGE, &range_header);
        debug!(
            url = url,
            range = range_header,
            chunk_index = chunk.index,
            "Sending range request"
        );

        // Send request
        let resp = req.send().await.map_err(|e| {
            error!(error = %e, chunk = chunk.display(), "Error sending request");
            DoveError::HttpError(e)
        })?;

        debug!(
            "Response received for chunk {}, status: {}",
            chunk.index,
            resp.status()
        );

        if !resp.status().is_success() {
            warn!(
                status = %resp.status(),
                url = url,
                chunk_index = chunk.index,
                "Received non-success status code for chunk download"
            );
            return Err(DoveError::server_error(
                resp.status().as_u16(),
                resp.status().to_string(),
            ));
        }

        // Download chunk data
        let mut stream = resp.bytes_stream();
        let mut offset = 0;
        let mut total_bytes_received = 0u64;

        debug!(
            chunk_index = chunk.index,
            temp_path = %chunk.temp_path.display(),
            "Starting to stream chunk data"
        );

        while let Some(chunk_result) = stream.next().await {
            let chunk_data = chunk_result.map_err(|e| {
                error!(
                    error = ?e,
                    chunk = chunk.display(),
                    "Error reading chunk data"
                );
                DoveError::HttpError(e)
            })?;
            let chunk_data_length = chunk_data.len() as u64;
            total_bytes_received += chunk_data_length;

            trace!(
                chunk_index = chunk.index,
                bytes = chunk_data_length,
                offset = offset,
                "Received data packet for chunk"
            );

            // Write chunk data to temporary file - use a shorter lock duration
            let storage_guard = storage.lock().await;
            storage_guard.write_chunk(&chunk.temp_path, &chunk_data, offset)?;
            drop(storage_guard);

            // Update offset and clear buffer
            offset += chunk_data_length;

            // Update progress without spawning a task
            if processor.update(chunk_data_length).await {
                let progress = processor.progress().await;
                trace!(
                    chunk_index = chunk.index,
                    progress = ?progress,
                    "Progress updated"
                );
                progress_callback(progress);
            }
        }

        debug!(
            chunk_index = chunk.index,
            total_bytes = total_bytes_received,
            "Chunk downloaded completely"
        );

        Ok(())
    }
}

/// Information about a file to be downloaded
pub struct FileInfo {
    pub total_size: u64,
    pub supports_range: bool,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}
