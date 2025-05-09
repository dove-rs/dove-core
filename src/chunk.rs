use std::fmt::Display;
use std::path::PathBuf;
use std::sync::Arc;

use crate::error::DoveResult;
use crate::storage::StorageManager;

/// Represents a chunk of a file to be downloaded
#[derive(Debug, Clone)]
pub struct Chunk {
    /// Index of the chunk
    pub index: usize,
    /// Start byte of the chunk (inclusive)
    pub start: u64,
    /// End byte of the chunk (inclusive)
    pub end: u64,
    /// Path to the temporary file for this chunk
    pub temp_path: PathBuf,
    /// Number of retry attempts made for this chunk
    pub retries: usize,
}

impl Chunk {
    pub fn new(index: usize, start: u64, end: u64, temp_path: PathBuf) -> Self {
        Self {
            index,
            start,
            end,
            temp_path,
            retries: 0,
        }
    }

    pub fn display(&self) -> tracing::field::DisplayValue<&Chunk> {
        tracing::field::display(self)
    }

    pub fn increment_retry(&mut self) {
        self.retries += 1;
    }
}

impl Display for Chunk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Chunk {{ index: {}, start: {}, end: {}, temp_path: {} }}",
            self.index,
            self.start,
            self.end,
            self.temp_path.display()
        )
    }
}

/// Manages the creation and organization of download chunks
pub struct ChunkManager {
    storage: Arc<StorageManager>,
}

impl ChunkManager {
    pub fn new(storage: Arc<StorageManager>) -> Self {
        Self { storage }
    }

    /// Creates download chunks based on file size and thread count
    pub async fn create_chunks(
        &self,
        temporary_directory_id: &str,
        total_size: u64,
        thread_count: usize,
    ) -> DoveResult<Vec<Chunk>> {
        let mut chunks = Vec::new();

        // Calculate chunk size
        let chunk_size = if thread_count > 1 {
            total_size.div_ceil(thread_count as u64)
        } else {
            total_size
        };

        // Create chunks
        for i in 0..thread_count {
            let start = i as u64 * chunk_size;
            let end = std::cmp::min(start + chunk_size - 1, total_size - 1);

            // Skip empty chunks
            if start > end {
                continue;
            }

            // Create temporary file for this chunk
            let temp_path = self
                .storage
                .create_chunk_file(temporary_directory_id, i)
                .await?;

            chunks.push(Chunk::new(i, start, end, temp_path));
        }

        Ok(chunks)
    }
}
