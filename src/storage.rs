//! Storage module for the Dove downloader
//!
//! This module handles persistence of download state, management of temporary files,
//! and provides functionality for resuming downloads from interruptions.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::error::{DoveError, DoveResult};
use crate::progress::Progress;

/// Represents the state of a download that can be persisted
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadState {
    /// URL of the file being downloaded
    pub url: String,
    /// Destination path for the download
    pub destination: PathBuf,
    /// Current progress of the download
    pub progress: Progress,
    /// ETag from the server, if available
    pub etag: Option<String>,
    /// Last-Modified timestamp from the server, if available
    pub last_modified: Option<String>,
    /// Temporary file paths for each chunk
    pub temp_files: Vec<PathBuf>,
}

/// Manages the storage of download state and temporary files
#[derive(Debug, Clone)]
pub struct StorageManager {
    /// Base directory for temporary files
    base_temp_dir: PathBuf,
    /// Current download state
    state: Arc<Mutex<Option<DownloadState>>>,
}

impl StorageManager {
    /// Creates a new StorageManager
    pub fn new() -> DoveResult<Self> {
        // Use system temp directory as base
        let base_temp_dir = std::env::temp_dir().join("dove-downloader");

        trace!(
            path = %base_temp_dir.display(),
            "Creating storage manager with system temp directory",
        );
        fs::create_dir_all(&base_temp_dir).map_err(|e| {
            error!(
                error = %e,
                path = %base_temp_dir.display(),
                "Failed to create base temp directory",
            );
            DoveError::DirectoryCreationFailed {
                path: base_temp_dir.clone(),
                error: e,
            }
        })?;
        debug!(
            "Base temporary directory created: {}",
            base_temp_dir.display()
        );

        Ok(Self {
            base_temp_dir,
            state: Arc::new(Mutex::new(None)),
        })
    }

    /// Creates a new StorageManager with a custom temporary directory
    pub fn with_temp_dir<P: AsRef<Path>>(temp_dir: P) -> DoveResult<Self> {
        let temp_dir = temp_dir.as_ref().to_path_buf();

        trace!(
            path = %temp_dir.display(),
            "Creating storage manager with custom temp directory",
        );
        fs::create_dir_all(&temp_dir).map_err(|e| {
            error!(
                error = %e,
                path = %temp_dir.display(),
                "Failed to create custom temp directory",
            );
            DoveError::DirectoryCreationFailed {
                path: temp_dir.clone(),
                error: e,
            }
        })?;
        debug!(path = %temp_dir.display(), "Custom temporary directory created");

        Ok(Self {
            base_temp_dir: temp_dir,
            state: Arc::new(Mutex::new(None)),
        })
    }

    /// Initializes a new download
    pub async fn init_download(
        &self,
        url: &str,
        temporary_directory_id: &str,
        destination: &Path,
        total_size: u64,
        etag: Option<String>,
        last_modified: Option<String>,
    ) -> DoveResult<()> {
        trace!(
            url,
            destination = %destination.display(),
            total_size,
            "Initializing download",
        );
        debug!(etag, last_modified, "Download metadata");

        let progress = Progress::new(total_size);

        let state = DownloadState {
            url: url.to_string(),
            destination: destination.to_path_buf(),
            progress,
            etag,
            last_modified,
            temp_files: Vec::new(),
        };

        self.save_state(&state, temporary_directory_id).await?;

        self.set_state(state).await;

        info!("Download initialized successfully");
        Ok(())
    }

    /// Creates a temporary file for a chunk
    pub async fn create_chunk_file(
        &self,
        temporary_directory_id: &str,
        chunk_index: usize,
    ) -> DoveResult<PathBuf> {
        trace!(index = chunk_index, "Creating temporary file for chunk");

        let mut state_mutex = self.state.lock().await;
        let state = state_mutex.as_mut().ok_or_else(|| {
            error!("No active download when creating chunk file");
            DoveError::Other("No active download".to_string())
        })?;

        let file_name = format!("chunk_{}.part", chunk_index);
        let temp_path = self
            .base_temp_dir
            .join(temporary_directory_id)
            .join(file_name);
        debug!("Temporary file path: {}", temp_path.display());

        // Create the file
        File::create(&temp_path).map_err(|e| {
            error!(error = %e, path = %temp_path.display(), "Failed to create temporary file");
            DoveError::IoError(e)
        })?;
        debug!(temp_path = %temp_path.display(), "Temporary file created");

        // Add to state
        if chunk_index >= state.temp_files.len() {
            debug!(
                from = state.temp_files.len(),
                to = chunk_index + 1,
                "Resizing temp_files vector",
            );
            state.temp_files.resize(chunk_index + 1, PathBuf::new());
            debug!(length = state.temp_files.len(), "Resized temp_files vector");
        }

        state.temp_files[chunk_index] = temp_path.clone();
        debug!(
            length = state.temp_files.len(),
            "Added to temp_files vector"
        );

        // Save updated state
        self.save_state(state, temporary_directory_id).await?;

        Ok(temp_path)
    }

