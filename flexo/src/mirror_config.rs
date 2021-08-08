static CONFIG_FILE: &str = "/etc/flexo/flexo.toml";

extern crate serde;

use std::fs;
use serde::Deserialize;
use flexo::Properties;
use std::time::Duration;
use regex::Regex;

static DEFAULT_JSON_URI: &str = "https://archlinux.org/mirrors/status/json/";

static DEFAULT_REFRESH_AFTER_SECONDS: u64 = 3600 * 24 * 14;

#[derive(Deserialize, Debug, Copy, Clone, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
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

#[derive(Deserialize, Debug, PartialEq, Eq, Copy, Clone)]
#[serde(rename_all = "lowercase")]
pub enum MirrorsRandomOrSort {
    Sort,
    Random,
}

#[derive(Deserialize, Debug, Clone)]
pub struct MirrorsAutoConfig {
    pub mirrors_status_json_endpoint: String,
    #[serde(default)]
    pub mirrors_status_json_endpoint_fallbacks: Vec<String>,
    #[serde(default)]
    pub mirrors_blacklist: Vec<String>,
    pub https_required: bool,
    pub ipv4: bool,
    pub ipv6: bool,
    pub max_score: f64,
    pub num_mirrors: usize,
    pub mirrors_random_or_sort: MirrorsRandomOrSort,
    pub timeout: u64,
    #[serde(default)]
    pub allowed_countries: Vec<String>,
}

impl MirrorsAutoConfig {
    pub fn relax(&self) -> Self {
        let mut relaxed = self.clone();
        relaxed.max_score += 3.0;
        relaxed.timeout += 100;
        relaxed
    }
}

impl Properties for MirrorConfig {}

#[derive(Deserialize, Debug, Clone)]
pub struct MirrorConfig {
    pub cache_directory: String,
    pub mirrorlist_fallback_file: String,
    pub mirrorlist_latency_test_results_file: Option<String>,
    pub refresh_latency_tests_after: Option<String>,
    pub port: u16,
    pub listen_ip_address: Option<String>,
    pub mirror_selection_method: MirrorSelectionMethod,
    pub mirrors_predefined: Vec<String>,
    pub custom_repo: Option<Vec<CustomRepo>>,
    low_speed_limit: Option<u32>,
    low_speed_limit_formatted: Option<String>,
    pub low_speed_time_secs: Option<u64>,
    pub connect_timeout: Option<u64>,
    pub max_speed_limit: Option<u64>,
    pub num_versions_retain: Option<u32>,
    pub mirrors_auto: Option<MirrorsAutoConfig>,
}

impl MirrorConfig {
    pub fn low_speed_limit(&self) -> Option<u32> {
        match &self.low_speed_limit_formatted {
            Some(limit) => {
                match parse_bandwidth(&limit) {
                    None => {
                        warn!("Unable to parse low_speed_limit_formatted <{}>.", limit);
                        None
                    }
                    Some(parsed_limit) => Some(parsed_limit)

                }
            },
            None => self.low_speed_limit
        }
    }
}

#[derive(Deserialize, Debug, Clone)]
pub struct CustomRepo {
    pub name: String,
    pub url: String,
}

impl MirrorConfig {
    pub fn refresh_latency_tests_after(&self) -> Duration {
        match &self.refresh_latency_tests_after {
            None => Duration::from_secs(DEFAULT_REFRESH_AFTER_SECONDS),
            Some(s) => match humantime::parse_duration(s) {
                Ok(d) => {
                    debug!("Latency tests will be refreshed after {:?}", &d);
                    d
                },
                Err(e) => {
                    error!("Unable to parse duration {:?}: {:?}", s, e);
                    Duration::from_secs(DEFAULT_REFRESH_AFTER_SECONDS)
                }
            }
        }
    }
}

