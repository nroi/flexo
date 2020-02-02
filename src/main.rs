extern crate http;
extern crate rand;
extern crate flexo;

use std::io::prelude::*;
use std::io;
use http::Uri;
use flexo::*;
use crate::mirror_config::MirrorSelectionMethod;
use mirror_flexo::*;

mod mirror_config;
mod mirror_fetch;
mod mirror_cache;
mod mirror_flexo;

fn main() {
    let mirror_config = mirror_config::load_config();
    let providers: Vec<DownloadProvider> = if mirror_config.mirror_selection_method == MirrorSelectionMethod::Auto {
        match mirror_fetch::fetch_providers() {
            Ok(mirror_urls) => rate_providers(mirror_urls, &mirror_config),
            Err(e) => {
                println!("Unable to fetch mirrors remotely: {:?}", e);
                println!("Will try to fetch them from cache.");
                let mirrors = mirror_cache::fetch().unwrap();
                mirrors.iter().map(|url| {
                    DownloadProvider {
                        uri: url.parse::<Uri>().unwrap(),
                        mirror_results: MirrorResults::default(),
                        country: "unknown".to_owned(),
                    }
                }).collect()
            },
        }
    } else {
        let default_mirror_result: MirrorResults = Default::default();
        mirror_config.mirrors_predefined.into_iter().map(|uri| {
            DownloadProvider {
                uri: uri.parse::<Uri>().unwrap(),
                mirror_results: default_mirror_result,
                country: "Unknown".to_owned(),
            }
        }).collect()
    };
    println!("{:#?}", providers);

    let urls: Vec<String> = providers.iter().map(|x| x.uri.to_string()).collect();
    mirror_cache::store(&urls);

    let mut job_context: JobContext<DownloadJob> = JobContext::new(providers, mirror_config.mirrors_auto);

    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let filename: String = line.unwrap();
        let order = DownloadOrder {
            filepath: filename,
        };
        job_context.schedule(order);
    }
}
