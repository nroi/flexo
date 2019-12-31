static CONFIG_FILE: &str = "/etc/cpcache/cpcache.toml";

extern crate serde;

use std::fs;
use serde::Deserialize;

#[serde(rename_all = "lowercase")]
#[derive(Deserialize, Debug, PartialEq, Eq)]
pub enum MirrorSelectionMethod {
    Auto,
    Predefined,
}

#[serde(rename_all = "lowercase")]
#[derive(Deserialize, Debug, PartialEq, Eq)]
pub enum MirrorsRandomOrSort {
    Sort,
    Random,
}

#[derive(Deserialize, Debug)]
pub struct MirrorsAutoConfig {
    pub https_required: bool,
    pub ipv4: bool,
    pub ipv6: bool,
    pub max_score: f64,
    pub num_mirrors: usize,
    pub mirrors_random_or_sort: MirrorsRandomOrSort,
    pub timeout: u64,
}

#[derive(Deserialize, Debug)]
pub struct MirrorConfig {
    pub mirror_selection_method: MirrorSelectionMethod,
    pub mirrors_predefined: Vec<String>,
    pub mirrors_blacklist: Vec<String>,
    pub mirrors_auto: MirrorsAutoConfig,
}

pub fn load_config() -> MirrorConfig {
    let config_contents = fs::read_to_string(CONFIG_FILE)
        .unwrap_or_else(|_| panic!("Unable to read file: {}", CONFIG_FILE));
    toml::from_str(&config_contents).unwrap()
}
