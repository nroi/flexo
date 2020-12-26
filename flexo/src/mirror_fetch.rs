extern crate serde;
use serde::Deserialize;
use crate::mirror_config::{MirrorConfig, MirrorsAutoConfig};
use curl::easy::{Easy, HttpVersion};
use std::time::Duration;
use std::str;
use crate::MirrorResults;
use crate::mirror_fetch::MirrorFetchError::{CurlError, DemarshallError, Utf8Error};

// If Flexo starts automatically with each system boot, it may happen that internet connectivity is not immediately
// available. For this reason, more than one attempt is made to connect to the server, hoping that the client
// will eventually have internet connectivity.
static INITIAL_CONNECTIVITY_NUM_ATTEMPTS: i32 = 8;

// For failed connection attempts that were made immediately after Flexo has started, we wait for this period of time
// before making a new attempt.
static INITIAL_CONNECTIVITY_DELAY_AFTER_FAILURE_SECONDS: u64 = 3;

// integer values are easier to handle than float, since we don't have things like NaN. Hence, we just
// scale the float values from the JSON file in order to obtain integer values.
static SCORE_SCALE: u64 = 1_000_000_000_000_000;

#[derive(Deserialize, Debug)]
pub struct MirrorListOption {
    pub urls: Vec<MirrorUrlOption>,
}

struct MirrorList {
    urls: Vec<MirrorUrl>,
}

impl From<MirrorListOption> for MirrorList {
    fn from(mirror_list_option: MirrorListOption) -> Self {
        let urls: Vec<Option<MirrorUrl>> = mirror_list_option.urls.into_iter().map(|mirror_url_option| {
            mirror_url_option.mirror_url()
        }).collect();
        let urls: Vec<MirrorUrl> = urls.into_iter().filter_map(|x| x).collect();
        MirrorList {
            urls
        }
    }
}

#[serde(rename_all = "lowercase")]
#[derive(Deserialize, Debug, PartialEq, Eq, Copy, Clone)]
pub enum MirrorProtocol {
    Http,
    Https,
    Rsync,
}

#[derive(Debug)]
pub enum MirrorFetchError {
    DemarshallError(serde_json::error::Error),
    CurlError(curl::Error),
    Utf8Error(str::Utf8Error),
}

impl From<curl::Error> for MirrorFetchError {
    fn from(error: curl::Error) -> Self {
        CurlError(error)
    }
}

impl From<serde_json::Error> for MirrorFetchError {
    fn from(error: serde_json::Error) -> Self {
        DemarshallError(error)
    }
}

impl From<str::Utf8Error> for MirrorFetchError {
    fn from(error: str::Utf8Error) -> Self {
        Utf8Error(error)
    }
}

#[derive(Deserialize, Debug)]
pub struct MirrorUrlOption {
    pub url: String,
    pub protocol: Option<MirrorProtocol>,
    pub last_sync: Option<String>,
    pub completion_pct: Option<f64>,
    pub delay: Option<i32>,
    pub duration_avg: Option<f64>,
    pub duration_stddev: Option<f64>,
    pub score: Option<f64>,
    #[serde(default)]
    pub country_code: Option<String>,
    pub ipv4: Option<bool>,
    pub ipv6: Option<bool>,
}

impl MirrorUrlOption {
    pub fn mirror_url(self) -> Option<MirrorUrl> {
        let protocol = self.protocol?;
        let last_sync = self.last_sync?;
        let completion_pct = self.completion_pct?;
        let delay = self.delay?;
        let duration_avg = self.duration_avg?;
        let duration_stddev = self.duration_stddev?;
        let score = (self.score? * SCORE_SCALE as f64) as u64;
        let country_code = self.country_code?;
        let ipv4 = self.ipv4?;
        let ipv6 = self.ipv6?;
        Some(MirrorUrl {
            url: self.url,
            protocol,
            last_sync,
            completion_pct,
            delay,
            duration_avg,
            duration_stddev,
            score,
            country_code,
            ipv4,
            ipv6
        })
    }
}

#[derive(Debug, Clone)]
pub struct MirrorUrl {
    pub url: String,
    pub protocol: MirrorProtocol,
    pub last_sync: String,
    pub completion_pct: f64,
    pub delay: i32,
    pub duration_avg: f64,
    pub duration_stddev: f64,
    pub score: u64,
    pub country_code: String,
    pub ipv4: bool,
    pub ipv6: bool,
}

