extern crate http;
extern crate rand;
extern crate flexo;

use std::io::prelude::*;
use std::io;
use std::sync::{Arc, Mutex};
use http::Uri;
use flexo::*;
use crate::mirror_config::MirrorSelectionMethod;
use mirror_flexo::*;

mod mirror_config;
mod mirror_fetch;
mod mirror_cache;
mod mirror_flexo;

use std::net::{TcpListener, Shutdown, TcpStream};
use std::thread;
use std::time::Duration;
use std::path::{Path, PathBuf};
use std::fs::File;

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
    let mut job_context: Arc<Mutex<JobContext<DownloadJob>>> = Arc::new(Mutex::new(job_context));

    let listener = TcpListener::bind("localhost:7878").unwrap();
    for stream in listener.incoming() {
        let mut stream: TcpStream = stream.unwrap();
        println!("connection established!");
        stream.set_read_timeout(Some(Duration::from_millis(500))).unwrap();

        let mut job_context = job_context.clone();
        let _t = thread::spawn(move || {
            match read_header(&mut stream) {
                Ok(get_request) => {
                    let path = Path::new(PATH_PREFIX).join(&get_request.path);
                    let order = DownloadOrder {
                        filepath: path.to_str().unwrap().to_owned()
                    };
                    let mut job_context = job_context.lock().unwrap();
                    match job_context.schedule(order, stream) {
                        ScheduleOutcome::Skipped(_) => {
                            todo!("what now?")
                        },
                        ScheduleOutcome::Scheduled(_) => {
                            todo!("download_from_growing_file")
                        },
                        ScheduleOutcome::Cached => {
                            todo!("sendfile_from_cache()")
                        }
                    }
                },
                Err(e) => {
                    println!("error: {:?}", e);
                },
            };
        });
    }
}

