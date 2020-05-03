extern crate serde;
use serde::Deserialize;
use crate::mirror_config::MirrorConfig;
use curl::easy::{Easy, HttpVersion};
use std::time::Duration;
use crate::MirrorResults;

// integer values are easier to handle than float, since we don't have things like NaN. Hence, we just
// scale the float values from the JSON file in order to obtain integer values.
static SCORE_SCALE: u64 = 1_000_000_000_000_000;

#[derive(Deserialize, Debug)]
struct MirrorListOption {
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
#[derive(Deserialize, Debug, PartialEq, Eq)]
pub enum MirrorProtocol {
    Http,
    Https,
    Rsync,
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
    pub country: Option<String>,
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
        let country = self.country?;
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
            country,
            ipv4,
            ipv6
        })
    }
}

#[derive(Debug)]
pub struct MirrorUrl {
    pub url: String,
    pub protocol: MirrorProtocol,
    pub last_sync: String,
    pub completion_pct: f64,
    pub delay: i32,
    pub duration_avg: f64,
    pub duration_stddev: f64,
    pub score: u64,
    pub country: String,
    pub ipv4: bool,
    pub ipv6: bool,
}

impl MirrorUrl {
    pub fn filter_predicate(&self, config: &MirrorConfig) -> bool {
        !(
            (config.mirrors_auto.https_required && self.protocol != MirrorProtocol::Https) ||
                (config.mirrors_auto.ipv4 && !self.ipv4) ||
                (config.mirrors_auto.ipv6 && !self.ipv6) ||
                (config.mirrors_auto.max_score < (self.score as f64) / (SCORE_SCALE as f64)) ||
                (config.mirrors_blacklist.contains(&self.url)))
    }
}

fn fetch_json(mirror_config: &MirrorConfig) -> Result<String, curl::Error> {
    try_num_attempts(8, || {
        let mut received = Vec::new();
        let mut easy = Easy::new();
        easy.url(&mirror_config.mirrors_auto.mirrors_status_json_endpoint)?;
        {
            let mut transfer = easy.transfer();
            transfer.write_function(|data| {
                received.extend_from_slice(data);
                Ok(data.len())
            })?;
            transfer.perform()?
        }
        Ok(std::str::from_utf8(received.as_slice()).unwrap().to_owned())
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
                info!("Failure: {:?}. No internet connectivity yet? Will try again in a few seconds.", reason);
                std::thread::sleep(Duration::from_secs(3));
                result = action();
                num_attempts += 1;
            },
            Err(_) => break result
        }
    }
}

pub fn fetch_providers_from_json_endpoint(mirror_config: &MirrorConfig) -> Result<Vec<MirrorUrl>, curl::Error> {
    let json = fetch_json(mirror_config)?;
    let mirror_list_option: MirrorListOption = serde_json::from_str(&json).unwrap();
    let mirror_list: MirrorList = MirrorList::from(mirror_list_option);
    Ok(mirror_list.urls)
}

pub fn measure_latency(url: &str, timeout: Duration) -> Option<MirrorResults> {
    let mut easy = Easy::new();
    easy.url(url).unwrap();
    easy.follow_location(true).unwrap();
    easy.connect_only(true).unwrap();
    easy.dns_cache_timeout(Duration::from_secs(3600 * 24)).unwrap();
    easy.connect_timeout(timeout).unwrap();
    // we use httparse to parse the headers, but httparse doesn't support HTTP/2 yet. HTTP/2 shouldn't provide
    // any benefit for our use case (afaik), so this setting should not have any downsides.
    easy.http_version(HttpVersion::V11).unwrap();
    easy.transfer().perform().ok()?;
    Some(MirrorResults {
        namelookup_duration: easy.namelookup_time().ok()?,
        connect_duration: easy.connect_time().ok()?,
    })
}

