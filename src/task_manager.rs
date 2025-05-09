use std::sync::Arc;

use futures::stream::{FuturesUnordered, StreamExt};
use tokio::sync::{mpsc, Mutex};

use crate::chunk::Chunk;
use crate::error::{DoveError, DoveResult};
use crate::http::HttpClient;
use crate::progress::{Progress, ProgressTracker};
use crate::storage::StorageManager;

/// Manages concurrent download tasks
pub struct TaskManager;

impl TaskManager {
    /// Executes download tasks for all chunks
    pub async fn execute_tasks<F>(
        url: String,
        chunks: Vec<Chunk>,
        http_client: Arc<HttpClient>,
        storage: Arc<StorageManager>,
        progress_tracker: Arc<ProgressTracker>,
        mut cancel_rx: mpsc::Receiver<()>,
        progress_callback: F,
    ) -> DoveResult<()>
    where
        F: FnMut(Progress) + Send + Copy + Sync + 'static,
    {
        debug!(url = url, chunks = chunks.len(), "Download URL");

        // Create a channel for completed chunks
        let (chunk_tx, mut chunk_rx) = mpsc::channel::<(usize, DoveResult<()>)>(chunks.len());
        debug!(
            "Created channel for completed chunks with capacity {}",
            chunks.len()
        );

        // Start download tasks
        let mut tasks = FuturesUnordered::new();

        let storage_mutex = Arc::new(Mutex::new(storage));
        debug!("Created storage mutex for concurrent access");

        for chunk in chunks.iter() {
            let chunk_tx = chunk_tx.clone();
            let url = url.clone();
            let processor = Arc::clone(&progress_tracker);
            let http_client = Arc::clone(&http_client);
            let storage_mutex = storage_mutex.clone();

            let mut chunk = chunk.clone();
            debug!(
                chunk_index = chunk.index,
                start = chunk.start,
                end = chunk.end,
                "Spawning download task for chunk"
            );

            let task = tokio::spawn(async move {
                debug!(chunk_index = chunk.index, "Starting download for chunk");
                let result = http_client
                    .download_with_retry(
                        &url,
                        &mut chunk,
                        storage_mutex,
                        Arc::clone(&processor),
                        progress_callback,
                    )
                    .await;

                // Send result back through channel
                match &result {
                    Ok(_) => debug!(
                        chunk_index = chunk.index,
                        "Chunk download completed successfully"
                    ),
                    Err(e) => warn!(error = ?e, chunk_index = chunk.index, "Chunk download failed"),
                }

                match chunk_tx.send((chunk.index, result)).await {
                    Ok(_) => trace!(chunk_index = chunk.index, "Result sent through channel"),
                    Err(e) => {
                        error!(error = ?e, chunk_index = chunk.index, "Failed to send result through channel")
                    }
                }
            });

            tasks.push(task);
        }

        // Process results and update progress
        let mut completed_chunks = 0;
        let total_chunks = tasks.len();
        info!(
            "All {} download tasks spawned, waiting for completion",
            total_chunks
        );

        loop {
            trace!(
                "Entering select loop, completed: {}/{}",
                completed_chunks,
                total_chunks
            );
            tokio::select! {
                // Check for cancellation
                _ = cancel_rx.recv() => {
                    info!("Received cancellation signal, stopping all tasks");

                    // Cancel all tasks
                    debug!("Adding dummy tasks to force FuturesUnordered completion");
                    for i in 0..tasks.len() {
                        tasks.push(tokio::spawn(async { }));
                        trace!("Added dummy task {}", i);
                    }

                    debug!("Draining remaining tasks");
                    let mut drained_count = 0;
                    while let Some(task) = tasks.next().await {
                        let _ = task;
                        drained_count += 1;
                        trace!("Drained task {}", drained_count);
                    }

                    info!("Download cancelled, all tasks drained");

                    return Err(DoveError::Cancelled);
                }

                // Process completed chunks
                Some((index, result)) = chunk_rx.recv() => {
                    debug!(chunk_index = index, "Received completion signal for chunk");
                    match result {
                        Ok(()) => {
                            completed_chunks += 1;
                            info!(
                                chunk_index = index,
                                completed = completed_chunks,
                                total = total_chunks,
                                "Chunk completed successfully, progress: {}/{}",
                                completed_chunks, total_chunks
                            );

                            // Check if all chunks are completed
                            if completed_chunks == total_chunks {
                                info!("All chunks downloaded successfully");
                                break;
                            }
                        }
                        Err(e) => {
                            error!(
                                error = ?e,
                                chunk_index = index,
                                "Chunk download failed with error"
                            );

                            return Err(e);
                        }
                    }
                }

                // Process task completion
                Some(task_result) = tasks.next() => {
                    debug!("Task completed, checking for errors");
                    // Just check for task errors
                    if let Err(e) = task_result {
                        error!(error = ?e, "Task panicked with error");
                        return Err(DoveError::Other(format!("Task error: {}", e)));
                    }
                    trace!("Task completed without panic");
                }

                // No more tasks or chunks
                else => {
                    debug!("No more tasks or chunks to process, breaking loop");
                    break;
                }
            }
        }

        info!("Task execution completed successfully");
        Ok(())
    }
}
