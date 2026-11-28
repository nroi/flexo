extern crate flexo;

use std::{fs, str};
use std::cmp::Ordering;
use std::fs::File;
use std::fs::OpenOptions;
use std::io::BufWriter;
use std::io::{ErrorKind, Read, Write};
use std::num::ParseIntError;
use std::path::{Path, PathBuf};
use std::string::FromUtf8Error;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::os::unix::ffi::OsStrExt;

use crossbeam::channel::Sender;
use curl::easy::{Easy2, Handler, HttpVersion, List, WriteError};
use httparse::{Header, Status};
use serde::{Deserialize, Serialize};
use tempfile::TempPath;
use walkdir::WalkDir;

use flexo::*;

use crate::mirror_config::{LowSpeedConfiguration, MirrorConfig, MirrorsAutoConfig};
use crate::{fs_utils, mirror_fetch};
use crate::mirror_fetch::{MirrorProtocol, Mirror};
use crate::str_path::StrPath;
use uuid::Uuid;
use crate::mirror_flexo::RequestMethod::{Get, Post};

// Since a restriction for the size of header fields is also implemented by web servers like NGINX or Apache,
// we keep things simple by just setting a fixed buffer length.
// TODO return 414 (Request-URI Too Large) if this size is exceeded, instead of just panicking.
const MAX_HEADER_SIZE: usize = 8192;

const MAX_HEADER_COUNT: usize = 64;

#[cfg(test)]
const TEST_CHUNK_SIZE: usize = 128;

#[cfg(test)]
const TEST_REQUEST_HEADER: &[u8] = "GET / HTTP/1.1\r\nHost: www.example.com\r\n\r\n".as_bytes();

const CURLE_OPERATION_TIMEDOUT: u32 = 28;

const MAX_REDIRECTIONS: u32 = 40;

const LATENCY_TEST_NUM_ATTEMPTS: u32 = 5;

pub const UNCACHEABLE_DIRECTORY: &str = "/tmp/flexo/uncacheable";

const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_millis(3000);
const DATABASE_RETRY_WINDOW: Duration = Duration::from_secs(4);

#[derive(Debug, PartialEq, Eq)]
pub enum ClientError {
    BufferSizeExceeded,
    TimedOut,
    IoError(std::io::ErrorKind),
    UnsupportedHttpMethod(ClientStatus),
    InvalidHeader(ClientStatus),
    Other(ErrorKind),
    FileAttrError(FileAttrError),
}

impl From<std::io::Error> for ClientError {
    fn from(error: std::io::Error) -> Self {
        ClientError::IoError(error.kind())
    }
}

