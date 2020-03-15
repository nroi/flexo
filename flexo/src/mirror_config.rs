static CONFIG_FILE: &str = "/etc/flexo/flexo.toml";

extern crate serde;

use std::fs;
use serde::Deserialize;
use flexo::Properties;

#[serde(rename_all = "lowercase")]
#[derive(Deserialize, Debug, PartialEq, Eq)]
pub enum MirrorSelectionMethod {
    Auto,
    Predefined,
}

#[serde(rename_all = "lowercase")]
#[derive(Deserialize, Debug, PartialEq, Eq, Copy, Clone)]
pub enum MirrorsRandomOrSort {
    Sort,
    Random,
}

#[derive(Deserialize, Debug, Copy, Clone)]
pub struct MirrorsAutoConfig {
    pub https_required: bool,
    pub ipv4: bool,
    pub ipv6: bool,
    pub max_score: f64,
    pub num_mirrors: usize,
    pub mirrors_random_or_sort: MirrorsRandomOrSort,
    pub timeout: u64,
    pub low_speed_limit: Option<u32>,
    pub low_speed_time_secs: Option<u64>,
    pub max_speed_limit: Option<u64>,
}

impl Properties for MirrorsAutoConfig {}

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