impl MirrorUrl {
    pub fn filter_predicate(&self, mirrors_auto: &MirrorsAutoConfig) -> bool {
        !(
            (mirrors_auto.https_required && self.protocol != MirrorProtocol::Https) ||
                (mirrors_auto.ipv4 && !self.ipv4) ||
                (mirrors_auto.ipv6 && !self.ipv6) ||
                (mirrors_auto.max_score < (self.score as f64) / (SCORE_SCALE as f64)) ||
                (mirrors_auto.mirrors_blacklist.contains(&self.url)))
    }
}

fn fetch_json(mirror_config: &MirrorConfig) -> Result<String, MirrorFetchError> {
    let mirrors_auto = mirror_config.mirrors_auto.as_ref().unwrap();
    debug!("Fetch json from {:?}", &mirrors_auto.mirrors_status_json_endpoint);
    try_num_attempts(INITIAL_CONNECTIVITY_NUM_ATTEMPTS, || {
        let mut received = Vec::new();
        let mut easy = Easy::new();
        easy.follow_location(true).unwrap();
        easy.url(&mirrors_auto.mirrors_status_json_endpoint)?;
        {
            let mut transfer = easy.transfer();
            transfer.write_function(|data| {
                received.extend_from_slice(data);
                Ok(data.len())
            })?;
            transfer.perform()?
        }
        Ok(str::from_utf8(received.as_slice())?.to_owned())
    })
}

fn try_num_attempts<T, F, E>(max_num_attempts: i32, action: F) -> Result<T, E>
where F: Fn() -> Result<T, E>, E: std::fmt::Debug
{
    let mut result = action();
    let mut num_attempts = 1;
    loop {
        match result {
            Ok(_) => {
                break result;
            },
            Err(reason) if num_attempts < max_num_attempts => {
                info!("Failure: {:?}. No internet connectivity yet? Will try again in {} seconds.",
                      reason, INITIAL_CONNECTIVITY_DELAY_AFTER_FAILURE_SECONDS);
                std::thread::sleep(Duration::from_secs(INITIAL_CONNECTIVITY_DELAY_AFTER_FAILURE_SECONDS));
                result = action();
                num_attempts += 1;
            },
            Err(_) => break result
        }
    }
}

pub fn fetch_providers_from_json_endpoint(mirror_config: &MirrorConfig) -> Result<Vec<MirrorUrl>, MirrorFetchError> {
    let json = fetch_json(mirror_config)?;
    let mirror_list_option: MirrorListOption = serde_json::from_str(&json)?;
    let mirror_list: MirrorList = MirrorList::from(mirror_list_option);
    Ok(mirror_list.urls)
}

pub fn measure_latency(url: &str, timeout: Duration) -> Result<MirrorResults, curl::Error> {
    let mut easy = Easy::new();
    let url = url.to_owned() + "core/os/x86_64/core.db";
    easy.url(&url)?;
    easy.nobody(true)?;
    easy.follow_location(true)?;
    easy.dns_cache_timeout(Duration::from_secs(3600 * 24))?;
    easy.timeout(timeout)?;
    // we use httparse to parse the headers, but httparse doesn't support HTTP/2 yet. HTTP/2 shouldn't provide
    // any benefit for our use case (afaik), so this setting should not have any downsides.
    easy.http_version(HttpVersion::V11)?;
    easy.fail_on_error(true)?;
    easy.header_function(move |header: &[u8]| {
        // Exclude Cloudflare mirrors, because they mess up our latency results: Measuring the latency against a
        // Cloudflare protected server usually yields excellent results, but these results are meaningless since
        // Cloudflare uses caching: So the latency might have been low only because the request could be served
        // from the cache, but we can't assume that every request will be a cache hit.
        if header.to_ascii_lowercase().starts_with("server: cloudflare".as_bytes()) {
            debug!("Remote mirror {} appears to use CloudFlare, this mirror will be ignored.", &url);
            false
        } else {
            true
        }
    }).unwrap();
    easy.transfer().perform()?;
    Ok(MirrorResults {
        namelookup_duration: easy.namelookup_time()?,
        connect_duration: easy.connect_time()?,
        pretransfer_time: easy.pretransfer_time()?,
        total_time: easy.total_time()?,
        starttransfer_time: easy.starttransfer_time()?,
    })
}

