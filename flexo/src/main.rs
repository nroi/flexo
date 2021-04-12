extern crate flexo;
extern crate http;
#[macro_use] extern crate log;
extern crate rand;

use std::ffi::OsString;
use std::fs::File;
use std::io;
use std::io::ErrorKind;
use std::io::prelude::*;
use std::net::{TcpListener, TcpStream};
use std::os::unix::io::AsRawFd;
use std::path;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

use crossbeam::channel::Receiver;
use crossbeam::channel::RecvTimeoutError;
use libc::off64_t;
#[cfg(test)]
use tempfile::tempfile;

use flexo::*;
use mirror_flexo::*;

use crate::mirror_cache::{DemarshallError, TimestampedDownloadProviders};
use crate::mirror_config::{CustomRepo, MirrorConfig, MirrorSelectionMethod};
use crate::str_path::StrPath;

mod mirror_config;
mod mirror_fetch;
mod mirror_cache;
mod mirror_flexo;
mod str_path;

// man 2 read: read() (and similar system calls) will transfer at most 0x7ffff000 bytes.
#[cfg(not(test))]
const MAX_SENDFILE_COUNT: usize = 0x7fff_f000;

// Choose a smaller size in test, this makes it easier to have fast tests.
#[cfg(test)]
const MAX_SENDFILE_COUNT: usize = 128;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum PayloadOrigin {
    Cache,
    RemoteMirror,
    NoPayload,
}

fn main() {
    env_logger::builder().format_timestamp_millis().init();

    // Exit the entire process when a single thread panics:
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        hook(panic_info);
        std::process::exit(1);
    }));

    let properties = mirror_config::load_config();
    debug!("The following settings were fetched from the TOML file or environment variables: {:#?}", &properties);
    initialize_cache(&properties);
    match properties.low_speed_limit {
        None => {},
        Some(limit) => {
            info!("Will switch mirror if download speed falls below {}/s", size_to_human_readable(limit.into()));
        },
    }
    let job_context: Arc<Mutex<JobContext<DownloadJob>>> = match initialize_job_context(properties.clone()) {
        Ok(jc) =>  Arc::new(Mutex::new(jc)),
        Err(ProviderSelectionError::NoProviders) => {
            error!("Unable to find remote mirrors that match the selected criteria. Please \
            adapt your flexo.toml configuration file. See \
            https://github.com/nroi/flexo/blob/master/mirror_selection.md for more information.");
            std::process::exit(1);
        }
    };
    let port = job_context.lock().unwrap().properties.port;
    let listen_ip_address =
        job_context.lock().unwrap().properties.listen_ip_address.clone().unwrap_or("0.0.0.0".to_owned());
    debug!("Listen on address {}", listen_ip_address);
    let addr = format!("{}:{}", listen_ip_address, port);
    let listener = match TcpListener::bind(&addr) {
        Ok(l) => l,
        Err(e) => panic!("Unable to listen on address {}: {:?}", &addr, e),
    };
    // Synchronize file system access: We only want one cache purging process running at any given time.
    let cache_purge_mutex = Arc::new(Mutex::new(()));

    for client_stream in listener.incoming() {
        let client_stream: TcpStream = client_stream.unwrap();
        debug!("Established connection with client.");
        let job_context = job_context.clone();
        let properties = properties.clone();
        let num_versions_retain = properties.num_versions_retain;
        let cache_directory = properties.cache_directory.clone();
        debug!("All set, spawning new thread.");
        let cache_purge_mutex = cache_purge_mutex.clone();
        std::thread::spawn(move || {
            debug!("Started new thread.");
            let cache_tainted_result = serve_client(job_context, client_stream, properties);
            let _ = cache_purge_mutex.lock().unwrap();
            match (cache_tainted_result, num_versions_retain) {
                (Ok(true), Some(0)) => {},
                (Ok(true), Some(v)) => {
                    purge_cache(&cache_directory, v);
                },
                _ => {},
            }
        });
    }
}

