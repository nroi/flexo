// Fallback strategy in case the JSON endpoint cannot be reached: The selected mirrors are stored in a text file
// so that we can simply retrieve and reuse the previously selected mirrors from this file, instead of fetching
// the mirrors from the JSON endpoint.

use crate::mirror_config::MirrorConfig;
use crate::mirror_flexo::DownloadProvider;

pub fn store(properties: &MirrorConfig, mirrors: &[String]) {
    // TODO reconsider if we still need this file: if we already store the result of our latency tests
    // in JSON format, we most likely won't need this file anymore.
    let data = mirrors.join("\n");
    std::fs::write(&properties.mirrorlist_fallback_file, data)
        .unwrap_or_else(|_| panic!("Unable to write file: {}", properties.mirrorlist_fallback_file));
}

pub fn store_download_providers(properties: &MirrorConfig, download_providers: Vec<DownloadProvider>) {
    let serialized = serde_json::to_string(&download_providers).unwrap();
    std::fs::write(&properties.mirrorlist_latency_test_results_file, serialized)
        .unwrap_or_else(|_| panic!("Unable to write file: {}", properties.mirrorlist_latency_test_results_file));
}

pub fn fetch(properties: &MirrorConfig) -> Result<Vec<String>, std::io::Error> {
    // TODO see the previous comment about using the JSON file instead of the plaintext file:
    // most likely, we won't need this function anymore.
    let contents = std::fs::read_to_string(&properties.mirrorlist_fallback_file)?;
    Ok(contents.split('\n').map(|s| s.to_owned()).collect())
}

pub fn fetch_download_providers(properties: &MirrorConfig) -> Result<Vec<DownloadProvider>, std::io::Error> {
    let contents = std::fs::read_to_string(&properties.mirrorlist_latency_test_results_file)?;
    // let download_providers: Vec<DownloadProvider> = serde_json::from_str(&contents)?;
    todo!("TODO")
}
