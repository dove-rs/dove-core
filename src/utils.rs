//! Utility functions for the Dove downloader
//!
//! This module provides helper functions for URL parsing, HTTP client creation,
//! retry logic, and other utilities.

use std::time::Duration;

use blake3::Hasher;
use reqwest::header::{HeaderMap, HeaderValue, USER_AGENT};
use reqwest::Response;
use url::Url;

use crate::config::DownloadConfig;
use crate::error::{DoveError, DoveResult};

/// Creates an HTTP client with the specified configuration
pub(crate) fn create_client(config: &DownloadConfig) -> DoveResult<reqwest::Client> {
    let mut headers = HeaderMap::new();

    // Set user agent
    if let Ok(value) = HeaderValue::from_str(&config.user_agent) {
        headers.insert(USER_AGENT, value);
    }

    // Build client
    let client = reqwest::Client::builder()
        .default_headers(headers)
        .pool_idle_timeout(Duration::from_secs(30))
        .pool_max_idle_per_host(config.threads.max(20))
        .http1_only()
        .tcp_keepalive(Duration::from_secs(60))
        .danger_accept_invalid_certs(!config.verify_ssl)
        .redirect(if config.follow_redirects {
            reqwest::redirect::Policy::limited(10)
        } else {
            reqwest::redirect::Policy::none()
        })
        .build()
        .map_err(DoveError::HttpError)?;

    Ok(client)
}

/// Parses the Content-Length header from a response
pub(crate) fn parse_content_length(response: &Response) -> Option<u64> {
    response
        .headers()
        .get(reqwest::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
}

const SIZE_UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];

fn format_bytes_with_unit(bytes: u64) -> (f64, usize) {
    let mut size = bytes as f64;
    let mut unit_index = 0;

    while size >= 1024.0 && unit_index < SIZE_UNITS.len() - 1 {
        size /= 1024.0;
        unit_index += 1;
    }

    (size, unit_index)
}

/// Formats a size in bytes to a human-readable string
pub fn format_size(size: u64) -> String {
    let (size, unit_index) = format_bytes_with_unit(size);
    format!("{:.2} {}", size, SIZE_UNITS[unit_index])
}

/// Formats speed in bytes per second to a human-readable string
pub fn format_speed(speed: u64) -> String {
    let (speed, unit_index) = format_bytes_with_unit(speed);
    format!("{:.2} {}/s", speed, SIZE_UNITS[unit_index])
}

/// Formats a duration in seconds to a human-readable string
pub fn format_duration(seconds: u64) -> String {
    if seconds < 60 {
        return format!("{} sec", seconds);
    }

    let minutes = seconds / 60;
    let seconds = seconds % 60;

    if minutes < 60 {
        return format!("{} min {} sec", minutes, seconds);
    }

    let hours = minutes / 60;
    let minutes = minutes % 60;

    format!("{} hr {} min", hours, minutes)
}

/// Validates a URL
pub(super) fn validate_url(url: &str) -> DoveResult<url::Url> {
    match url::Url::parse(url) {
        Ok(url) => {
            if url.scheme() != "http" && url.scheme() != "https" {
                return Err(DoveError::InvalidUrl(format!(
                    "Unsupported scheme: {}",
                    url.scheme()
                )));
            }
            Ok(url)
        }
        Err(e) => Err(DoveError::InvalidUrl(e.to_string())),
    }
}

/// Extracts the filename from a URL
pub fn extract_filename(url: &Url) -> Option<String> {
    url.path_segments()?
        .next_back()?
        .split('?')
        .next()
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
}

/// Implements exponential backoff for retries
pub(crate) fn calculate_backoff_duration(retry: usize, base_delay: Duration) -> Duration {
    let backoff_factor = std::cmp::min(1 << retry, 32); // Cap at 32x to avoid overflow
    let backoff_secs = base_delay.as_secs() * backoff_factor as u64;

    // Add jitter (±10%)
    let jitter_factor = 0.9 + (rand::random::<f64>() * 0.2);
    let jitter_secs = (backoff_secs as f64 * jitter_factor) as u64;

    Duration::from_secs(jitter_secs)
}

pub fn get_temporary_directory(input: &[u8]) -> String {
    let mut hasher = Hasher::new();
    hasher.update(input);
    let hash = hasher.finalize();

    // Get the first 4 bytes (8 characters hex) as the prefix
    let prefix_bytes = &hash.as_bytes()[..4];
    let mut hex = String::new();
    for b in prefix_bytes {
        hex.push_str(&format!("{:02x}", b));
    }
    hex
}