fn purge_cache(directory: &str, num_versions_retain: u32) {
    debug!("Purging package cache");
    let flexo_purge_cache = "/usr/bin/flexo_purge_cache";
    let result = Command::new(flexo_purge_cache)
        .env("FLEXO_CACHE_DIRECTORY", directory)
        .env("FLEXO_NUM_VERSIONS_RETAIN", num_versions_retain.to_string())
        .output();
    match result {
        Ok(v) => {
            if let Some(s) = str_from_vec(v.stderr) {
                eprint!("{}", s);
            }
            if let Some(s) = str_from_vec(v.stdout) {
                print!("{}", s);
            }
            if v.status.success() {
                debug!("Package cache purged successfully");
            } else {
                warn!("Unable to purge package cache: {} has exited with failure (exit code {:?})",
                      flexo_purge_cache, v.status.code());
            }
        }
        Err(e) => {
            warn!("Unable to purge package cache: {:?}", &e);
        }
    }
}

fn str_from_vec(v: Vec<u8>) -> Option<String> {
    match String::from_utf8(v) {
        Ok(s) if !s.is_empty() => Some(s),
        Ok(_) => None,
        Err(_) => None,
    }
}

fn valid_path(path: &Path) -> bool {
    path.components().all(|c| {
        match c {
            path::Component::Normal(_) => true,
            path::Component::RootDir => true,
            _ => false,
        }
    })
}

fn serve_request(job_context: Arc<Mutex<JobContext<DownloadJob>>>,
                 client_stream: &mut TcpStream,
                 properties: MirrorConfig,
                 get_request: GetRequest,
) -> Result<PayloadOrigin, ClientError> {
    let (custom_provider, get_request) =
        custom_provider_from_request(get_request, &properties.custom_repo.unwrap_or(vec![]));
    if !valid_path(&get_request.path.as_ref())  {
        info!("Invalid path: Serve 403");
        serve_403_header(client_stream)?;
        Ok(PayloadOrigin::NoPayload)
    } else if get_request.path.to_str() == "status" {
        serve_200_ok_empty(client_stream)?;
        Ok(PayloadOrigin::NoPayload)
    } else {
        let order = DownloadOrder {
            filepath: get_request.path,
        };
        debug!("Attempt to schedule new job");
        let result = job_context.lock().unwrap().try_schedule(order.clone(), custom_provider, get_request.resume_from);
        match result {
            ScheduleOutcome::AlreadyInProgress => {
                debug!("Job is already in progress");
                let path = Path::new(&properties.cache_directory).join(&order.filepath.as_ref());
                let complete_filesize: u64 = try_complete_filesize_from_path(&path)?;
                let content_length = complete_filesize - get_request.resume_from.unwrap_or(0);
                let file: File = File::open(&path)?;
                serve_from_growing_file(file, content_length, get_request.resume_from, client_stream)?;
                Ok(PayloadOrigin::RemoteMirror)
            }
            ScheduleOutcome::Scheduled(ScheduledItem { rx_progress, .. }) => {
                // TODO this branch is also executed when the server returns 404.
                debug!("Job was scheduled, will serve from growing file");
                match receive_content_length(rx_progress) {
                    Ok(ContentLengthResult::ContentLength(content_length)) => {
                        debug!("Received content length via channel: {}", content_length);
                        let path = Path::new(&properties.cache_directory).join(&order.filepath);
                        let file: File = File::open(&path)?;
                        serve_from_growing_file(file, content_length, get_request.resume_from, client_stream)?;
                        Ok(PayloadOrigin::RemoteMirror)
                    },
                    Ok(ContentLengthResult::AlreadyCached) => {
                        debug!("File is already available in cache.");
                        let path = Path::new(&properties.cache_directory).join(&order.filepath);
                        let file: File = File::open(&path)?;
                        serve_from_complete_file(file, get_request.resume_from, client_stream)?;
                        Ok(PayloadOrigin::Cache)
                    },
                    Err(ContentLengthError::Unavailable) => {
                        debug!("Will send 404 reply to client.");
                        serve_404_header(client_stream)?;
                        Ok(PayloadOrigin::NoPayload)
                    },
                    Err(ContentLengthError::OrderError) => {
                        debug!("Will send 400 reply to client.");
                        serve_400_header(client_stream)?;
                        Ok(PayloadOrigin::NoPayload)
                    },
                    Err(ContentLengthError::TransmissionError(RecvTimeoutError::Disconnected)) => {
                        eprintln!("Remote server has disconnected unexpectedly.");
                        serve_500_header(client_stream)?;
                        Ok(PayloadOrigin::NoPayload)
                    },
                    Err(ContentLengthError::TransmissionError(RecvTimeoutError::Timeout)) => {
                        // TODO we should not immediately return 500, and instead try another mirror.
                        // TODO the problem is that the entire logic about retrying other mirrors is
                        // inside lib.rs
                        error!("Timeout: Unable to obtain content length.");
                        serve_500_header(client_stream)?;
                        Ok(PayloadOrigin::NoPayload)
                    },
                }
            },
            ScheduleOutcome::Cached => {
                debug!("Cache hit for request {:?}", &order.filepath);
                let path = Path::new(&properties.cache_directory).join(&order.filepath);
                let file: File = match File::open(&path) {
                    Ok(f) => f,
                    Err(e) => {
                        error!("Unable to open file {:?}: {:?}", &path, e);
                        return Err(ClientError::from(e));
                    }
                };
                serve_from_complete_file(file, get_request.resume_from, client_stream)?;
                Ok(PayloadOrigin::Cache)
            },
            ScheduleOutcome::Uncacheable(p) => {
                debug!("Serve file via redirect.");
                let uri_string = uri_from_components(&p.uri, order.filepath.to_str());
                serve_via_redirect(uri_string, client_stream)?;
                Ok(PayloadOrigin::NoPayload)
            }
        }
    }
}

