static CONFIG_FILE: &str = "/etc/cpcache/cpcache.toml";

extern crate serde;

use std::fs;
use serde::Deserialize;

#[serde(rename_all = "lowercase")]
#[derive(Deserialize, Debug)]
pub enum MirrorSelectionMethod {
    Auto,
    Predefined,
}

#[serde(rename_all = "lowercase")]
#[derive(Deserialize, Debug)]
pub enum MirrorsRandomOrSort {
    Sort,
    Random,
}

#[derive(Deserialize, Debug)]
pub struct MirrorsAutoConfig {
    https_required: bool,
    ipv4: bool,
    ipv6: bool,
    max_score: f64,
    num_mirrors: i32,
    mirrors_random_or_sort: MirrorsRandomOrSort,
    timeout: i32,
}

#[derive(Deserialize, Debug)]
pub struct MirrorConfig {
    mirror_selection_method: MirrorSelectionMethod,
    mirrors_predefined: Vec<String>,
    mirrors_blacklist: Vec<String>,
    mirrors_auto: MirrorsAutoConfig,
}

pub fn load_config() -> MirrorConfig {
    let config_contents = fs::read_to_string(CONFIG_FILE)
        .expect(&format!("Unable to read file: {}", CONFIG_FILE));
    toml::from_str(&config_contents).unwrap()
}
