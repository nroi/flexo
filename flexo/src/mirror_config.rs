static CONFIG_FILE: &str = "/etc/flexo/flexo.toml";

extern crate serde;

use std::fs;
use serde::Deserialize;
use flexo::Properties;

static DEFAULT_JSON_URI: &str = "https://www.archlinux.org/mirrors/status/json/";

#[serde(rename_all = "lowercase")]
#[derive(Deserialize, Debug, Copy, Clone, PartialEq, Eq)]
pub enum MirrorSelectionMethod {
    Auto,
    Predefined,
}

fn quote_str(s: String) -> String {
    format!("\"{}\"", s)
}

trait TomlValue {
    // the default implementation is fine for most cases. However, since TOML requires Strings to be quoted,
    // we need to provide an implementation in case the type of the TOML value is a String. Our own enums are also
    // represented by TOML strings, so we need an implementation for those types as well.
    fn toml_value_from_str(s: String) -> String {
        s
    }
}

impl TomlValue for bool { }
impl TomlValue for usize { }
impl TomlValue for f64 { }
impl TomlValue for u64 { }
impl TomlValue for u32 { }
impl TomlValue for u16 { }
impl TomlValue for Vec<String> { }
impl TomlValue for String {
    fn toml_value_from_str(s: String) -> String {
        quote_str(s)
    }
}

impl TomlValue for MirrorsRandomOrSort {
    fn toml_value_from_str(s: String) -> String {
        quote_str(s)
    }
}
impl TomlValue for MirrorSelectionMethod {
    fn toml_value_from_str(s: String) -> String {
        quote_str(s)
    }
}

#[serde(rename_all = "lowercase")]
#[derive(Deserialize, Debug, PartialEq, Eq, Copy, Clone)]
pub enum MirrorsRandomOrSort {
    Sort,
    Random,
}

#[derive(Deserialize, Debug, Clone)]
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
    pub mirrors_status_json_endpoint: String,
}

impl Properties for MirrorConfig {}

#[derive(Deserialize, Debug, Clone)]
pub struct MirrorConfig {
    pub cache_directory: String,
    pub mirrorlist_fallback_file: String,
    pub port: u16,
    pub mirror_selection_method: MirrorSelectionMethod,
    pub mirrors_predefined: Vec<String>,
    pub mirrors_blacklist: Vec<String>,
    pub mirrors_auto: MirrorsAutoConfig,
}

fn load_toml_config() -> MirrorConfig {
    let config_contents = fs::read_to_string(CONFIG_FILE)
        .unwrap_or_else(|_| panic!("Unable to read file: {}", CONFIG_FILE));
    toml::from_str(&config_contents).unwrap()
}

#[derive(Deserialize)]
struct DValue <T> {
    value: T
}

fn parse_env_toml<T>(s: &str) -> Option<T> where
          T: serde::de::DeserializeOwned + TomlValue + 'static,
{
    let env_var = std::env::var(s).ok()?;
    let toml_document = format!("value = {}", T::toml_value_from_str(env_var));
    // Our actual intent is to parse the environment variable as a TOML value, but the parser accepts only complete
    // TOML documents with key-value pairs. So we construct a TOML document with a single key-value pair, and
    // then extract the value.
    let deserialized = toml::from_str::<DValue<T>>(&toml_document).unwrap();
    Some(deserialized.value)
}

fn mirrors_auto_config_from_env() -> MirrorsAutoConfig {
    let https_required = parse_env_toml::<bool>("FLEXO_MIRRORS_AUTO_HTTPS_REQUIRED").unwrap();
    let ipv4 = parse_env_toml::<bool>("FLEXO_MIRRORS_AUTO_IPV4").unwrap();
    let ipv6 = parse_env_toml::<bool>("FLEXO_MIRRORS_AUTO_IPV6").unwrap();
    let max_score = parse_env_toml::<f64>("FLEXO_MIRRORS_AUTO_MAX_SCORE").unwrap();
    let num_mirrors = parse_env_toml::<usize>("FLEXO_MIRRORS_AUTO_NUM_MIRRORS").unwrap();
    let mirrors_random_or_sort = parse_env_toml::<MirrorsRandomOrSort>("FLEXO_MIRRORS_AUTO_MIRRORS_RANDOM_OR_SORT")
        .unwrap();
    let timeout = parse_env_toml::<u64>("FLEXO_MIRRORS_AUTO_TIMEOUT").unwrap();
    let low_speed_limit = parse_env_toml::<u32>("FLEXO_MIRRORS_AUTO_LOW_SPEED_LIMIT");
    let low_speed_time_secs = parse_env_toml::<u64>("FLEXO_MIRRORS_AUTO_LOW_SPEED_TIME_SECS");
    let max_speed_limit = parse_env_toml::<u64>("FLEXO_MIRRORS_AUTO_MAX_SPEED_LIMIT");
    let mirrors_status_json_endpoint = parse_env_toml::<String>("FLEXO_MIRRORS_AUTO_MIRRORS_STATUS_JSON_ENDPOINT")
            .unwrap_or_else(|| DEFAULT_JSON_URI.to_owned());
    MirrorsAutoConfig {
        https_required,
        ipv4,
        ipv6,
        max_score,
        num_mirrors,
        mirrors_random_or_sort,
        timeout,
        low_speed_limit,
        low_speed_time_secs,
        max_speed_limit,
        mirrors_status_json_endpoint,
    }
}

fn mirror_config_from_env() -> MirrorConfig {
    let mirrors_auto = mirrors_auto_config_from_env();
    let cache_directory = parse_env_toml::<String>("FLEXO_CACHE_DIRECTORY").unwrap();
    let mirrorlist_fallback_file = parse_env_toml::<String>("FLEXO_MIRRORLIST_FALLBACK_FILE").unwrap();
    let port = parse_env_toml::<u16>("FLEXO_PORT").unwrap();
    let mirror_selection_method = parse_env_toml::<MirrorSelectionMethod>("FLEXO_MIRROR_SELECTION_METHOD").unwrap();
    let mirrors_predefined = parse_env_toml::<Vec<String>>("FLEXO_MIRRORS_PREDEFINED").unwrap();
    let mirrors_blacklist = parse_env_toml::<Vec<String>>("FLEXO_MIRRORS_BLACKLIST").unwrap();
    MirrorConfig {
        cache_directory,
        mirrorlist_fallback_file,
        port,
        mirror_selection_method,
        mirrors_predefined,
        mirrors_blacklist,
        mirrors_auto
    }
}

pub fn load_config() -> MirrorConfig {
    if std::env::vars().any(|(key, _value)| key.starts_with("FLEXO_")) {
        mirror_config_from_env()
    } else {
        load_toml_config()
    }
}