fn serve_client(
    job_context: Arc<Mutex<JobContext<DownloadJob>>>,
    mut client_stream: TcpStream,
    properties: MirrorConfig
) -> Result<bool, ClientError> {
    let mut cache_tainted = false;
    // Loop for persistent connections: Will wait for subsequent requests instead of closing immediately.
    loop {
        debug!("Reading header from client.");
        match read_client_header(&mut client_stream) {
            Ok(ClientResponse::GetRequest(get_request)) => {
                let request_path = get_request.path.clone();
                match serve_request(job_context.clone(), &mut client_stream, properties.clone(), get_request) {
                    Ok(payload_origin) => {
                        let payload_origin_human_readable = match payload_origin {
                            PayloadOrigin::Cache => "CACHE HIT",
                            PayloadOrigin::RemoteMirror => {
                                // When the payload is downloaded from a remote mirror, a new file is stored in the
                                // cache.
                                cache_tainted = true;
                                "CACHE MISS"
                            },
                            PayloadOrigin::NoPayload => "NO PAYLOAD",
                        };
                        info!("Request served [{}]: {:?}", payload_origin_human_readable, &request_path.to_str())
                    },
                    Err(e) => {
                        error!("Unable to serve request {:?}: {:?}", &request_path.to_str(), e);
                        handle_client_error(&mut client_stream, e)?;
                        return Ok(cache_tainted)
                    },
                }
            }
            Ok(ClientResponse::SocketClosed) => {
                return Ok(cache_tainted);
            }
            Err(e) => {
                handle_client_error(&mut client_stream, e)?;
                return Ok(cache_tainted);
            }
        };
    }
}

/// Returns the custom provider, if a custom provider needs to be used, and the GetRequest. The GetRequest
/// is adapted to the returned custom provider, or returned unchanged if no custom provider needs to
/// be used.
fn custom_provider_from_request(get_request: GetRequest,
                                custom_repos: &Vec<CustomRepo>) -> (Option<DownloadProvider>, GetRequest) {
    match repo_name_from_get_request(&get_request) {
        None => (None, get_request),
        Some((repo_name, path)) => {
            let custom_repo = match custom_repos.iter().find(|r| r.name == repo_name) {
                None => {
                    warn!("A custom repo named {} is required to serve the GET request, \
                but no custom repo with that name was found.", repo_name);
                    return (None, get_request);
                }
                Some(r) => r
            };
            let provider = DownloadProvider {
                uri: custom_repo.url.clone(),
                name: custom_repo.name.clone(),
                mirror_results: Default::default(),
                country_code: "Unknown".to_string(),
            };
            let new_get_request = GetRequest {
                resume_from: get_request.resume_from,
                path,
            };
            (Some(provider), new_get_request)
        }
    }
}