fn mirror_config_from_toml() -> MirrorConfig {
    let config_contents = fs::read_to_string(CONFIG_FILE)
        .unwrap_or_else(|_| panic!("Unable to read file: {}", CONFIG_FILE));
    match toml::from_str(&config_contents) {
        Ok(v) => v,
        Err(e) => panic!("Unable to parse file {}: {:?}\nPlease make sure that the file contains \
        valid TOML syntax and that all required attributes are set.", CONFIG_FILE, e)
    }
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
    let mirrors_status_json_endpoint = parse_env_toml::<String>("FLEXO_MIRRORS_AUTO_MIRRORS_STATUS_JSON_ENDPOINT")
            .unwrap_or_else(|| DEFAULT_JSON_URI.to_owned());
    let mirrors_status_json_endpoint_fallbacks =
        parse_env_toml::<String>("FLEXO_MIRRORS_AUTO_MIRRORS_STATUS_JSON_ENDPOINT_FALLBACKS")
            .map(|fallbacks| comma_separated_to_vec(fallbacks))
            .unwrap_or_default();
    let allowed_countries = parse_env_toml::<String>("FLEXO_MIRRORS_AUTO_ALLOWED_COUNTRIES")
        .map(|country_list| comma_separated_to_vec(country_list))
        .unwrap_or_default();
    let mirrors_blacklist =
        parse_env_toml::<Vec<String>>("FLEXO_MIRRORS_AUTO_MIRRORS_BLACKLIST").unwrap_or_else(Vec::new);
    MirrorsAutoConfig {
        mirrors_status_json_endpoint,
        mirrors_status_json_endpoint_fallbacks,
        mirrors_blacklist,
        https_required,
        ipv4,
        ipv6,
        max_score,
        num_mirrors,
        mirrors_random_or_sort,
        timeout,
        allowed_countries,
    }
}

fn mirror_config_from_env() -> MirrorConfig {
    let cache_directory = parse_env_toml::<String>("FLEXO_CACHE_DIRECTORY").unwrap();
    let mirrorlist_fallback_file = parse_env_toml::<String>("FLEXO_MIRRORLIST_FALLBACK_FILE").unwrap();
    let mirrorlist_latency_test_results_file = parse_env_toml::<String>("FLEXO_MIRRORLIST_LATENCY_TEST_RESULTS_FILE");
    let listen_ip_address = parse_env_toml::<String>("FLEXO_LISTEN_IP_ADDRESS");
    let port = parse_env_toml::<u16>("FLEXO_PORT").unwrap();
    let mirror_selection_method = parse_env_toml::<MirrorSelectionMethod>("FLEXO_MIRROR_SELECTION_METHOD").unwrap();
    let mirrors_predefined = parse_env_toml::<Vec<String>>("FLEXO_MIRRORS_PREDEFINED").unwrap();
    let connect_timeout = parse_env_toml::<u64>("FLEXO_CONNECT_TIMEOUT");
    let low_speed_limit = parse_env_toml::<u32>("FLEXO_LOW_SPEED_LIMIT");
    let low_speed_limit_formatted = parse_env_toml::<String>("FLEXO_LOW_SPEED_LIMIT_FORMATTED");
    let low_speed_time_secs = parse_env_toml::<u64>("FLEXO_LOW_SPEED_TIME_SECS");
    let max_speed_limit = parse_env_toml::<u64>("FLEXO_MAX_SPEED_LIMIT");
    let refresh_latency_tests_after = parse_env_toml::<String>("FLEXO_REFRESH_LATENCY_TESTS_AFTER");
    let custom_repo_env = parse_env_toml::<String>("FLEXO_CUSTOM_REPO");
    let num_versions_retain = parse_env_toml::<u32>("FLEXO_NUM_VERSIONS_RETAIN");
    let custom_repo = custom_repos_from_env(custom_repo_env);

    let mirrors_auto = match mirror_selection_method {
        MirrorSelectionMethod::Auto => Some(mirrors_auto_config_from_env()),
        MirrorSelectionMethod::Predefined => None,
    };
    MirrorConfig {
        cache_directory,
        mirrorlist_fallback_file,
        mirrorlist_latency_test_results_file,
        refresh_latency_tests_after,
        port,
        listen_ip_address,
        mirror_selection_method,
        mirrors_predefined,
        custom_repo,
        low_speed_limit,
        low_speed_limit_formatted,
        low_speed_time_secs,
        connect_timeout,
        max_speed_limit,
        num_versions_retain,
        mirrors_auto,
    }
}

