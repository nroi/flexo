// Fallback strategy in case the JSON endpoint cannot be reached: The selected mirrors are stored in a text file
// so that we can simply retrieve and reuse the previously selected mirrors from this file, instead of fetching
// the mirrors from the JSON endpoint.

use crate::mirror_config::MirrorConfig;
use crate::mirror_flexo::DownloadProvider;

use std::io;
use serde::{Serialize, Deserialize};

const DEFAULT_LATENCY_TEST_RESULTS_FILE: &str = "/var/cache/flexo/state/latency_test_results.json";

#[derive(Deserialize, Serialize)]
pub struct TimestampedDownloadProviders {
    pub timestamp: String,
    pub download_providers: Vec<DownloadProvider>,
}

fn latency_test_results_file(properties: &MirrorConfig) -> &str {
    match &properties.mirrorlist_latency_test_results_file {
        None => {
            warn!("The setting \"mirrorlist_latency_test_results_file\" is missing, will use default value {}. \
            Add the setting to the TOML config file or environment variable to avoid this warning.",
                  DEFAULT_LATENCY_TEST_RESULTS_FILE
            );
            DEFAULT_LATENCY_TEST_RESULTS_FILE
        }
        Some(p) => p,
    }
}

pub fn store_download_providers(properties: &MirrorConfig,
                                download_providers: Vec<DownloadProvider>) -> Vec<DownloadProvider> {
    let timestamped = TimestampedDownloadProviders {
        timestamp: format!("{}", chrono::Utc::now()),
        download_providers,
    };
    let serialized = serde_json::to_string(&timestamped).unwrap();
    let file_path = latency_test_results_file(properties);
    std::fs::write(file_path, serialized)
        .unwrap_or_else(|_| panic!("Unable to write file: {}", file_path));

    // Return the providers so that ownership is given back to the caller. This way, we can avoid
    // copying the providers.
    timestamped.download_providers
}

pub fn fetch(properties: &MirrorConfig) -> Result<Vec<String>, std::io::Error> {
    // TODO see the previous comment about using the JSON file instead of the plaintext file:
    // most likely, we won't need this function anymore.
    let contents = std::fs::read_to_string(&properties.mirrorlist_fallback_file)?;
    Ok(contents.split('\n').map(|s| s.to_owned()).collect())
}

pub fn fetch_download_providers(properties: &MirrorConfig) -> Result<TimestampedDownloadProviders, io::Error> {
    let file_path = latency_test_results_file(properties);
    let contents = std::fs::read_to_string(file_path)?;
    let download_providers: TimestampedDownloadProviders = serde_json::from_str(&contents)?;
    Ok(download_providers)
}