/// Returns Ok if it is save to continue serving requests to this client, or Err otherwise.
fn handle_client_error(mut client_stream: &mut TcpStream, client_error: ClientError) -> Result<(), ClientError> {
    let result = match client_error {
        ClientError::Other(kind) if kind == ErrorKind::ConnectionReset => {
            debug!("Socket closed by client.");
            Err(client_error)
        }
        ClientError::TimedOut => {
            debug!("Connection client-to-server has timed out. New connection required \
                        for subsequent requests from the client.");
            Err(client_error)
        }
        ClientError::UnsupportedHttpMethod(ClientStatus { response_headers_sent }) => {
            error!("The client has used an HTTP method that is not supported by flexo.");
            if !response_headers_sent {
                serve_400_header(&mut client_stream)?;
            }
            Ok(())
        },
        ClientError::InvalidHeader(ClientStatus { response_headers_sent }) => {
            error!("The client has sent an invalid header");
            if !response_headers_sent {
                serve_400_header(&mut client_stream)?;
            }
            Ok(())
        }
        ClientError::IoError(error_kind) => {
            error!("Input/Output Error: {:?}", error_kind);
            Err(client_error)
        }
        _ => {
            eprintln!("Unable to read header: {:?}", &client_error);
            Err(client_error)
        }
    };
    match result {
        Err(ref e) => {
            warn!("Closing TCP socket due to error: {:?}", e);
            let _ = client_stream.shutdown(std::net::Shutdown::Both);
        },
        Ok(()) => {
            // nothing to do.
        }
    }
    result
}

fn repo_name_from_get_request(get_request: &GetRequest) -> Option<(String, StrPath)> {
    let mut component_iterator = get_request.path.as_ref().components();
    if component_iterator.next()?.as_os_str().to_str()? == "custom_repo" {
        let repo_name = component_iterator.next()?.as_os_str().to_str()?.to_owned();
        let path_without_repo_prefix = component_iterator.collect::<PathBuf>();
        let path = StrPath::from_path_buf(path_without_repo_prefix)?;
        info!("Request {:?} will be served via unofficial repository {:?}", get_request.path.to_str(), &repo_name);
        Some((repo_name, path))
    } else {
        None
    }
}

pub enum ProviderSelectionError {
    NoProviders,
}

fn initialize_job_context(properties: MirrorConfig) -> Result<JobContext<DownloadJob>, ProviderSelectionError> {
    let providers: Vec<DownloadProvider> = rated_providers(&properties);
    if providers.is_empty() {
        return Err(ProviderSelectionError::NoProviders)
    }
    info!("Primary mirror: {:#?}", providers[0].uri);
    let providers = mirror_cache::store_download_providers(&properties, providers);

    Ok(JobContext::new(providers, properties))
}

fn fetch_auto(mirror_config: &MirrorConfig) -> Vec<DownloadProvider> {
    let country_codes = mirror_config.mirrors_auto.as_ref()
        .map(|ma| ma.allowed_countries.clone())
        .flatten();
    let country_filter_uncached = match country_codes {
        None => CountryFilter::AllCountries,
        Some(v) if v.is_empty() => CountryFilter::AllCountries,
        Some(v) => CountryFilter::SelectedCountries(v),
    };
    match mirror_fetch::fetch_providers_from_json_endpoint(mirror_config) {
        Ok(mirror_urls) => {
            match mirror_cache::fetch_download_providers(mirror_config) {
                Ok(download_providers) => {
                    match latency_tests_refresh_required(mirror_config, &download_providers) {
                        true => {
                            info!("Continue to run latency test against all mirrors.");
                            rate_providers_uncached_retry(mirror_urls,
                                                          mirror_config.mirrors_auto.as_ref().unwrap().clone(),
                                                          &country_filter_uncached,
                                                          Limit::NoLimit)
                        },
                        false => {
                            info!("Continue to run latency test against a limited number of mirrors.");
                            rate_providers_cached(mirror_urls,
                                                  mirror_config,
                                                  download_providers.download_providers)
                        }
                    }
                },
                Err(e) => {
                    match e {
                        DemarshallError::IoError(e) if e.kind() == ErrorKind::NotFound => {
                            info!("No cached latency test results available. \
                            Continue to run latency tests on all mirrors.");
                        }
                        DemarshallError::VersionMismatch => {
                            info!("Latency test results are currently stored in an outdated \
                            format. This can happen if you have recently upgraded Flexo. Will \
                            continue to re-run latency tests and store them in the new format.");
                        }
                        DemarshallError::SerdeError(e) => {
                            info!("Unable to deserialize latency test results from file: {:?}. \
                            This can happen if you have recently upgraded Flexo. Will continue to \
                            re-run latency tests and store them in the new format.", e);
                        }
                        _ => {
                            error!("Unable to fetch latency test results from file: {:?}. \
                            Continue to run latency tests on all mirrors.", e);
                        }
                    };
                    rate_providers_uncached_retry(mirror_urls,
                                                  mirror_config.mirrors_auto.as_ref().unwrap().clone(),
                                                  &country_filter_uncached,
                                                  Limit::NoLimit)
                }
            }
        }
        Err(e) => {
            info!("Unable to fetch mirrors remotely: {:?}\nWill try to fetch them from cache.", e);
            match mirror_cache::fetch_download_providers(&mirror_config) {
                Ok(v) => v.download_providers,
                Err(e) => panic!("Unable to fetch mirrors from cache: {:?}", e),
            }
        },
    }
}