    /// Writes data to a chunk file at the specified offset
    pub fn write_chunk(&self, chunk_path: &Path, data: &[u8], offset: u64) -> DoveResult<()> {
        trace!(
            path = %chunk_path.display(),
            offset,
            data_size = data.len(),
            "Writing chunk data to temporary file",
        );

        // Use buffered writing for better performance
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true) // Ensure file is created if it doesn't exist
            .open(chunk_path)
            .map_err(|e| {
                error!(error = %e, path = %chunk_path.display(), "Failed to create or open temporary file");
                DoveError::IoError(e)
            })?;
        debug!(path = %chunk_path.display(), "Temporary file created");

        let mut buffered_writer = std::io::BufWriter::with_capacity(
            8192, // Use 8KB buffer size for better performance
            file,
        );
        trace!("Buffered writer created with capacity: 8192");

        // Set file length to match the maximum offset + data length if needed
        let file_len = std::fs::metadata(chunk_path).map(|m| m.len()).unwrap_or(0);
        trace!(length = file_len, "File length retrieved");

        let required_len = offset + data.len() as u64;

        if file_len < required_len {
            debug!(
                "File length {} is less than required length {}, extending file",
                file_len, required_len
            );
            buffered_writer
                .get_mut()
                .set_len(required_len)
                .map_err(|e| {
                    error!(error = %e, "Failed to set file length");
                    DoveError::IoError(e)
                })?;
            debug!(length = required_len, "File length set");
        }

        buffered_writer.seek(SeekFrom::Start(offset)).map_err(|e| {
            error!(error = %e, "Failed to seek in temporary file");
            DoveError::IoError(e)
        })?;
        debug!(offset = offset, "Seeked in temporary file");

        buffered_writer.write_all(data).map_err(|e| {
            error!(error = %e, "Failed to write to temporary file");
            DoveError::IoError(e)
        })?;
        debug!(bytes = data.len(), "Wrote chunk data to temporary file");

        // Ensure data is flushed to disk
        buffered_writer.flush().map_err(|e| {
            error!(error = %e, "Failed to flush buffered writer");
            DoveError::IoError(e)
        })?;
        debug!("Buffered writer flushed");