impl From<FileAttrError> for ClientError {
    fn from(error: FileAttrError) -> Self {
        ClientError::FileAttrError(error)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct ClientStatus {
    pub response_headers_sent: bool
}

impl ClientStatus {
    pub fn no_response_headers_sent() -> Self {
        ClientStatus {
            response_headers_sent: false
        }
    }
}

#[derive(Debug, Clone)]
pub enum CountryFilter {
    AllCountries,
    SelectedCountries(Vec<String>),
}

#[derive(Copy, Clone)]
pub enum Limit {
    NoLimit,
    Limit(usize),
}

impl CountryFilter {
    fn includes_country(&self, country_code: &str) -> bool {
        match self {
            CountryFilter::AllCountries =>
                true,
            CountryFilter::SelectedCountries(country_codes) =>
                country_codes.iter().any(|c| c == country_code)
        }
    }
}

fn parse_range_header_value(s: &str) -> Result<u64, ClientError> {
    let s = s.to_lowercase();
    // We ignore everything after the - sign: We assume that pacman will never request only up to a certain size,
    // pacman will only skip the beginning of a file if the file has already been partially downloaded.
    match s.split('-').next() {
        None => {
            debug!("Unable to read the range header from the HTTP request.");
            Err(ClientError::InvalidHeader(ClientStatus::no_response_headers_sent()))
        }
        Some(range_start) => {
            match range_start.replace("bytes=", "").parse::<u64>() {
                Ok(v) => Ok(v),
                Err(_) => {
                    error!("Invalid range start submitted by client: {}", range_start);
                    Err(ClientError::InvalidHeader(ClientStatus::no_response_headers_sent()))
                }
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ClientResponse {
    Request(Request),
    SocketClosed,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Request {
    pub resume_from: Option<u64>,
    pub path: StrPath,
    pub method: RequestMethod,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum RequestMethod {
    Get,
    Post,
}

impl Request {
    fn new(request: httparse::Request) -> Result<Self, ClientError> {
        let range_header = request.headers
            .iter()
            .find(|h| h.name.eq_ignore_ascii_case("range"));
        let range_header_value = range_header.map(|h| {
            match str::from_utf8(h.value) {
                Ok(v) => Ok(v),
                Err(_) => {
                    error!("Unable to parse header value to UTF8");
                    Err(ClientError::InvalidHeader(ClientStatus::no_response_headers_sent()))
                }
            }
        });
        let resume_from = match range_header_value {
            None => None,
            Some(v) => {
                Some(parse_range_header_value(v?)?)
            }
        };
        let path = match request.path {
            None => {
                let client_status = ClientStatus { response_headers_sent: false };
                Err(ClientError::InvalidHeader(client_status))
            }
            Some(p) => Ok(p)
        }?;
        let request_method = match request.method {
            Some("GET") => Get,
            Some("POST") if path == "/reset-metrics" => Post,
            Some(method) => {
                error!("Unsupported HTTP method: {}", method);
                return Err(ClientError::UnsupportedHttpMethod(ClientStatus::no_response_headers_sent()));
            },
            None => {
                error!("Expected the request method to be set.");
                return Err(ClientError::InvalidHeader(ClientStatus::no_response_headers_sent()));
            },
        };
        let request_path = StrPath::new(path.to_owned());
        Ok(Self {
            path: request_path,
            method: request_method,
            resume_from,
        })
    }
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Hash, Clone, Debug)]
pub struct DownloadProvider {
    pub uri: String,
    pub name: String,
    // TODO both mirror_results and country_code should be Optional. Right now, we're storing garbage values
    // when the country is unknown and no results are available, which has already caused problems, see issue #58.
    pub mirror_results: MirrorResults,
    pub country_code: String,
}

impl Provider for DownloadProvider {
    type J = DownloadJob;

    fn new_job(&self, properties: &<<Self as Provider>::J as Job>::PR, order: DownloadOrder) -> DownloadJob {
        let uri = uri_from_components(&self.uri, order.requested_path.to_str());
        let provider = self.clone();
        let properties = properties.clone();
        DownloadJob {
            provider,
            uri,
            order,
            properties
        }
    }

    fn initial_score(&self) -> MirrorResults {
        self.mirror_results
    }

    fn identifier(&self) -> ProviderIdentifier {
        ProviderIdentifier {
            identifier: self.uri.clone()
        }
    }
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Hash, Copy, Clone, Debug, Default)]
pub struct MirrorResults {
    pub total_time: Duration,
    pub namelookup_duration: Duration,
    pub connect_duration: Duration,
    pub pretransfer_time: Duration,
    pub starttransfer_time: Duration,
}

impl Ord for MirrorResults {
    fn cmp(&self, other: &Self) -> Ordering {
        // namelookup_duration is excluded for performance comparisons, because DNS lookups are usually
        // cached, so we can assume that slow DNS lookups usually will not affect the latency experienced
        // by the user.
        let self_latency = self.total_time - self.namelookup_duration;
        let other_latency = other.total_time - other.namelookup_duration;
        self_latency.cmp(&other_latency)
    }
}

impl PartialOrd for MirrorResults {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug)]
pub enum DownloadJobError {
    CurlError(curl::Error),
    HttpFailureStatus(u32),
}

#[derive(Debug)]
pub struct DownloadJob {
    provider: DownloadProvider,
    uri: String,
    order: DownloadOrder,
    properties: MirrorConfig,
}

#[derive(Debug)]
pub enum OrderError {
    IoError(std::io::Error),
}

impl From<std::io::Error> for OrderError {
    fn from(error: std::io::Error) -> Self {
        OrderError::IoError(error)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum FileAttrError {
    Utf8Error(FromUtf8Error),
    ParseError(ParseIntError),
    IoError(std::io::ErrorKind),
    TimeoutError,
}

impl From<FromUtf8Error> for FileAttrError {
    fn from(error: FromUtf8Error) -> Self {
        FileAttrError::Utf8Error(error)
    }
}

impl From<ParseIntError> for FileAttrError {
    fn from(error: ParseIntError) -> Self {
        FileAttrError::ParseError(error)
    }
}

impl From<std::io::Error> for FileAttrError {
    fn from(error: std::io::Error) -> Self {
        FileAttrError::IoError(error.kind())
    }
}

impl Job for DownloadJob {
    type S = MirrorResults;
    type JS = DownloadJobResources;
    type C = DownloadChannel;
    type O = DownloadOrder;
    type P = DownloadProvider;
    type E = DownloadJobError;
    type PI = String;
    type PR = MirrorConfig;
    type OE = OrderError;
    type DSU = DynamicScoreUncacheableDownload;
    type DSC = DynamicScoreCacheableDownload;

    fn provider(&self) -> &DownloadProvider {
        &self.provider
    }

    fn order(&self) -> DownloadOrder {
        self.order.clone()
    }

    fn properties(&self) -> Self::PR {
        self.properties.clone()
    }

    // TODO find a better function name than "cache_state": This function does not only return something,
    // it also has side effects.
    fn cache_state(order: &Self::O, properties: &Self::PR) -> Option<CachedItem> {
        if order.requires_revalidation() {
            return None;
        }
        persist_and_get_cache_state(&order.filepath(properties))
    }

    fn serve_from_provider(
        self, mut channel: DownloadChannel,
        properties: &MirrorConfig,
    ) -> JobResult<DownloadJob> {
        debug!("Fetch package from remote mirror: {}.", &self.uri);
        channel.handle.url(&self.uri).unwrap();
        // we use httparse to parse the headers, but httparse doesn't support HTTP/2 yet. HTTP/2 shouldn't provide
        // any benefit for our use case (afaik), so this setting should not have any downsides.
        channel.handle.http_version(HttpVersion::V11).unwrap();
        let connect_timeout = match properties.connect_timeout {
            None => DEFAULT_CONNECT_TIMEOUT,
            Some(timeout) => Duration::from_millis(timeout),
        };
        channel.handle.connect_timeout(connect_timeout).unwrap();
        let low_speed_limit = match properties.effective_low_speed_limit() {
            LowSpeedConfiguration::ConfiguredByUser(limit) => limit,
            LowSpeedConfiguration::SensibleMinimumFallback(fallback) => {
                info!("No low_speed_limit was configured, using {} as fallback.", fallback);
                fallback
            }
        };
        channel.handle.low_speed_limit(low_speed_limit).unwrap();
        let low_speed_time_secs = match properties.effective_low_speed_time_secs() {
            LowSpeedConfiguration::ConfiguredByUser(low_speed_time_secs) => low_speed_time_secs,
            LowSpeedConfiguration::SensibleMinimumFallback(fallback) => {
                info!("No low_speed_time was configured, using {} as fallback.", fallback);
                fallback
            }
        };
        channel.handle.low_speed_time(Duration::from_secs(low_speed_time_secs)).unwrap();
        match properties.max_speed_limit {
            None => {
                debug!("No speed limit was set.")
            },
            Some(speed) => {
                info!("Apply speed limit of {}/s", size_to_human_readable(speed));
                channel.handle.max_recv_speed(speed).unwrap();
            },
        }
        channel.handle.follow_location(true).unwrap();
        channel.handle.max_redirections(MAX_REDIRECTIONS).unwrap();
        let if_modified_since = channel.if_modified_since();
        if let Some(last_modified) = if_modified_since {
            let mut headers = List::new();
            let header = format!("If-Modified-Since: {}", httpdate::fmt_http_date(last_modified));
            debug!("Send conditional request header to remote mirror: {}", header);
            headers.append(&header).unwrap();
            channel.handle.http_headers(headers).unwrap();
        }
        match channel.progress_indicator() {
            None => {
                debug!("No range request will be used, i.e., the entire file will be downloaded");
            },
            Some(start) => {
                info!("Resume download of {} from byte {}", &self.uri, start);
                channel.handle.resume_from(start).unwrap();
            }
        }
        debug!("Start download from {}", self.provider.identifier());
        match channel.handle.perform() {
            Ok(()) => {
                let response_code = channel.handle.response_code().unwrap();
                debug!("Download completed: {} replied with status code {}.",
                    self.provider.identifier(), response_code);
                if response_code == 200 && self.order.requires_revalidation() {
                    match channel.publish_database() {
                        Ok(size) => JobResult::Complete(JobCompleted::new(channel, self.provider, size as i64)),
                        Err(e) => {
                            error!("Unable to publish refreshed database {}: {:?}", self.order.description(), e);
                            JobResult::UnexpectedInternalError
                        }
                    }
                } else if (200..300).contains(&response_code) && !self.order.requires_revalidation() {
                    let size = channel.progress_indicator().unwrap_or(0);
                    JobResult::Complete(JobCompleted::new(channel, self.provider, size as i64))
                } else if response_code == 304 && if_modified_since.is_some() {
                    JobResult::Complete(JobCompleted::new(channel, self.provider, 0))
                } else if response_code == 404 {
                    JobResult::Unavailable(channel)
                } else {
                    let termination = JobTerminated {
                        channel,
                        error: DownloadJobError::HttpFailureStatus(response_code),
                    };
                    JobResult::Error(termination)
                }
            },
            Err(e) => {
                if e.code() == CURLE_OPERATION_TIMEDOUT {
                    warn!("Unable to download from {:?}: Timeout reached. Try another remote mirror.", &self.uri);
                } else {
                    warn!("An unknown error occurred while downloading from remote mirror {:?}: {:?}", &self.uri, e);
                }
                match channel.progress_indicator() {
                    Some(size) if size > 0 => {
                        JobResult::Partial(JobPartiallyCompleted::new(channel, size))
                    }
                    _ => {
                        let termination = JobTerminated {
                            channel,
                            error: DownloadJobError::CurlError(e),
                        };
                        JobResult::Error(termination)
                    }
                }
            }
        }
    }

    fn handle_error(self, error: OrderError) -> JobResult<Self> {
        match error {
            OrderError::IoError(e) if e.kind() == ErrorKind::NotFound => {
                // The client has specified a path that does not exist on the local file system. This can happen
                // if the required directory structure has not been created on the device running flexo, or if
                // the client has submitted an invalid request.
                JobResult::ClientError
            }
            e => {
                error!("Unexpected error: {:?}", e);
                JobResult::UnexpectedInternalError
            }
        }
    }

    fn acquire_resources(
        order: &DownloadOrder,
        properties: &MirrorConfig,
        last_chance: bool,
    ) -> std::io::Result<DownloadJobResources> {
        let path = order.filepath(&properties);
        debug!("Attempt to create file: {:?}", &path);
        let parent = path.parent().unwrap();
        fs_utils::create_dir_unless_exists(parent);
        let (file, temp_path, if_modified_since) = if order.requires_revalidation() {
            // A temporary file beside the destination can replace it atomically.
            let (file, temp_path) = tempfile::Builder::new()
                .prefix(".flexo-db-")
                .tempfile_in(parent)?
                .into_parts();
            (file, Some(temp_path), cached_database_modified_time(&path))
        } else {
            let file = OpenOptions::new().create(true).append(true).open(&path)?;
            (file, None, None)
        };
        let size_written = file.metadata()?.len();
        let buf_writer = BufWriter::new(file);
        let header_state = HeaderState {
            received_header: vec![],
            header_success: None,
            remote_last_modified: None,
        };
        let filename = path.file_name().unwrap().to_str().unwrap().to_owned();
        let file_state = FileState  {
            buf_writer,
            size_written,
            filename,
        };
        let download_job_resources = DownloadJobResources {
            file_state,
            header_state,
            last_chance,
            if_modified_since,
            temp_path,
        };
        Ok(download_job_resources)
    }
}

fn cached_database_modified_time(path: &Path) -> Option<SystemTime> {
    let metadata = path.metadata().ok()?;
    if !metadata.is_file() || metadata.len() == 0 {
        return None;
    }
    // Older Flexo builds could leave a partially downloaded database beside a size file.
    if let Some(expected_size) = get_complete_size_from_cfs_file(path) {
        if expected_size != metadata.len() {
            return None;
        }
    }
    // An epoch timestamp marks responses that supplied no usable Last-Modified value.
    metadata.modified().ok().filter(|modified| *modified > UNIX_EPOCH)
}

pub fn inspect_and_initialize_cache(mirror_config: &MirrorConfig) {
    let mut sum_size = 0;
    let mut count_cache_items = 0;
    fs_utils::create_dir_unless_exists(Path::new(&mirror_config.cache_directory));
    for entry in WalkDir::new(&mirror_config.cache_directory) {
        let entry = entry.expect("Error while reading directory entry");
        if entry.file_type().is_file() && !entry.file_name().as_bytes().starts_with(b".") {
            match persist_and_get_cache_state(entry.path()) {
                None => {},
                Some(CachedItem { cached_size, .. }) => {
                    sum_size += cached_size;
                    count_cache_items += 1;
                }
            }
        }
    }
    let size_formatted = size_to_human_readable(sum_size);
    info!("Retrieved {} files with a total size of {} from local file system.", count_cache_items, size_formatted);
}

fn persist_and_get_cache_state(path: &Path) -> Option<CachedItem> {
    debug!("Determine cache state for path {:?}", &path);
    let file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == ErrorKind::NotFound => {
            debug!("The file {:?} does not exist (yet)", &path);
            // The requested file is not cached, yet.
            return None;
        }
        Err(e) => {
            // TODO this can be made more resilient: For example, suppose that a single file in the cache directory
            // has the wrong read permissions set, so that flexo cannot access it. If the user attempts to download
            // multiple files, including this file, then we should return 500 only for this particular file, instead
            // of just crashing the entire thread.
            panic!("Unexpected I/O error occurred: {:?}", e);
        }
    };
    let file_size = file.metadata().expect("Unable to fetch file metadata").len();
    let complete_size = match get_complete_size_from_cfs_file(path) {
        None => {
            // Flexo maintains the complete file size (i.e., the expected file size when the
            // download has completed) in files with the "cfs" extension (cfs = complete file size).
            // If, for some reason, the complete file size could not be determined from the cfs
            // file, then just assuming that the current file size is already the complete file
            // size is usually a safe fallback. For example, this case can occur if files have been
            // copied to Flexo's package directory, or if the user removed the files with the cfs
            // extension.
            info!("Unable to fetch file size from CFS file for {:?}", path);
            match path.metadata().expect("Unable to fetch file metadata").len() {
                0 => {
                    info!("File {:?} is empty. Apparently, a previous download was aborted. This file will be removed",
                          path);
                    let _ = fs::remove_file(path);
                    return None;
                },
                s => {
                    info!("No CFS file exists for {:?} yet, continue to create it.", &path);
                    create_cfs_file(path, s);
                    Some(s)
                }
            }
        }
        Some(s) => Some(s)
    };
    Some(CachedItem {
        cached_size: file_size,
        complete_size,
    })
}

pub fn get_complete_size_from_cfs_file(path: &Path) -> Option<u64> {
    let cfs_path = cfs_path_from_pkg_path(&path)?;
    match fs::read_to_string(&cfs_path) {
        Ok(s) => {
            if s.ends_with('\n') && s.chars().rev().skip(1).all(|c| c.is_ascii_digit()) {
                let without_newline = &s[0..s.len()-1];
                match without_newline.parse::<u64>() {
                    Ok(size) => Some(size),
                    Err(e) => {
                        error!("Unable to parse {:?} into integer: {:?}", s, e);
                        None
                    }
                }
            } else {
                error!("File {:?} has unexpected format: Expected a single line containing digits only", cfs_path);
                None
            }
        }
        Err(e) if e.kind() == ErrorKind::NotFound => {
            debug!("CFS file {:?} does not exist", cfs_path);
            None
        }
        Err(e) => {
            error!("Unable to read file {:?} into string: {:?}", cfs_path, e);
            None
        }
    }
}

fn cfs_path_from_pkg_path(path: &Path) -> Option<PathBuf> {
    match path.file_name() {
        None => {
            warn!("Unable to determine file name from path {:?}", path);
            None
        }
        Some(f) => {
            let filename = f.to_str().expect("Expected all file names to be valid UTF-8");
            let cfs_filename = format!(".{}.cfs", filename);
            Some(path.with_file_name(cfs_filename))
        }
    }
}

#[derive(PartialEq, Eq, Hash, Clone, Debug)]
pub struct DownloadOrder {
    /// This path is relative to the given root directory.
    pub requested_path: StrPath,
    pub cacheability: Cacheability,
}

#[derive(PartialEq, Eq, Hash, Clone, Debug)]
pub enum Cacheability {
    NonCacheable(Uuid),
    Cacheable,
}

impl DownloadOrder {
    pub fn new(requested_path: StrPath) -> Self {
        Self {
            cacheability: if Self::is_database_signature(&requested_path) {
                Cacheability::NonCacheable(Uuid::new_v4())
            } else {
                Cacheability::Cacheable
            },
            requested_path,
        }
    }

    fn is_database_signature(path: &StrPath) -> bool {
        path.to_str().ends_with(".db.sig") || path.to_str().ends_with(".files.sig")
    }

    fn requires_revalidation(&self) -> bool {
        self.requested_path.to_str().ends_with(".db") ||
            self.requested_path.to_str().ends_with(".files")
    }
}

impl Order for DownloadOrder {
    type J = DownloadJob;

    fn new_channel(
        self,
        properties: MirrorConfig,
        tx: Sender<FlexoProgress>,
        last_chance: bool,
    ) -> Result<DownloadChannel, <Self::J as Job>::OE> {
        let download_state = DownloadState::new(self, properties, tx, last_chance)?;
        Ok(DownloadChannel {
            handle: Easy2::new(download_state)
        })
    }

    fn reuse_channel(
        self,
        properties: MirrorConfig,
        tx: Sender<FlexoProgress>,
        last_chance: bool,
        previous_channel: DownloadChannel,
    ) -> Result<DownloadChannel, <Self::J as Job>::OE> {
        let download_state = DownloadState::new(self, properties, tx, last_chance)?;
        let mut handle = previous_channel.handle;
        // Reset all libcurl options to their defaults before reusing this handle for a new
        // download. This is particularly important for HTTP range requests: if the previous
        // request was a range request, then we certainly don't want to re-use that setting
        // for the next request.
        handle.reset();
        handle.get_mut().replace(download_state);
        Ok(DownloadChannel {
            handle
        })
    }

    fn is_cacheable(&self) -> bool {
        match self.cacheability {
            Cacheability::NonCacheable(_) => false,
            Cacheability::Cacheable => true,
        }
    }

    fn use_cacheable_provider_selection(&self) -> bool {
        self.is_cacheable() && !self.requires_revalidation()
    }

    fn wait_for_completion(&self) -> bool {
        self.requires_revalidation()
    }

    fn retry_window(&self) -> Option<Duration> {
        self.requires_revalidation().then_some(DATABASE_RETRY_WINDOW)
    }

    fn retryable(&self) -> bool {
        // As of now (2021-06-05), pacman attempts to fetch files ending with .db.sig from the remote
        // mirrors, but the remote mirrors don't have those files available, because signatures for
        // database files still seems to be an open issue in Arch Linux. So pacman just silently ignores
        // the 404 responses returned by the remote mirrors. The default strategy for Flexo is to try all
        // mirrors one after another if a mirror returns 404. However, in this case, this is not a very useful
        // behavior because no mirror has those files, so Flexo would cause a small latency for the client
        // while it attempts to fetch the signature file from the various remote mirrors.
        // For this reason, we choose to not do any retries for all files ending with .db.sig.
        !Self::is_database_signature(&self.requested_path)
    }

    fn description(&self) -> &str {
        self.requested_path.to_str()
    }

}

impl DownloadOrder {
    pub fn filepath(&self, properties: &MirrorConfig) -> PathBuf {
        match self.cacheability {
            Cacheability::Cacheable => {
                Path::new(&properties.cache_directory).join(&self.requested_path)
            }
            Cacheability::NonCacheable(unique_id) => {
                let path = Path::join(Path::new(UNCACHEABLE_DIRECTORY), &self.requested_path);
                fs_utils::create_dir_unless_exists(path.parent().unwrap());
                let filename = format!("{}-{}", &self.requested_path.to_str(), unique_id);
                Path::new(UNCACHEABLE_DIRECTORY).join(filename)
            }
        }
    }
}

#[derive(Debug)]
pub struct FileState {
    buf_writer: BufWriter<File>,
    size_written: u64,
    filename: String,
}

#[derive(Debug)]
pub struct DownloadJobResources {
    file_state: FileState,
    header_state: HeaderState,
    last_chance: bool,
    if_modified_since: Option<SystemTime>,
    temp_path: Option<TempPath>,
}

#[derive(Debug)]
pub struct HeaderState {
    received_header: Vec<u8>,
    header_success: Option<HeaderOutcome>,
    remote_last_modified: Option<SystemTime>,
}

#[derive(Debug)]
enum HeaderOutcome {
    /// Header was read successfully and we're ready to write the payload to the local file system.
    Ok(u64),
    /// Server returned 304, so the locally cached file is still current.
    Cached,
    /// Server has returned 404.
    Unavailable,
}

#[derive(Debug)]
struct DownloadState {
    job_state: JobState<DownloadJob>,
    properties: MirrorConfig,
}

impl DownloadState {
    pub fn new(
        order: DownloadOrder,
        properties: MirrorConfig,
        tx: Sender<FlexoProgress>,
        last_chance: bool,
    ) -> std::io::Result<Self> {
        let download_job_resources = DownloadJob::acquire_resources(&order, &properties, last_chance)?;
        let job_state = JobState {
            order,
            job_resources: Some(download_job_resources),
            tx,
        };
        Ok(DownloadState { job_state, properties })
    }

    pub fn replace(&mut self, new_state: Self) {
        *self = new_state;
    }
}

impl Handler for DownloadState {
    fn write(&mut self, data: &[u8]) -> Result<usize, WriteError> {
        let revalidating = self.job_state.order.requires_revalidation();
        let job_resources = self.job_state.job_resources.as_mut().unwrap();
        match job_resources.header_state.header_success {
            Some(HeaderOutcome::Ok(_content_length)) => {},
            Some(HeaderOutcome::Cached) => {
                debug!("Cached file is up to date - return without writing anything.");
                return Ok(data.len());
            },
            Some(HeaderOutcome::Unavailable) => {
                // If the header says the file is not available, we return early without writing anything to
                // the file on disk. The content returned is just the HTML code saying the file is not available,
                // so there is no reason to write this data to disk.
                debug!("File unavailable - return content length without writing anything.");
                return Ok(data.len());
            },
            None => {
                unreachable!("The header should have been parsed before this function is called");
            }
        }
        if job_resources.file_state.size_written == 0 {
            debug!("Begin to transfer body to file {}", self.job_state.order.requested_path.to_str());
        }
        match job_resources.file_state.buf_writer.write_all(data) {
            Ok(()) => {
                job_resources.file_state.size_written += data.len() as u64;
                if !revalidating {
                    let len = job_resources.file_state.buf_writer.get_ref().metadata().unwrap().len();
                    let _ = self.job_state.tx.send(FlexoProgress::Progress(len));
                }
                Ok(data.len())
            },
            Err(e) => {
                error!("Error while writing data: {:?}", e);
                if revalidating { Ok(0) } else { Err(WriteError::Pause) }
            }
        }
    }

    fn header(&mut self, data: &[u8]) -> bool {
        let revalidating = self.job_state.order.requires_revalidation();
        let job_resources = self.job_state.job_resources.as_mut().unwrap();
        job_resources.header_state.received_header.extend(data);

        let mut headers: [Header; MAX_HEADER_COUNT] = [httparse::EMPTY_HEADER; MAX_HEADER_COUNT];
        let mut req: httparse::Response = httparse::Response::new(&mut headers);
        let result: std::result::Result<httparse::Status<usize>, httparse::Error> =
            req.parse(job_resources.header_state.received_header.as_slice());

        match result {
            Ok(Status::Complete(_header_size)) => {
                debug!("Received complete header from remote mirror");
                let code = req.code.unwrap();
                debug!("HTTP response code for {} is {}", &self.job_state.order.requested_path.to_str(), code);
                if revalidating && (code == 206 || code == 416) {
                    warn!("Unexpected range response to a full database request: {}", code);
                    return false;
                }
                if code == 200 || code == 206 {
                    let maybe_content_length = req.headers.iter().find_map(|header|
                        if header.name.eq_ignore_ascii_case("content-length") {
                            str::from_utf8(header.value).ok().and_then(|h| h.parse::<u64>().ok())
                        } else {
                            None
                        }
                    );
                    let content_length = match maybe_content_length {
                        None => {
                            warn!("Remote mirror did not send a valid Content-Length header.");
                            return false
                        }
                        Some(cl) => cl
                    };
                    debug!("Server replied with content length {} for {}",
                        content_length, self.job_state.order.requested_path.to_str());
                    job_resources.header_state.header_success = Some(HeaderOutcome::Ok(content_length));
                    if revalidating {
                        if content_length == 0 {
                            warn!("Remote mirror returned an empty database.");
                            return false;
                        }
                        job_resources.header_state.remote_last_modified = req.headers.iter()
                            .find(|header| header.name.eq_ignore_ascii_case("last-modified"))
                            .and_then(|header| str::from_utf8(header.value).ok())
                            .and_then(|value| httpdate::parse_http_date(value).ok());
                        return true;
                    }
                    // TODO it may be safer to obtain the size_written from the job_state, i.e., add a new item to
                    // the job state that stores the size the job should be started with. With the current
                    // implementation, we assume that the header method is always called before anything is written to
                    // the file.
                    let size_written = self.job_state.job_resources.as_ref().unwrap().file_state.size_written;
                    let path = self.job_state.order.filepath(&self.properties);
                    // TODO stick to a consistent terminology, everywhere: client_content_length = the content length
                    // as communicated to the client, i.e., what the client receives in his headers.
                    // provider_content_length = the content length we send to the provider.
                    let client_content_length = size_written + content_length;
                    if self.job_state.order.is_cacheable() {
                        create_cfs_file(&path, client_content_length);
                    }
                    debug!("Sending content length: {}", client_content_length);
                    let _ = self.job_state.tx.send(FlexoProgress::JobSize(client_content_length));
                } else if code == 304 {
                    if job_resources.if_modified_since.is_none() {
                        warn!("Remote mirror returned 304 without a conditional database request.");
                        return false;
                    }
                    debug!("Remote mirror reported that cached file is still current.");
                    job_resources.header_state.header_success = Some(HeaderOutcome::Cached);
                } else if code == 416 {
                    // If the requested file was already cached, but we don't know if the cached file has been
                    // downloaded completely or only partially, we send the Content-Range header in order to not
                    // download anything we already have available in cache.
                    // If the server responds with 416, we assume that the cached file was already complete.
                    job_resources.header_state.header_success = Some(HeaderOutcome::Unavailable);
                    let _ = self.job_state.tx.send(FlexoProgress::Completed);
                } else if (100..200).contains(&code) || (300..400).contains(&code) {
                    debug!("Received an interim response or redirect, waiting for the final header.");
                    job_resources.header_state.received_header.clear();
                } else {
                    job_resources.header_state.header_success = Some(HeaderOutcome::Unavailable);
                    if job_resources.last_chance && !revalidating {
                        let _ = self.job_state.tx.send(FlexoProgress::Unavailable);
                    }
                }
            }
            Ok(Status::Partial) => {
                // nothing to do, wait until this function is invoked again.
            }
            Err(e) => {
                error!("Unable to parse header: {:?}", e);
                return false;
            }
        }

        true
    }
}

fn create_cfs_file(path: &Path, complete_filesize: u64) {
    debug!("Creating CFS file for {:?}", &path);
    let cfs_path = cfs_path_from_pkg_path(path).unwrap();
    let mut cfs_file = match File::create(&cfs_path) {
        Ok(f) => f,
        Err(e) => {
            error!("Unable to create CFS file {:?}: {:?}", &cfs_path, e);
            return;
        }
    };
    match cfs_file.write_all(format!("{}\n", complete_filesize).as_bytes()) {
        Ok(_) => {
            debug!("Created CFS file: {:?} has size {}", &path, complete_filesize);
        }
        Err(e) => {
            error!("Unable to write to CFS file {:?}: {:?}", &cfs_path, e);
        }
    }
}

#[derive(Debug)]
pub struct DownloadChannel {
    handle: Easy2<DownloadState>,
}

impl DownloadChannel {
    fn if_modified_since(&self) -> Option<SystemTime> {
        self.handle.get_ref().job_state.job_resources.as_ref().unwrap().if_modified_since
    }

    fn publish_database(&mut self) -> std::io::Result<u64> {
        let state = self.handle.get_mut();
        let job_resources = state.job_state.job_resources.as_mut().unwrap();
        let file = &mut job_resources.file_state;
        let size = file.size_written;
        if !matches!(job_resources.header_state.header_success, Some(HeaderOutcome::Ok(expected)) if expected == size) {
            return Err(std::io::Error::new(ErrorKind::InvalidData, "incomplete database download"));
        }
        file.buf_writer.flush()?;
        let timestamp = job_resources.header_state.remote_last_modified.unwrap_or(UNIX_EPOCH);
        file.buf_writer.get_ref().set_modified(timestamp)?;
        let path = state.job_state.order.filepath(&state.properties);
        job_resources.temp_path.take().unwrap().persist(&path)?;
        if let Some(size_path) = cfs_path_from_pkg_path(&path) {
            if let Err(e) = fs::remove_file(&size_path) {
                if e.kind() != ErrorKind::NotFound {
                    warn!("Unable to remove obsolete database size file {:?}: {:?}", size_path, e);
                }
            }
        }
        Ok(size)
    }
}

impl Channel for DownloadChannel {
    type J = DownloadJob;

    fn progress_indicator(&self) -> Option<u64> {
        let job_resources = self.handle.get_ref().job_state.job_resources.as_ref().unwrap();
        let size_written = job_resources.file_state.size_written;
        if size_written > 0 {
            debug!("File size of {} is {}", job_resources.file_state.filename, size_written);
            Some(size_written)
        } else {
            debug!("File is still empty: {}", job_resources.file_state.filename);
            None
        }
    }

    fn job_state(&mut self) -> &mut JobState<DownloadJob> {
        &mut self.handle.get_mut().job_state
    }
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Copy, Clone)]
pub struct DynamicScoreCacheableDownload {
    num_failures: u32,
    // Avoid multiple parallel downloads from the same remote mirror: We prefer to download from different mirrors
    // instead, to increase the chance that the client's bandwidth is saturated.
    num_current_usages: usize,
    initial_score: MirrorResults,
}

impl DynamicScoreCacheable<MirrorResults> for DynamicScoreCacheableDownload {
    fn from_dynamic_provider_metrics(metrics: DynamicProviderMetrics<MirrorResults>) -> Self {
        Self {
            num_failures: metrics.num_failures,
            num_current_usages: metrics.num_current_usages,
            initial_score: metrics.initial_score,
        }
    }
}

/// Flexo's caching mechanism is based on the assumption that there's a one-to-one mapping between a file and its
/// content: For example, if a remote mirror stores the file glibc-2.33-3-x86_64.pkg.tar.zst, then this file will not
/// be replaced by a different file with the same name: if a new version of glibc is stored on the remote mirror,
/// then the version string will change as well. However, some files, such as database files (core.db etc.), don't use
/// version strings: The remote mirror simply replaces the old file by a new file with the same name. This means not
/// only that Flexo cannot use its caching mechanism for such files, it also needs to avoid that the following situation
/// occurs:
///     1. Client requests the uncacheable resource (e.g. core.db) from Flexo. Flexo chooses the remote mirror A to
///        fetch the resource.
///     2. A few moments later, the same client fetches the same resources from Flexo. Flexo now chooses a different
///        remote mirror B to fetch the resource. Mirror B has an older version of the resource than mirror A had
///        available at step 1.
/// Now, the client has obtained an older version in step 2 than it had obtained previously in step 1. This is not only
/// unexpected for the client, it also causes warnings like the following to appear in pacman:
///     warning: python-more-itertools: local (8.10.0-1) is newer than community (8.9.0-1)
/// To avoid this, we should not switch from one mirror to another when the requested resource is uncacheable: If we
/// have used mirror A to serve uncacheable resources in the past, we should continue to do so, unless there's a
/// good reason to switch the mirror (e.g. if the mirror is completely down or too slow).
/// To avoid unnecessary mirror-swapping for uncacheable resources, we introduce a separate struct to score the mirror.
#[derive(PartialEq, Eq, PartialOrd, Ord, Copy, Clone)]
pub struct DynamicScoreUncacheableDownload {
    num_failures: u32,
    initial_score: MirrorResults,
}

impl DynamicScoreUncacheable<MirrorResults> for DynamicScoreUncacheableDownload {
    fn from_dynamic_provider_metrics(metrics: DynamicProviderMetrics<MirrorResults>) -> Self {
        Self {
            num_failures: metrics.num_failures,
            initial_score: metrics.initial_score,
        }
    }
}

pub fn rated_providers_retry(
    mirrors: Vec<Mirror>,
    mirrors_auto: MirrorsAutoConfig,
    country_filter: &CountryFilter,
    limit: Limit,
) -> Vec<DownloadProvider> {
    for i in 0..LATENCY_TEST_NUM_ATTEMPTS {
        let mirrors_auto = match i {
            0 => mirrors_auto.clone(),
            _ => {
                warn!("The previous latency test did not return any results. This may indicate that your settings \
                in the [mirrors_auto] section are too restrictive. We will try to relax these settings and make \
                a new attempt.");
                let relaxed = mirrors_auto.relax();
                debug!("The relaxed settings are: {:?}", relaxed);
                relaxed
            },
        };
        let providers = rated_providers(mirrors.clone(), &mirrors_auto, &country_filter, limit);
        if !providers.is_empty() {
            return providers;
        }
    }
    error!("The latency test did not return any results.");
    vec![]
}

pub fn rated_providers(
    mut mirrors: Vec<Mirror>,
    mirrors_auto: &MirrorsAutoConfig,
    country_filter: &CountryFilter,
    limit: Limit,
) -> Vec<DownloadProvider> {
    mirrors.sort_by(|a, b| a.score.cmp(&b.score));
    debug!("Mirrors will be filtered according to the following criteria: {:#?}", mirrors_auto);
    debug!("The following CountryFilter is applied: {:?}", country_filter);
    let filtered_mirrors_unlimited = mirrors
        .into_iter()
        .filter(|mirror| mirror.protocol == MirrorProtocol::Http || mirror.protocol == MirrorProtocol::Https)
        .filter(|mirror| mirror.filter_predicate(&mirrors_auto))
        .filter(|mirror| country_filter.includes_country(&mirror.country_code));
    let filtered_mirror_urls: Vec<Mirror> = match limit {
        Limit::NoLimit => filtered_mirrors_unlimited.collect(),
        Limit::Limit(l) => filtered_mirrors_unlimited.take(l).collect(),
    };
    debug!("Running latency tests on the following mirrors: {:#?}", filtered_mirror_urls);
    let mut mirrors_with_latencies = Vec::new();
    let request_timeout = Duration::from_millis(mirrors_auto.timeout);
    let mut num_successes = 0;
    let mut num_failures = 0;
    for mirror in filtered_mirror_urls.into_iter() {
        let is_success = match mirror_fetch::measure_latency(&mirror.url, request_timeout) {
            Err(e) => {
                num_failures += 1;
                if e.code() == CURLE_OPERATION_TIMEDOUT {
                    debug!("Skip mirror {} due to timeout.", mirror.url);
                } else {
                    debug!("Skip mirror {}: Latency test did not succeed: {:?}", mirror.url, e);
                }
                false
            }
            Ok(mirror_results) => {
                mirrors_with_latencies.push((mirror, mirror_results));
                true
            }
        };
        if is_success {
            num_successes += 1;
        } else {
            num_failures += 1;
        }
    }
    debug!("Ran latency test on {} mirrors with {} successes and {} failures.",
           num_successes + num_failures, num_successes, num_failures);
    mirrors_with_latencies.sort_unstable_by_key(|(_, mirror_result)| {
        *mirror_result
    });

    mirrors_with_latencies.into_iter().map(|(mirror, mirror_results)| {
        DownloadProvider {
            uri: mirror.url.clone(),
            name: mirror.url,
            mirror_results,
            country_code: mirror.country_code,
        }
    }).collect()
}

pub fn read_client_header<T>(client_stream: &mut T) -> Result<ClientResponse, ClientError> where T: Read {
    let mut buf = [0; MAX_HEADER_SIZE + 1];
    let mut size_read_all = 0;

    loop {
        if size_read_all >= MAX_HEADER_SIZE {
            return Err(ClientError::BufferSizeExceeded);
        }
        let size = match client_stream.read(&mut buf[size_read_all..]) {
            Ok(0) => {
                // we need this branch in case the socket is closed: Otherwise, we would read a size of 0 indefinitely.
                return Ok(ClientResponse::SocketClosed);
            }
            Ok(s) if s > MAX_HEADER_SIZE => return Err(ClientError::BufferSizeExceeded),
            Ok(s) => s,
            Err(e) => {
                let error = match e.kind() {
                    ErrorKind::TimedOut => ClientError::TimedOut,
                    ErrorKind::WouldBlock => ClientError::TimedOut,
                    other => ClientError::Other(other),
                };
                return Err(error);
            }
        };
        size_read_all += size;

        let mut headers: [Header; 64] = [httparse::EMPTY_HEADER; MAX_HEADER_COUNT];
        let mut req: httparse::Request = httparse::Request::new(&mut headers);
        let res: std::result::Result<httparse::Status<usize>, httparse::Error> = req.parse(&buf[..size_read_all]);

        match res {
            Ok(Status::Complete(_result)) => {
                debug!("Received header from client");
                break(Ok(ClientResponse::Request(Request::new(req)?)))
            }
            Ok(Status::Partial) => {
                {}
            }
            Err(_) => {
                let client_status = ClientStatus { response_headers_sent: false };
                break Err(ClientError::InvalidHeader(client_status))
            }
        }
    }
}

pub fn uri_from_components(prefix: &str, suffix: &str) -> String {
    format!("{}/{}", prefix.trim_end_matches('/'), suffix.trim_start_matches('/'))
}

pub fn size_to_human_readable(size_in_bytes: u64) -> String {
    let exponent = ((size_in_bytes as f64).log2() / 10.0) as u32;
    let (unit, too_large) = match exponent {
        0 => ("B", false),
        1 => ("KiB", false),
        2 => ("MiB", false),
        3 => ("GiB", false),
        4 => ("TiB", false),
        5 => ("PiB", false),
        6 => ("EiB", false),
        7 => ("ZiB", false),
        8 => ("YiB", false),
        _ => ("B", true),
    };
    if too_large {
        format!("{}", size_in_bytes)
    } else {
        let quantity = (size_in_bytes as f64) / ((1024u64).pow(exponent) as f64);
        format!("{:.2} {}", quantity, unit)
    }
}

#[cfg(test)]
mod tests {
    use std::io::Error;
    use std::net::TcpListener;

    use super::*;

    struct TooMuchDataReader {}
    impl Read for TooMuchDataReader {
        fn read(&mut self, buf: &mut [u8]) -> Result<usize, Error> {
            // Notice that we cause an error by writing the exact amount of the maximum header size,
            // has received this exact amount of bytes, or if it has received more than 8192 bytes but the returned
            // value is 8192 because that is the maximum buffer size. So we cautiously assume the latter case and
            // return an error.
            let too_much_data = [0; MAX_HEADER_SIZE + 1];
            buf[..too_much_data.len()].copy_from_slice(&too_much_data);
            Ok(MAX_HEADER_SIZE + 1)
        }
    }

    // writes a single byte at a time.
    struct OneByteReader {
        size_read: usize,
    }
    impl Read for OneByteReader {
        fn read(&mut self, buf: &mut [u8]) -> Result<usize, Error> {
            if self.size_read < TEST_REQUEST_HEADER.len() {
                buf[0] = TEST_REQUEST_HEADER[self.size_read];
                self.size_read += 1;
                Ok(1)
            } else {
                Ok(0)
            }
        }
    }
    struct NoDelimiterReader {}
    impl Read for NoDelimiterReader {
        fn read(&mut self, buf: &mut [u8]) -> Result<usize, Error> {
            let array: [u8; TEST_CHUNK_SIZE] = [b'a'; TEST_CHUNK_SIZE];
            buf[0..TEST_CHUNK_SIZE].copy_from_slice(&array);
            Ok(256)
        }
    }

    #[test]
    fn test_buffer_size_exceeded() {
        let mut stream = TooMuchDataReader {};
        let result = read_client_header(&mut stream);
        assert_eq!(result, Err(ClientError::BufferSizeExceeded));
    }

    #[test]
    fn test_database_paths_are_cacheable_but_revalidated() {
        let db_order = DownloadOrder::new(StrPath::new("core/os/x86_64/core.db".to_owned()));
        assert!(db_order.is_cacheable());
        assert!(db_order.requires_revalidation());
        assert!(!db_order.use_cacheable_provider_selection());

        let files_order = DownloadOrder::new(StrPath::new("core/os/x86_64/core.files".to_owned()));
        assert!(files_order.is_cacheable());
        assert!(files_order.requires_revalidation());
        assert!(!files_order.use_cacheable_provider_selection());

        let db_sig_order = DownloadOrder::new(StrPath::new("core/os/x86_64/core.db.sig".to_owned()));
        assert!(!db_sig_order.is_cacheable());
        assert!(!db_sig_order.requires_revalidation());

        let files_sig_order = DownloadOrder::new(StrPath::new("core/os/x86_64/core.files.sig".to_owned()));
        assert!(!files_sig_order.is_cacheable());
        assert!(!files_sig_order.requires_revalidation());
    }

    fn database_fixture(root: &Path) -> (MirrorConfig, DownloadOrder, PathBuf) {
        let cache_directory = root.join("pkg");
        let properties = toml::from_str(&format!("\
            cache_directory = {:?}\n\
            mirrorlist_fallback_file = {:?}\n\
            port = 7878\n\
            low_speed_time_secs = 10\n\
            mirror_selection_method = \"predefined\"\n\
            mirrors_predefined = [\"http://example.invalid\"]\n",
            cache_directory.to_str().unwrap(),
            root.join("mirrorlist").to_str().unwrap(),
        )).unwrap();
        let order = DownloadOrder::new(StrPath::new("core/os/x86_64/core.db".to_owned()));
        let path = order.filepath(&properties);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        (properties, order, path)
    }

    fn mock_mirror(listener: &TcpListener, latency: Duration) -> DownloadProvider {
        let uri = format!("http://{}", listener.local_addr().unwrap());
        DownloadProvider {
            name: uri.clone(),
            uri,
            mirror_results: MirrorResults { total_time: latency, ..MirrorResults::default() },
            country_code: String::new(),
        }
    }

    #[test]
    fn test_database_rejects_304_without_a_validator() {
        let tempdir = tempfile::tempdir().unwrap();
        let (properties, order, path) = database_fixture(tempdir.path());
        let (tx, _) = crossbeam::channel::unbounded();
        let mut channel = order.new_channel(properties, tx, false).unwrap();
        assert_eq!(channel.if_modified_since(), None);
        assert!(!channel.handle.get_mut().header(b"HTTP/1.1 304 Not Modified\r\n\r\n"));
        assert!(!path.exists());
    }

    #[test]
    fn test_database_refresh_keeps_old_file_until_publish_and_shares_completion() {
        use std::io::{BufRead, BufReader};

        let tempdir = tempfile::tempdir().unwrap();
        let (properties, order, path) = database_fixture(tempdir.path());
        fs::write(&path, b"original").unwrap();
        // An obsolete size file makes this database ineligible for a conditional request.
        create_cfs_file(&path, 42);
        assert!(DownloadJob::cache_state(&order, &properties).is_none());
        assert_eq!(cached_database_modified_time(&path), None);

        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let provider = mock_mirror(&listener, Duration::ZERO);
        let mut jobs = JobContext::<DownloadJob>::new(vec![provider], properties);
        let (requests, rx_requests) = crossbeam::channel::unbounded();
        let (reply, rx_reply) = crossbeam::channel::unbounded::<&[u8]>();
        let timeout = Duration::from_secs(5);
        let mirror = std::thread::spawn(move || {
            for _ in 0..3 {
                let (stream, _) = listener.accept().unwrap();
                stream.set_read_timeout(Some(timeout)).unwrap();
                let mut reader = BufReader::new(stream);
                let mut headers = Vec::new();
                loop {
                    let mut line = String::new();
                    assert!(reader.read_line(&mut line).unwrap() > 0);
                    if line == "\r\n" {
                        break;
                    }
                    headers.push(line);
                }
                requests.send(headers).unwrap();
                reader.get_mut().write_all(rx_reply.recv_timeout(timeout).unwrap()).unwrap();
            }
        });

        let ScheduleOutcome::Scheduled(first) = jobs.try_schedule(order.clone(), None, None) else {
            panic!("expected the database download to be scheduled");
        };
        let request = rx_requests.recv_timeout(timeout).unwrap().concat().to_ascii_lowercase();
        assert!(!request.contains("if-modified-since:"));
        let ScheduleOutcome::AwaitingCompletion(second) = jobs.try_schedule(order.clone(), None, None) else {
            panic!("expected the second request to join the database download");
        };
        let mut old_file = File::open(&path).unwrap();
        assert!(first.rx_progress.is_empty());
        assert!(second.is_empty());
        reply.send(b"HTTP/1.1 200 OK\r\n\
                     Content-Length: 7\r\n\
                     Last-Modified: Thu, 06 Apr 2023 20:00:18 GMT\r\n\
                     Connection: close\r\n\r\nupdated").unwrap();
        assert_eq!(first.rx_progress.recv_timeout(timeout).unwrap(), FlexoProgress::Completed);
        assert_eq!(second.recv_timeout(timeout).unwrap(), FlexoProgress::Completed);
        first.join_handle.join().unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"updated");
        assert!(!cfs_path_from_pkg_path(&path).unwrap().exists());
        let mut old_contents = Vec::new();
        old_file.read_to_end(&mut old_contents).unwrap();
        assert_eq!(old_contents, b"original");

        let ScheduleOutcome::Scheduled(refresh) = jobs.try_schedule(order.clone(), None, None) else {
            panic!("expected the cached database to be revalidated");
        };
        let request = rx_requests.recv_timeout(timeout).unwrap().concat().to_ascii_lowercase();
        assert!(request.contains("if-modified-since: thu, 06 apr 2023 20:00:18 gmt\r\n"));
        assert!(!request.contains("\r\nrange:"));
        reply.send(b"HTTP/1.1 304 Not Modified\r\nConnection: close\r\n\r\n").unwrap();
        assert_eq!(refresh.rx_progress.recv_timeout(timeout).unwrap(), FlexoProgress::Completed);
        refresh.join_handle.join().unwrap();

        let ScheduleOutcome::Scheduled(refresh) = jobs.try_schedule(order, None, None) else {
            panic!("expected another database refresh");
        };
        rx_requests.recv_timeout(timeout).unwrap();
        reply.send(b"HTTP/1.1 200 OK\r\nContent-Length: 12\r\nConnection: close\r\n\r\ntruncated").unwrap();
        assert!(matches!(refresh.join_handle.join().unwrap(), JobOutcome::Error(_)));
        assert_eq!(refresh.rx_progress.try_recv(), Err(crossbeam::channel::TryRecvError::Disconnected));
        assert_eq!(fs::read(&path).unwrap(), b"updated");
        assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
        mirror.join().unwrap();
    }

    #[test]
    fn test_database_refresh_stops_retrying_when_window_expires() {
        let tempdir = tempfile::tempdir().unwrap();
        let (properties, order, _) = database_fixture(tempdir.path());
        let first_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let backup_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let first = mock_mirror(&first_listener, Duration::ZERO);
        let backup = mock_mirror(&backup_listener, Duration::from_millis(1));
        let backup_id = backup.identifier();
        let mut jobs = JobContext::<DownloadJob>::new(vec![first, backup], properties);

        let mirror = std::thread::spawn(move || {
            let (mut stream, _) = first_listener.accept().unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            assert!(matches!(read_client_header(&mut stream).unwrap(), ClientResponse::Request(_)));
            std::thread::sleep(DATABASE_RETRY_WINDOW);
            stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
        });
        let ScheduleOutcome::Scheduled(refresh) = jobs.try_schedule(order, None, None) else {
            panic!("expected a database refresh");
        };
        let progress = refresh.rx_progress.recv_timeout(DATABASE_RETRY_WINDOW + Duration::from_secs(3));
        assert_eq!(progress.unwrap(), FlexoProgress::Unavailable);
        refresh.join_handle.join().unwrap();
        mirror.join().unwrap();
        assert!(!jobs.provider_metrics().contains_key(&backup_id));
    }

    #[test]
    fn test_formatting_two_kilobytes() {
        let result = size_to_human_readable(2048);
        assert_eq!(result, "2.00 KiB");
    }

    #[test]
    fn test_formatting_two_point_something_kilobytes() {
        let result = size_to_human_readable(2300);
        assert_eq!(result, "2.25 KiB");
    }

    #[test]
    fn test_formatting_gigabyte() {
        let result = size_to_human_readable(7040779151);
        assert_eq!(result, "6.56 GiB");
    }

    #[test]
    fn test_formatting_two_bytes() {
        let result = size_to_human_readable(2);
        assert_eq!(result, "2.00 B");
    }
}