fn latency_tests_refresh_required(mirror_config: &MirrorConfig,
                                  download_providers: &TimestampedDownloadProviders) -> bool {
    let refresh_latency_tests_after = match chrono::Duration::from_std(mirror_config.refresh_latency_tests_after()) {
        Ok(d) => d,
        Err(e) => {
            error!("Unable to convert duration: {:?}", e);
            return true;
        }
    };
    let last_check = match chrono::DateTime::parse_from_rfc3339(&download_providers.timestamp) {
        Ok(dt) => dt.naive_utc(),
        Err(e) => {
            error!("Unable to convert timestamp {:?}: {:?}", &download_providers.timestamp, e);
            return true;
        }
    };
    info!("The most recent latency test ran at {}. Latency tests are scheduled to run against all mirrors after a \
    duration of: {:?}", last_check, refresh_latency_tests_after);
    let duration_since_last_check = chrono::Utc::now().naive_utc() - last_check;
    duration_since_last_check > refresh_latency_tests_after
}


fn rated_providers(mirror_config: &MirrorConfig) -> Vec<DownloadProvider> {
    if mirror_config.mirror_selection_method == MirrorSelectionMethod::Auto {
        let providers = fetch_auto(mirror_config);
        debug!("Mirror latency test results: {:#?}", providers);
        providers
    } else {
        let default_mirror_result: MirrorResults = Default::default();
        let mirrors_predefined = mirror_config.mirrors_predefined.clone();
        mirrors_predefined.into_iter().map(|uri| {
            DownloadProvider {
                uri: uri.clone(),
                name: uri,
                mirror_results: default_mirror_result,
                country_code: "Unknown".to_owned(),
            }
        }).collect()
    }
}

#[derive(Debug)]
enum ContentLengthError {
    TransmissionError(RecvTimeoutError),
    Unavailable,
    OrderError,
}

enum ContentLengthResult {
    ContentLength(u64),
    AlreadyCached,
}

fn receive_content_length(rx: Receiver<FlexoProgress>) -> Result<ContentLengthResult, ContentLengthError> {
    loop {
        match rx.recv_timeout(std::time::Duration::from_secs(6)) {
            Ok(FlexoProgress::JobSize(content_length)) => {
                break Ok(ContentLengthResult::ContentLength(content_length));
            }
            Ok(FlexoProgress::Completed) => {
                break Ok(ContentLengthResult::AlreadyCached);
            }
            Ok(FlexoProgress::Unavailable) => {
                break Err(ContentLengthError::Unavailable);
            }
            Ok(FlexoProgress::OrderError) => {
                break Err(ContentLengthError::OrderError);
            }
            Ok(msg) => {
                panic!("Unexpected message: {:?}", msg);
            },
            Err(e) => break Err(ContentLengthError::TransmissionError(e)),
        }
    }
}

/// Returns the size of the complete file. This size may be larger than the size we have stored locally.
fn try_complete_filesize_from_path(path: &Path) -> Result<u64, FileAttrError> {
    let mut num_attempts = 0;
    // Timeout after 2 seconds.
    while num_attempts < 2_000 * 2 {
        match content_length_from_path(path)? {
            None => {
                // for the unlikely event that this file has just been created, but the extended attribute
                // has not been set yet.
                std::thread::sleep(std::time::Duration::from_micros(500));
            },
            Some(v) => return Ok(v),
        }
        num_attempts += 1;
    }

    info!("Number of attempts exceeded: File {:?} not found.", &path);
    Err(FileAttrError::TimeoutError)
}