        Ok(())
    }

    /// Merges all chunk files into the final destination file
    pub async fn finalize_download(&self, temporary_directory_id: &str) -> DoveResult<()> {
        trace!("Merging chunk files into final destination");

        let state_mutex = self.state.lock().await;
        let state = state_mutex.as_ref().ok_or_else(|| {
            error!("No active download when finalizing");
            DoveError::Other("No active download".to_string())
        })?;

        debug!(state = ?state, "Download state retrieved");

        // Create parent directories if they don't exist
        if let Some(parent) = state.destination.parent() {
            debug!(path = %parent.display(), "Creating parent directories");
            fs::create_dir_all(parent).map_err(|e| {
                error!(path = %parent.display(), error = %e, "Failed to create parent directories");
                DoveError::DirectoryCreationFailed {
                    path: parent.to_path_buf(),
                    error: e,
                }
            })?;
            debug!(path = %parent.display(), "Parent directories created");
        }

        // Create or truncate the destination file
        let mut dest_file = File::create(&state.destination).map_err(|e| {
            error!(
                path = %state.destination.display(),
                error = %e,
                "Failed to create destination file",
            );
            DoveError::IoError(e)
        })?;
        debug!(path = %state.destination.display(), "Destination file created or truncated");

        // Copy each chunk to the destination file in order
        let mut buffer = vec![0u8; 8192]; // 8KB buffer
        debug!("Using buffer size of 8KB for merging chunks");

        // Get the chunks from the downloader state
        for (i, chunk_path) in state.temp_files.iter().enumerate() {
            if chunk_path.exists() {
                trace!(
                    "Processing chunk {}/{}: {}",
                    i + 1,
                    state.temp_files.len(),
                    chunk_path.display()
                );
                let mut chunk_file = File::open(chunk_path).map_err(|e| {
                    error!(path = %chunk_path.display(), error = %e, "Failed to open chunk file");
                    DoveError::IoError(e)
                })?;
                debug!(path = %chunk_path.display(), "Chunk file opened");

                // Get the file size to avoid reading uninitialized data
                let metadata = fs::metadata(chunk_path).map_err(|e| {
                    error!(
                        path = %chunk_path.display(),
                        error = %e,
                        "Failed to get metadata for chunk file"
                    );
                    DoveError::IoError(e)
                })?;
                let file_size = metadata.len();
                debug!(size = file_size, "Chunk file size retrieved");

                // Only read the actual data that was written
                let mut bytes_to_read = file_size;
                let mut total_bytes_read = 0u64;

                while bytes_to_read > 0 {
                    let read_size = std::cmp::min(bytes_to_read, buffer.len() as u64) as usize;
                    let bytes_read = chunk_file
                        .read(&mut buffer[0..read_size])
                        .map_err(|e| {
                            error!(path = %chunk_path.display(), error = %e, "Failed to read chunk file");
                            DoveError::IoError(e)
                        })?;
                    debug!(bytes_read = bytes_read, "Chunk file read");

                    if bytes_read == 0 {
                        warn!(
                            path = %chunk_path.display(),
                            "Unexpected end of file reached for chunk"
                        );
                        break;
                    }

                    dest_file
                        .write_all(&buffer[0..bytes_read])
                        .map_err(|e| {
                            error!(path = %state.destination.display(), error = %e, "Failed to write to destination file");
                            DoveError::IoError(e)})?;
                    debug!(
                        bytes_written = bytes_read,
                        path = %state.destination.display(),
                        "Data written to destination file"
                    );

                    bytes_to_read -= bytes_read as u64;
                    total_bytes_read += bytes_read as u64;
                }
                debug!(
                    "Completed writing chunk {}, total bytes: {}",
                    i, total_bytes_read
                );
            } else {
                warn!(path = %chunk_path.display(), "Chunk file does not exist");
            }
        }

        // Flush to ensure all data is written
        dest_file.flush().map_err(|e| {
            error!(path = %state.destination.display(), error = %e, "Failed to flush destination file");
            DoveError::IoError(e)})?;
        debug!(path = %state.destination.display(), "Destination file flushed");

        // Clean up temporary files
        self.cleanup(state, temporary_directory_id).await?;

        info!(
            destination = %state.destination.display(),
            "Download finalized successfully",
        );
        Ok(())
    }

    /// Cleans up temporary files
    pub async fn cleanup(
        &self,
        state: &DownloadState,
        temporary_directory_id: &str,
    ) -> DoveResult<()> {
        trace!("Cleaning up temporary files");

        for (i, chunk_path) in state.temp_files.iter().enumerate() {
            if chunk_path.exists() {
                debug!(
                    "Removing temporary file {}/{}: {}",
                    i + 1,
                    state.temp_files.len(),
                    chunk_path.display()
                );
                fs::remove_file(chunk_path).map_err(|e| {
                    error!(path = %chunk_path.display(), error = %e, "Failed to remove temporary file");
                    DoveError::IoError(e)
                })?;
                debug!(path = %chunk_path.display(), "Temporary file removed");
            } else {
                debug!(
                    path = %chunk_path.display(),
                    "Temporary file already removed or missing"
                );
            }
        }

        // Remove state file
        let state_path = Self::get_state_path(&self.get_state_directory(temporary_directory_id));
        if state_path.exists() {
            debug!("Removing state file: {}", state_path.display());
            fs::remove_file(&state_path).map_err(|e| {
                error!(path = %state_path.display(), error = %e, "Failed to remove state file");
                DoveError::IoError(e)
            })?;
            debug!(path = %state_path.display(), "State file removed");
        } else {
            debug!(
                "State file already removed or missing: {}",
                state_path.display()
            );
        }

        info!("Cleanup completed successfully");
        Ok(())
    }

    /// Saves the current download state to disk
    pub async fn save_state(
        &self,
        state: &DownloadState,
        temporary_directory_id: &str,
    ) -> DoveResult<()> {
        trace!("Saving download state for URL: {}", state.url);

        let temporary_directory = self.get_state_directory(temporary_directory_id);
        if !temporary_directory.exists() {
            debug!(
                "Creating temporary directory: {}",
                temporary_directory.display()
            );
            fs::create_dir_all(&temporary_directory).map_err(|e| {
                error!(
                    path = %temporary_directory.display(),
                    error = %e,
                    "Failed to create temporary directory",
                );
                DoveError::DirectoryCreationFailed {
                    path: temporary_directory.clone(),
                    error: e,
                }
            })?;
        }

        let state_path = Self::get_state_path(&temporary_directory);
        debug!("State file path: {}", state_path.display());

        let state_json = serde_json::to_string(state).map_err(|e| {
            error!(error = %e, "Failed to serialize download state");
            DoveError::progress_data_error("serialize", e.to_string())
        })?;
        debug!(state_json, "Download state serialized");

        fs::write(&state_path, state_json).map_err(|e| {
            error!(path = %state_path.display(), error = %e, "Failed to write state file");
            DoveError::IoError(e)
        })?;
        debug!(path = %state_path.display(), "State file written");

        Ok(())
    }

    /// Loads a download state from disk
    pub async fn load_state(
        &self,
        url: &str,
        temporary_directory_id: &str,
        destination: &Path,
    ) -> DoveResult<bool> {
        debug!(
            url,
            destination = %destination.display(),
            "Attempting to load download state",
        );
        let state_path = Self::get_state_path(&self.get_state_directory(temporary_directory_id));

        if !state_path.exists() {
            info!("State file not found, cannot resume download");
            return Ok(false);
        }

        debug!("State file found, loading");
        let state_json = fs::read_to_string(&state_path).map_err(|e| {
            error!(
                path = %state_path.display(),
                error = %e,
                "Failed to read state file",
            );
            DoveError::IoError(e)
        })?;

        let state: DownloadState = serde_json::from_str(&state_json).map_err(|e| {
            error!("Failed to deserialize download state: {}", e);
            DoveError::progress_data_error("deserialize", e.to_string())
        })?;
        debug!("State deserialized: {:?}", state);

        // Verify that this state matches the requested download
        if state.url != url || state.destination != destination {
            warn!(
                stored_url = state.url,
                requested_url = url,
                stored_destination = %state.destination.display(),
                requested_destination = %destination.display(),
                "State mismatch - URL or destination doesn't match"
            );
            return Ok(false);
        }

        // Verify that all temp files still exist
        let mut all_files_exist = true;
        for (i, chunk_path) in state.temp_files.iter().enumerate() {
            if !chunk_path.exists() {
                warn!("Chunk file {} missing: {}", i, chunk_path.display());
                all_files_exist = false;
                break;
            }
        }

        if !all_files_exist {
            warn!("Some chunk files are missing, cannot resume download");
            return Ok(false);
        }

        info!("Download state loaded successfully, can resume download");
        debug!(
            "Progress: {} / {} bytes",
            state.progress.downloaded(),
            state.progress.total()
        );
        self.set_state(state).await;

        Ok(true)
    }

    fn get_state_directory(&self, temporary_directory_id: &str) -> PathBuf {
        self.base_temp_dir.join(temporary_directory_id)
    }

    /// Gets the path to the state file
    fn get_state_path(temporary_directory: &Path) -> PathBuf {
        temporary_directory.join("download_state.json")
    }

    async fn set_state(&self, state: DownloadState) {
        debug!(
            "Setting download state: url={}, destination={}",
            state.url,
            state.destination.display()
        );
        let mut state_mutex = self.state.lock().await;
        *state_mutex = Some(state);
    }

    /// Checks if there's enough disk space for the download
    pub async fn check_disk_space(&self, required_bytes: u64) -> DoveResult<()> {
        debug!("Checking disk space, required: {} bytes", required_bytes);
        let state_mutex = self.state.lock().await;
        let state = state_mutex.as_ref().ok_or_else(|| {
            error!("No active download when checking disk space");
            DoveError::Other("No active download".to_string())
        })?;

        // Check temp directory space
        if let Ok(available) = fs2::available_space(&self.base_temp_dir) {
            debug!("Available space in temp directory: {} bytes", available);
            if available < required_bytes {
                warn!(
                    "Insufficient disk space in temp directory: needed={}, available={}",
                    required_bytes, available
                );
                return Err(DoveError::InsufficientDiskSpace {
                    needed: required_bytes,
                    available,
                });
            }
        } else {
            warn!("Could not determine available space in temp directory");
        }

        // Check destination directory space
        if let Some(parent) = state.destination.parent() {
            if let Ok(available) = fs2::available_space(parent) {
                debug!(
                    "Available space in destination directory: {} bytes",
                    available
                );
                if available < required_bytes {
                    warn!(
                        "Insufficient disk space in destination directory: needed={}, available={}",
                        required_bytes, available
                    );
                    return Err(DoveError::InsufficientDiskSpace {
                        needed: required_bytes,
                        available,
                    });
                }
            } else {
                warn!("Could not determine available space in destination directory");
            }
        } else {
            debug!("Destination has no parent directory, skipping destination space check");
        }

        info!("Disk space check passed");
        Ok(())
    }
}
