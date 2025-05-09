# Dove Core

A multi-threaded file downloader library with resume capability and progress tracking, written in Rust.

## Features

- **Multi-threaded downloading** for improved performance
- **Resume capability** for interrupted downloads
- **Real-time progress tracking** with speed, ETA, and percentage completion
- **Robust error handling** with configurable retry mechanisms
- **Customizable configuration** for threads, chunk sizes, timeouts, and more
- **Simple and intuitive API** for easy integration

## Installation

Add this to your `Cargo.toml`:

```toml
[dependencies]
dove-core = "0.1.0"
```

## Basic Usage

```rust
use std::path::Path;

use dove_core::{Dove, DoveResult, DownloadConfig, format_size, format_speed};

#[tokio::main]
async fn main() -> DoveResult<()> {
    // Configure the downloader
    let config = DownloadConfig::new()
        .with_threads(4)                // Use 4 concurrent download threads
        .with_chunk_size(1024 * 1024)   // 1MB chunks
        .with_retry_count(3)            // Retry failed chunks 3 times
        .with_overwrite(true);          // Overwrite existing files

    // Create downloader instance
    let mut dove = Dove::new(config);

    // Start download with progress callback
    dove.download(
        "https://example.com/large-file.zip",  // URL to download
        Path::new("./downloads"),              // Destination directory
        None,                                  // Auto-detect filename from URL
        |progress| {
            println!(
                "Downloaded: {:.2}% ({}/{}), Speed: {}, ETA: {:.0} sec",
                progress.percentage(),
                format_size(progress.downloaded()),
                format_size(progress.total()),
                format_speed(progress.speed()),
                progress.eta().unwrap_or(0.0)
            );
        }
    ).await?;

    println!("Download completed!");
    Ok(())
}
```

## Advanced Features

### Pausing and Resuming Downloads

```rust
// Pause a download
let paused = dove.pause().await;

// Resume a paused download
let resumed = dove.resume().await;

// Cancel a download
let cancelled = dove.cancel();
```

### Custom Configuration

```rust
use std::time::Duration;

let config = DownloadConfig::new()
    .with_threads(8)                                    // 8 concurrent threads
    .with_chunk_size(2 * 1024 * 1024)                  // 2MB chunks
    .with_retry_count(5)                               // Retry 5 times
    .with_retry_delay(Duration::from_secs(10))         // 10 seconds between retries
    .with_connection_timeout(Duration::from_secs(30))  // 30 second connection timeout
    .with_read_timeout(Duration::from_secs(60))        // 60 second read timeout
    .with_follow_redirects(true)                       // Follow HTTP redirects
    .with_user_agent("MyApp/1.0")                     // Custom user agent
    .with_verify_ssl(true)                            // Verify SSL certificates
    .with_buffer_size(16384)                          // 16KB buffer for disk writes
    .with_progress_update_interval(Duration::from_millis(200)); // Progress updates every 200ms
```

## API Documentation

### `Dove`

The main entry point for the downloader library.

- `new(config: DownloadConfig) -> Dove` - Create a new downloader with custom configuration
- `default() -> Dove` - Create a new downloader with default configuration
- `download<P, F>(url: &str, destination_dir: P, filename: Option<String>, progress_callback: F) -> DoveResult<()>` - Download a file
- `pause() -> bool` - Pause the current download
- `resume() -> bool` - Resume a paused download
- `cancel() -> bool` - Cancel the current download

### `DownloadConfig`

Configuration options for the downloader.

- `new() -> DownloadConfig` - Create a new configuration with default values
- `with_threads(threads: usize) -> Self` - Set the number of concurrent download threads
- `with_chunk_size(chunk_size: usize) -> Self` - Set the size of each download chunk
- `with_overwrite(overwrite: bool) -> Self` - Set whether to overwrite existing files
- `with_connection_timeout(timeout: Duration) -> Self` - Set the connection timeout
- `with_read_timeout(timeout: Duration) -> Self` - Set the read timeout
- `with_retry_count(count: usize) -> Self` - Set the number of retry attempts
- `with_retry_delay(delay: Duration) -> Self` - Set the delay between retry attempts
- `with_follow_redirects(follow: bool) -> Self` - Set whether to follow redirects
- `with_user_agent(user_agent: impl Into<String>) -> Self` - Set the user agent string
- `with_verify_ssl(verify: bool) -> Self` - Set whether to verify SSL certificates
- `with_buffer_size(size: usize) -> Self` - Set the buffer size for writing to disk
- `with_progress_update_interval(interval: Duration) -> Self` - Set the progress update interval

### `Progress`

Represents the current progress of a download.

- `percentage() -> f64` - Get the percentage of the download that has been completed
- `downloaded() -> u64` - Get the number of bytes downloaded so far
- `total() -> u64` - Get the total size of the file in bytes
- `speed() -> u64` - Get the current download speed in bytes per second
- `eta() -> Option<f64>` - Get the estimated time remaining in seconds

### Utility Functions

- `format_size(size: u64) -> String` - Format a size in bytes to a human-readable string
- `format_speed(speed: u64) -> String` - Format speed in bytes per second to a human-readable string
- `format_duration(seconds: f64) -> String` - Format a duration in seconds to a human-readable string

## Error Handling

The library uses a custom error type `DoveError` that covers various failure scenarios:

- HTTP errors
- I/O errors
- Invalid URLs
- Server errors
- Cancellation/pausing
- Directory creation failures
- File size determination failures
- Range request support issues
- Checksum verification failures
- Timeouts
- Retry exhaustion
- Serialization/deserialization errors
- File existence conflicts
- Disk space issues

## License

MIT