fn content_length_from_path(path: &Path) -> Result<Option<u64>, FileAttrError> {
    let key = OsString::from("user.content_length");
    let value = xattr::get(&path, &key)?;
    match value {
        Some(value) => {
            let content_length = String::from_utf8(value).map_err(FileAttrError::from)
                .and_then(|v| v.parse::<u64>().map_err(FileAttrError::from))?;
            debug!("Found file! content length is {}", content_length);
            Ok(Some(content_length))
        },
        None => {
            info!("file exists, but no content length is set.");
            Ok(None)
        }
    }
}

fn serve_from_growing_file(
    mut file: File,
    content_length: u64,
    resume_from: Option<u64>,
    client_stream: &mut TcpStream
) -> io::Result<()> {
    let header = match resume_from {
        None => reply_header_success(content_length, PayloadOrigin::RemoteMirror),
        Some(r) => reply_header_partial(content_length, r, PayloadOrigin::RemoteMirror)
    };
    client_stream.write_all(header.as_bytes())?;
    let resume_from = resume_from.unwrap_or(0);
    let mut client_received = resume_from;
    let complete_filesize = content_length + resume_from;
    while client_received < complete_filesize {
        let filesize = file.metadata()?.len();
        if filesize > client_received {
            // TODO note that this while loop runs indefinitely if the file stops growing for whatever reason.
            let result = send_payload_and_flush(&mut file, filesize, client_received as i64, client_stream);
            match result {
                Ok(size) => {
                    client_received = size as u64;
                },
                Err(e) => {
                    if e.kind() == ErrorKind::BrokenPipe || e.kind() == ErrorKind::ConnectionReset {
                        debug!("Broken Pipe or Connection reset. Connection closed by client?");
                    } else {
                        error!("Failed to send payload: An unexpected I/O error has occurred: {:?}", e);
                    }
                    return Err(e);
                },
            }
        }
        if client_received < content_length {
            std::thread::sleep(std::time::Duration::from_micros(500));
        }
    }
    debug!("File completely served from growing file.");
    Ok(())
}

fn serve_404_header(client_stream: &mut TcpStream) -> io::Result<()> {
    let header = reply_header_not_found();
    client_stream.write_all(header.as_bytes())
}

fn serve_400_header(client_stream: &mut TcpStream) -> io::Result<()> {
    let header = reply_header_bad_request();
    client_stream.write_all(header.as_bytes())
}

fn serve_500_header(client_stream: &mut TcpStream) -> io::Result<()> {
    let header = reply_header_internal_server_error();
    client_stream.write_all(header.as_bytes())
}

fn serve_403_header(client_stream: &mut TcpStream) -> io::Result<()> {
    let header = reply_header_forbidden();
    client_stream.write_all(header.as_bytes())
}

fn serve_200_ok_empty(client_stream: &mut TcpStream) -> io::Result<()> {
    let header = reply_header_success(0, PayloadOrigin::NoPayload);
    client_stream.write_all(header.as_bytes())
}

fn reply_header_success(content_length: u64, payload_origin: PayloadOrigin) -> String {
    reply_header("200 OK", content_length, None, payload_origin)
}

fn reply_header_partial(content_length: u64, resume_from: u64, payload_origin: PayloadOrigin) -> String {
    reply_header("206 Partial Content", content_length, Some(resume_from), payload_origin)
}

fn reply_header_not_found() -> String {
    reply_header("404 Not Found", 0, None, PayloadOrigin::NoPayload)
}

fn reply_header_bad_request() -> String {
    reply_header("400 Bad Request", 0, None, PayloadOrigin::NoPayload)
}

fn reply_header_internal_server_error() -> String {
    reply_header("500 Internal Server Error", 0, None, PayloadOrigin::NoPayload)
}

fn reply_header_forbidden() -> String {
    reply_header("403 Forbidden", 0, None, PayloadOrigin::NoPayload)
}

fn reply_header(status_line: &str,
                content_length: u64,
                resume_from: Option<u64>,
                payload_origin: PayloadOrigin) -> String {
    let now = time::now_utc();
    let timestamp = now.rfc822();
    let content_range_header = resume_from.map(|r| {
        let complete_size = content_length + r;
        let last_byte = complete_size - 1;
        format!("Content-Range: bytes {}-{}/{}\r\n", r, last_byte, complete_size)
    }).unwrap_or_else(|| "".to_owned());
    let header = format!("\
        HTTP/1.1 {}\r\n\
        Server: flexo\r\n\
        Date: {}\r\n\
        Flexo-Payload-Origin: {:?}\r\n\
        {}\
        Content-Length: {}\r\n\r\n",
                         status_line,
                         timestamp,
                         payload_origin,
                         content_range_header,
                         content_length
    );
    debug!("Sending header to client: {:?}", &header);

    header
}

