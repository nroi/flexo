// Fallback strategy in case the JSON endpoint cannot be reached: The selected mirrors are stored in a text file
// so that we can simply retrieve and reuse the previously selected mirrors from this file, instead of fetching
// the mirrors from the JSON endpoint.

use crate::mirror_config::MirrorConfig;

pub fn store(properties: &MirrorConfig, mirrors: &[String]) {
    let data = mirrors.join("\n");
    std::fs::write(&properties.mirrorlist_fallback_file, data)
        .unwrap_or_else(|_| panic!("Unable to write file: {}", properties.mirrorlist_fallback_file));
}

pub fn fetch(properties: &MirrorConfig) -> Result<Vec<String>, std::io::Error> {
    let contents = std::fs::read_to_string(&properties.mirrorlist_fallback_file)?;
    Ok(contents.split('\n').map(|s| s.to_owned()).collect())
}