fn comma_separated_to_vec(comma_separated: String) -> Vec<String> {
    comma_separated
        .split(',')
        .into_iter()
        .filter(|s| !s.is_empty())
        .map(|s| s.trim().to_owned())
        .collect::<Vec<String>>()
}

fn custom_repos_from_env(maybe_env: Option<String>) -> Option<Vec<CustomRepo>> {
    match maybe_env {
        None => None,
        Some(cr) => {
            cr.split(' ').map(|s| {
                split_once(s, "@").map(|(name, url)| {
                    CustomRepo {
                        name: name.to_owned(),
                        url: url.to_owned(),
                    }
                })
            }).collect()
        }
    }
}

// FIXME replace with split_once from the stdlib once it is stable.
pub fn split_once<'a>(s: &'a str, delimiter: &'a str) -> Option<(&'a str, &'a str)> {
    let v = s.splitn(2, delimiter).collect::<Vec<&str>>();
    if v.len() == 2 {
        Some((v[0], v[1]))
    } else {
        None
    }
}

pub fn load_config() -> MirrorConfig {
    if std::env::vars().any(|(key, _value)| key.starts_with("FLEXO_")) {
        mirror_config_from_env()
    } else {
        mirror_config_from_toml()
    }
}

fn parse_bandwidth(s: &str) -> Option<u32> {
    let re = Regex::new(r"(?P<numeric_value>\d+) *(?P<si_unit>.*)/s").ok()?;
    let caps = re.captures(s)?;
    let numeric_value = caps["numeric_value"].parse::<u32>().ok()?;
    let si_unit = &caps["si_unit"].to_lowercase();

    parse_si_unit(numeric_value, &si_unit)
}

fn parse_si_unit(bandwidth_value: u32, si_unit: &str) -> Option<u32> {
    let (si_prefix, is_bits_not_bytes) = if si_unit.ends_with("bit") {
        (&si_unit[0..si_unit.len() - 3], true)
    } else if si_unit.ends_with('b') {
        (&si_unit[0..si_unit.len() - 1], false)
    } else {
        return None
    };
    let multiplier = match si_prefix {
        "" => 1,
        "ki" => 1024,
        "mi" => 1024 * 1024,
        "gi" => 1024 * 1024 * 1024,
        "k" => 1000,
        "m" => 1000 * 1000,
        "g" => 1000 * 1000 * 1000,
        _ => return None
    };

    let result = if is_bits_not_bytes {
        (bandwidth_value * multiplier) / 8
    } else {
        bandwidth_value * multiplier
    };

    Some(result)
}

#[test]
fn test_parse_bandwidth() {
    assert_eq!(Some(1), parse_bandwidth("8 Bit/s"));
    assert_eq!(Some(8), parse_bandwidth("8 B/s"));
    assert_eq!(Some(125), parse_bandwidth("1000 Bit/s"));

    assert_eq!(Some(125), parse_bandwidth("1 KBit/s"));
    assert_eq!(Some(25_600), parse_bandwidth("25 KiB/s"));
    assert_eq!(Some(125_000), parse_bandwidth("1000 KBit/s"));

    assert_eq!(Some(125_000), parse_bandwidth("1 MBit/s"));
    assert_eq!(Some(26_214_400), parse_bandwidth("25 MiB/s"));
    assert_eq!(Some(125_000_000), parse_bandwidth("1000 MBit/s"));

    assert_eq!(Some(125_000_000), parse_bandwidth("1 GBit/s"));
}