fn redirect_header(path: &str) -> String {
    let now = time::now_utc();
    let timestamp = now.rfc822();
    let header = format!("\
        HTTP/1.1 301 Moved Permanently\r\n\
        Server: flexo\r\n\
        Date: {}\r\n\
        Content-Length: 0\r\n\
        Location: {}\r\n\r\n", timestamp, path);

    header
}

fn serve_from_complete_file(
    mut file: File,
    resume_from: Option<u64>,
    client_stream: &mut TcpStream
) -> io::Result<i64> {
    let filesize = file.metadata()?.len();
    let content_length = filesize - resume_from.unwrap_or(0);
    let header = match resume_from {
        None => reply_header_success(content_length, PayloadOrigin::Cache),
        Some(r) => reply_header_partial(content_length, r, PayloadOrigin::Cache)
    };
    client_stream.write_all(header.as_bytes())?;
    let bytes_sent = resume_from.unwrap_or(0) as i64;
    let result = send_payload_and_flush(&mut file, filesize, bytes_sent, client_stream);
    match &result {
        Ok(s) => debug!("{} bytes have been transmitted to the client.", s),
        Err(e) => warn!("Error while sending payload: {:?}", e),
    }
    result
}

fn serve_via_redirect(uri: String, client_stream: &mut TcpStream) -> io::Result<()> {
    debug!("Attempting to serve from {}", &uri);
    let header = redirect_header(&uri);
    client_stream.write_all(header.as_bytes())
}

fn send_payload_and_flush(
    mut source: &mut File,
    filesize: u64,
    bytes_sent: i64,
    receiver: &mut TcpStream
) -> io::Result<i64> {
    let result = send_payload(&mut source, filesize, bytes_sent, receiver);
    // Enabling and then disabling the nodelay option results in a flush.
    // For some reason, receiver.flush() does not have this effect.
    receiver.set_nodelay(true)?;
    receiver.set_nodelay(false)?;

    result
}

fn send_payload<T>(source: &mut File, filesize: u64, bytes_sent: i64, receiver: &mut T) -> io::Result<i64>
    where T: AsRawFd {
    let fd = source.as_raw_fd();
    let sfd = receiver.as_raw_fd();
    let size = unsafe {
        let mut offset = bytes_sent as off64_t;
        while (offset as u64) < filesize {
            let size: isize = libc::sendfile64(sfd, fd, &mut offset, MAX_SENDFILE_COUNT);
            if size == -1 {
                return Err(std::io::Error::last_os_error());
            }
        }
        offset
    };

    Ok(size)
}

#[test]
fn test_filesize_exceeds_sendfile_count() {
    let mut source: File = tempfile().unwrap();
    let mut receiver: File = tempfile().unwrap();
    let array: [u8; MAX_SENDFILE_COUNT * 3] = [b'a'; MAX_SENDFILE_COUNT * 3];
    source.write(&array).unwrap();
    source.flush().unwrap();
    let filesize = source.metadata().unwrap().len();
    let size = send_payload(&mut source, filesize, 0, &mut receiver).unwrap();
    assert_eq!(size, (MAX_SENDFILE_COUNT * 3) as i64);
}

#[test]
fn custom_provider_from_request_test() {
    let request = GetRequest {
        resume_from: None,
        path: StrPath::new("/custom_repo/archzfs/foo/bar/baz".to_owned()),
    };
    let custom_repo = CustomRepo {
        name: "archzfs".to_owned(),
        url: "https://archzfs.com".to_owned(),
    };
    let repos = vec![custom_repo];
    let (provider, new_get_request) = custom_provider_from_request(request, &repos);
    let expected_provider = DownloadProvider {
        uri: "https://archzfs.com".to_owned(),
        name: "archzfs".to_owned(),
        mirror_results: Default::default(),
        country_code: "Unknown".to_string()
    };
    let expected_get_request = GetRequest {
        resume_from: None,
        path: StrPath::new("/foo/bar/baz".to_owned()),
    };

    assert_eq!(provider, Some(expected_provider));
    assert_eq!(new_get_request, expected_get_request);
}

