use std::path::Path;

use dove_core::{format_duration, format_size, format_speed, Dove, DoveResult, DownloadConfig};

#[tokio::main]
async fn main() -> DoveResult<()> {
    // Create a simple example to demonstrate how to use the dove-core library
    println!("Dove Core - Multi-threaded File Downloader Example");

    // Set download URL and destination path
    let url = "http://localhost:8080/1.mp4"; // A test download file

    let destination_dir = Path::new("../downloads");

    // Create download configuration
    let config = DownloadConfig::new()
        .with_threads(8)
        .with_chunk_size(1024 * 1024) // 1MB chunk size
        .with_retry_count(3) // Retry 3 times on failure
        .with_overwrite(true);

    println!("Start downloading: {}", url);
    println!("Save to: {}", destination_dir.display());

    // Create downloader instance
    let mut dove = Dove::new(config);

    let start = std::time::Instant::now();

    // Start downloading with progress callback
    dove.download(url, destination_dir, None, |progress| {
        println!(
            "Downloaded: {:.2}% ({}/{}), Speed: {}, Time remaining: {}",
            progress.percentage(),
            format_size(progress.downloaded()),
            format_size(progress.total()),
            format_speed(progress.speed()),
            format_duration(progress.eta().unwrap_or(0))
        );
    })
    .await?;

    let elapsed = start.elapsed();
    println!("Completed! Time elapsed: {:?}", elapsed);

    Ok(())
}
