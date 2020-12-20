extern crate flexo;

use crate::mirror_config::{MirrorConfig, MirrorsAutoConfig};
use crate::mirror_fetch;
use crate::mirror_fetch::{MirrorUrl, MirrorProtocol};
use crate::str_path::StrPath;


use flexo::*;
use std::fs::File;
use std::time::Duration;
use std::cmp::Ordering;
use crossbeam::crossbeam_channel::Sender;
use curl::easy::{Easy2, Handler, WriteError, HttpVersion};
use std::fs::OpenOptions;
use std::io::BufWriter;
use std::collections::hash_map::RandomState;
use std::collections::HashMap;
use walkdir::WalkDir;
use std::ffi::OsString;
use httparse::{Status, Header};
use std::io::{Read, ErrorKind, Write};
use std::path::Path;
use std::string::FromUtf8Error;
use std::num::ParseIntError;
use serde::{Serialize, Deserialize};

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

const DEFAULT_LOW_SPEED_TIME_SECS: u64 = 2;

const MAX_REDIRECTIONS: u32 = 3;

const LATENCY_TEST_NUM_ATTEMPTS: u32 = 5;

#[derive(Debug, PartialEq, Eq)]
pub enum ClientError {
    BufferSizeExceeded,
    TimedOut,
    SocketClosed,
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

#[derive(Debug)]
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

fn parse_range_header_value(s: &str) -> u64 {
    let s = s.to_lowercase();
    // We ignore everything after the - sign: We assume that pacman will never request only up to a certain size,
    // pacman will only skip the beginning of a file if the file has already been partially downloaded.
    let range_start: &str = s.split('-').next().unwrap();
    let range_start = range_start.replace("bytes=", "");
    range_start.parse::<u64>().unwrap()
}

#[derive(Debug, PartialEq, Eq)]
pub struct GetRequest {
    pub resume_from: Option<u64>,
    pub path: StrPath,
}

impl GetRequest {
    fn new(request: httparse::Request) -> Result<Self, ClientError> {
        let range_header = request.headers
            .iter()
            .find(|h| h.name.to_lowercase() == "range");
        let resume_from = range_header.map(|h| parse_range_header_value(std::str::from_utf8(h.value).unwrap()));
        match request.method {
            Some("GET") => {},
            Some(_method) => {
                let client_status = ClientStatus { response_headers_sent: false };
                return Err(ClientError::UnsupportedHttpMethod(client_status));
            },
            None => panic!("Expected the request method to be set."),
        }
        let p = &request.path.unwrap()[1..]; // Skip the leading "/"
        Ok(Self {
            path: StrPath::new(p.to_owned()),
            resume_from,
        })
    }
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Hash, Clone, Debug)]
pub struct DownloadProvider {
    pub uri: String,
    pub mirror_results: MirrorResults,
    pub country_code: String,
}

impl Provider for DownloadProvider {
    type J = DownloadJob;

    fn new_job(&self, properties: &<<Self as Provider>::J as Job>::PR, order: DownloadOrder) -> DownloadJob {
        let uri = format!("{}/{}", self.uri, order.filepath.to_str());
        let provider = self.clone();
        let properties = properties.clone();
        DownloadJob {
            properties,
            provider,
            uri,
            order,
        }
    }

    fn identifier(&self) -> &String {
        &self.uri
    }

    fn score(&self) -> MirrorResults {
        self.mirror_results
    }

    fn description(&self) -> String {
        self.uri.clone()
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

    fn provider(&self) -> &DownloadProvider {
        &self.provider
    }

    fn order(&self) -> DownloadOrder {
        self.order.clone()
    }

    fn properties(&self) -> Self::PR {
        self.properties.clone()
    }

    fn initialize_cache(properties: Self::PR) -> HashMap<DownloadOrder, OrderState, RandomState> {
        let mut hashmap: HashMap<Self::O, OrderState> = HashMap::new();
        let mut sum_size = 0;
        for entry in WalkDir::new(&properties.cache_directory) {
            let entry = entry.expect("Error while reading directory entry");
            let key = OsString::from("user.content_length");
            if entry.file_type().is_file() {
                let file = File::open(entry.path())
                    .unwrap_or_else(|_| panic!("Unable to open file {:?}", entry.path()));
                let file_size = file.metadata().expect("Unable to fetch file metadata").len();
                sum_size += file_size;
                let complete_size = match xattr::get(entry.path(), &key).expect("Unable to get extended file attributes") {
                    Some(value) => {
                        let result = String::from_utf8(value).map_err(FileAttrError::from)
                            .and_then(|v| v.parse::<u64>().map_err(FileAttrError::from));
                        match result {
                            Ok(v) => Some(v),
                            Err(e) => {
                                error!("Unable to read extended attribute user.content_length from file {:?}: {:?}",
                                       entry.path(), e);
                                None
                            },
                        }
                    },
                    None => {
                        // Flexo sets the extended attributes for all files, but this file lacks this attribute:
                        // We assume that this mostly happens when the user copies files into the directory used
                        // by flexo, and we further assume that users will do this only if this file is complete.
                        // Therefore, we can set the content length attribute of this file to the file size.
                        debug!("Set content length for file: #{:?}", entry.path());
                        let value = file_size.to_string();
                        match xattr::set(entry.path(), &key, &value.as_bytes()) {
                            Ok(()) => {},
                            Err(e) => {
                                error!("Unable to set extended file attributes for {:?}: {:?}. Please make sure that \
                                flexo has write permissions for this file.", entry.path(), e)
                            },
                        }
                        Some(file_size)
                    },
                };
                let sub_path = entry.path().strip_prefix(&properties.cache_directory).unwrap();
                let order = DownloadOrder {
                    filepath: StrPath::new(sub_path.to_str().unwrap().to_owned())
                };
                let cached_item = CachedItem {
                    cached_size: file_size,
                    complete_size,
                };
                let state = OrderState::Cached(cached_item);
                hashmap.insert(order, state);
            }
        }
        let size_formatted = size_to_human_readable(sum_size);
        info!("Retrieved {} files with a total size of {} from local file system.", hashmap.len(), size_formatted);

        hashmap
    }

    fn serve_from_provider(self, mut channel: DownloadChannel,
                           properties: MirrorConfig,
                           resume_from: u64) -> JobResult<DownloadJob> {
        let url = format!("{}", &self.uri);
        debug!("Fetch package from remote mirror: {}. Resume from byte {}", &url, resume_from);
        channel.handle.url(&url).unwrap();
        channel.handle.resume_from(resume_from).unwrap();
        // we use httparse to parse the headers, but httparse doesn't support HTTP/2 yet. HTTP/2 shouldn't provide
        // any benefit for our use case (afaik), so this setting should not have any downsides.
        channel.handle.http_version(HttpVersion::V11).unwrap();
        // TODO avoid hardcoded values, make this configurable.
        channel.handle.connect_timeout(Duration::from_secs(3)).unwrap();
        match properties.low_speed_limit {
            None => {},
            Some(speed) => {
                channel.handle.low_speed_limit(speed).unwrap();
                let low_speed_time_secs = properties.low_speed_time_secs.unwrap_or(DEFAULT_LOW_SPEED_TIME_SECS);
                debug!("Set low_speed_time to {} seconds.", low_speed_time_secs);
                channel.handle.low_speed_time(std::time::Duration::from_secs(low_speed_time_secs)).unwrap();
            },
        }
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
        match channel.progress_indicator() {
            None => {},
            Some(start) => {
                channel.handle.resume_from(start).unwrap();
            }
        }
        info!("Start download from {}", self.provider.description());
        match channel.handle.perform() {
            Ok(()) => {
                let response_code = channel.handle.response_code().unwrap();
                info!("{} replied with status code {}.", self.provider.description(), response_code);
                if response_code >= 200 && response_code < 300 {
                    let size = channel.progress_indicator().unwrap();
                    JobResult::Complete(JobCompleted::new(channel, self.provider, size as i64))
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
                    warn!("Unable to download from {:?}: Timeout reached. Try another remote mirror.", &url);
                } else {
                    warn!("An unknown error occurred while downloading from remote mirror {:?}: {:?}", &url, e);
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

    fn acquire_resources(order: &DownloadOrder, properties: &MirrorConfig, last_chance: bool) -> std::io::Result<DownloadJobResources> {
        let path = Path::new(&properties.cache_directory).join(&order.filepath);
        info!("Attempt to create file: {:?}", path);
        let f = OpenOptions::new().create(true).append(true).open(path);
        if f.is_err() {
            warn!("Unable to create file: {:?}", f);
        }
        let f = f?;
        let size_written = f.metadata()?.len();
        let buf_writer = BufWriter::new(f);
        let header_state = HeaderState {
            received_header: vec![],
            header_success: None
        };
        let file_state = FileState  {
            buf_writer,
            size_written,
        };
        let download_job_resources = DownloadJobResources {
            file_state,
            header_state,
            last_chance,
        };
        Ok(download_job_resources)
    }
}

#[derive(PartialEq, Eq, Hash, Clone, Debug)]
pub struct DownloadOrder {
    /// This path is relative to the given root directory.
    pub filepath: StrPath,
}

impl Order for DownloadOrder {
    type J = DownloadJob;

    fn new_channel(self, properties: MirrorConfig,
                   tx: Sender<FlexoProgress>,
                   last_chance: bool) -> Result<DownloadChannel, <Self::J as Job>::OE> {
        let download_state = DownloadState::new(self, properties, tx, last_chance)?;
        Ok(DownloadChannel {
            handle: Easy2::new(download_state)
        })
    }

    fn reuse_channel(self,
                     properties: MirrorConfig,
                     tx: Sender<FlexoProgress>,
                     last_chance: bool,
                     previous_channel: DownloadChannel) -> Result<DownloadChannel, <Self::J as Job>::OE> {
        let download_state = DownloadState::new(self, properties, tx, last_chance)?;
        let mut handle = previous_channel.handle;
        handle.get_mut().replace(download_state);
        Ok(DownloadChannel {
            handle
        })
    }

    fn is_cacheable(&self) -> bool {
        !(self.filepath.to_str().ends_with(".db") || self.filepath.to_str().ends_with(".sig"))
    }
}


#[derive(Debug)]
pub struct FileState {
    buf_writer: BufWriter<File>,
    size_written: u64,
}

#[derive(Debug)]
pub struct DownloadJobResources {
    file_state: FileState,
    header_state: HeaderState,
    last_chance: bool,
}

#[derive(Debug)]
pub struct HeaderState {
    received_header: Vec<u8>,
    header_success: Option<HeaderOutcome>,
}

#[derive(Debug)]
enum HeaderOutcome {
    /// Header was read successfully and we're ready to write the payload to the local file system.
    Ok(u64),
    /// Server has returned 404.
    Unavailable,
}

#[derive(Debug)]
struct DownloadState {
    job_state: JobState<DownloadJob>,
    properties: MirrorConfig,
}

impl DownloadState {

    pub fn new(order: DownloadOrder, properties: MirrorConfig, tx: Sender<FlexoProgress>, last_chance: bool) -> std::io::Result<Self> {
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
        let mut job_resources = self.job_state.job_resources.as_mut().unwrap();
        match job_resources.header_state.header_success {
            Some(HeaderOutcome::Ok(_content_length)) => {},
            Some(HeaderOutcome::Unavailable) => {
                // If the header says the file is not available, we return early without writing anything to
                // the file on disk. The content returned is just the HTML code saying the file is not available,
                // so there is no reason to write this data to disk.
                info!("File unavailable - return content length without writing anything.");
                return Ok(data.len());
            },
            None => {
                unreachable!("The header should have been parsed before this function is called");
            }
        }
        if job_resources.file_state.size_written == 0 {
            info!("Starting to transfer body to file {}", self.job_state.order.filepath.to_str());
        }
        job_resources.file_state.size_written += data.len() as u64;
        match job_resources.file_state.buf_writer.write(data) {
            Ok(size) => {
                let len = job_resources.file_state.buf_writer.get_ref().metadata().unwrap().len();
                let _result = self.job_state.tx.send(FlexoProgress::Progress(len));
                Ok(size)
            },
            Err(e) => {
                error!("Error while writing data: {:?}", e);
                Err(WriteError::Pause)
            }
        }
    }

    fn header(&mut self, data: &[u8]) -> bool {
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
                debug!("HTTP response code is {}", code);
                if code == 200 || code == 206 {
                    let content_length = req.headers.iter().find_map(|header|
                        if header.name.eq_ignore_ascii_case("content-length") {
                            Some(std::str::from_utf8(header.value).unwrap().parse::<u64>().unwrap())
                        } else {
                            None
                        }
                    ).unwrap();
                    debug!("Content length is {}", content_length);
                    job_resources.header_state.header_success = Some(HeaderOutcome::Ok(content_length));
                    let path = Path::new(&self.properties.cache_directory).join(&self.job_state.order.filepath);
                    let key = OsString::from("user.content_length");
                    // TODO it may be safer to obtain the size_written from the job_state, i.e., add a new item to
                    // the job state that stores the size the job should be started with. With the current
                    // implementation, we assume that the header method is always called before anything is written to
                    // the file.
                    let size_written = self.job_state.job_resources.as_ref().unwrap().file_state.size_written;
                    // TODO stick to a consistent terminology, everywhere: client_content_length = the content length
                    // as communicated to the client, i.e., what the client receives in his headers.
                    // provider_content_length = the content length we send to the provider.
                    let client_content_length = size_written + content_length;
                    let value = format!("{}", client_content_length);
                    debug!("Setting the extended file attribute");
                    xattr::set(path, &key, &value.as_bytes())
                        .expect("Unable to set extended file attributes");
                    debug!("Sending content length: {}", client_content_length);
                    let message: FlexoProgress = FlexoProgress::JobSize(client_content_length);
                    let _ = self.job_state.tx.send(message);
                }  else if code == 416 {
                    // If the requested file was already cached, but we don't know if the cached file has been
                    // downloaded completely or only partially, we send the Content-Range header in order to not
                    // download anything we already have available in cache.
                    // If the server responds with 416, we assume that the cached file was already complete.
                    job_resources.header_state.header_success = Some(HeaderOutcome::Unavailable);
                    error!("All providers have been unable to fulfil this request.");
                    let message: FlexoProgress = FlexoProgress::Completed;
                    let _ = self.job_state.tx.send(message);
                } else if !job_resources.last_chance {
                    job_resources.header_state.header_success = Some(HeaderOutcome::Unavailable);
                    info!("Hoping that another provider can fulfil this request…");
                } else if job_resources.last_chance {
                    job_resources.header_state.header_success = Some(HeaderOutcome::Unavailable);
                    error!("All providers have been unable to fulfil this request.");
                    let message: FlexoProgress = FlexoProgress::Unavailable;
                    let _ = self.job_state.tx.send(message);
                }
            }
            Ok(Status::Partial) => {
                // nothing to do, wait until this function is invoked again.
            }
            Err(e) => {
                panic!("Error: {:?}", e)
            }
        }

        true
    }
}

#[derive(Debug)]
pub struct DownloadChannel {
    handle: Easy2<DownloadState>,
}

impl Channel for DownloadChannel {
    type J = DownloadJob;

    fn progress_indicator(&self) -> Option<u64> {
        let job_resources = self.handle.get_ref().job_state.job_resources.as_ref().unwrap();
        let size_written = job_resources.file_state.size_written;
        if size_written > 0 {
            Some(size_written)
        } else {
            None
        }
    }

    fn job_state(&mut self) -> &mut JobState<DownloadJob> {
        &mut self.handle.get_mut().job_state
    }
}

pub fn rate_providers_uncached_retry(mirror_urls: Vec<MirrorUrl>,
                                     mirrors_auto: MirrorsAutoConfig,
                                     country_filter: &CountryFilter,
                                     limit: Limit
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
        let providers = rate_providers_uncached(mirror_urls.clone(), &mirrors_auto, &country_filter, limit);
        if !providers.is_empty() {
            return providers;
        }
    }
    error!("The latency test did not return any results.");
    vec![]
}

pub fn rate_providers_uncached(mut mirror_urls: Vec<MirrorUrl>,
                               mirrors_auto: &MirrorsAutoConfig,
                               country_filter: &CountryFilter,
                               limit: Limit
) -> Vec<DownloadProvider> {
    mirror_urls.sort_by(|a, b| a.score.partial_cmp(&b.score).unwrap());
    debug!("Mirrors will be filtered according to the following criteria: {:#?}", mirrors_auto);
    debug!("The following CountryFilter is applied: {:?}", country_filter);
    let filtered_mirror_urls_unlimited = mirror_urls
        .into_iter()
        .filter(|x| x.protocol == MirrorProtocol::Http || x.protocol == MirrorProtocol::Https)
        .filter(|x| x.filter_predicate(&mirrors_auto))
        .filter(|x| country_filter.includes_country(&x.country_code));
    let filtered_mirror_urls: Vec<MirrorUrl> = match limit {
        Limit::NoLimit => filtered_mirror_urls_unlimited.collect(),
        Limit::Limit(l) => filtered_mirror_urls_unlimited.take(l).collect(),
    };
    debug!("Running latency tests on the following mirrors: {:#?}", filtered_mirror_urls);
    let mut mirrors_with_latencies = Vec::new();
    let timeout = Duration::from_millis(mirrors_auto.timeout);
    for mirror in filtered_mirror_urls.into_iter() {
        match mirror_fetch::measure_latency(&mirror.url, timeout) {
            Err(e) => {
                if e.code() == CURLE_OPERATION_TIMEDOUT {
                    debug!("Skip mirror {} due to timeout.", mirror.url);
                } else {
                    warn!("Error during latency test of mirror {}: {:?}", mirror.url, e);
                }
            }
            Ok(mirror_results) => {
                mirrors_with_latencies.push((mirror, mirror_results));
            }
        }
    }
    mirrors_with_latencies.sort_unstable_by_key(|(_, mirror_result)| {
        *mirror_result
    });

    mirrors_with_latencies.into_iter().map(|(mirror, mirror_results)| {
        DownloadProvider {
            uri: mirror.url,
            mirror_results,
            country_code: mirror.country_code,
        }
    }).collect()
}

pub fn rate_providers_cached(mirror_urls: Vec<MirrorUrl>,
                             mirror_config: &MirrorConfig,
                             prev_rated_providers: Vec<DownloadProvider>,
) -> Vec<DownloadProvider> {
    let mirrors_auto = mirror_config.mirrors_auto.as_ref().unwrap();
    let country_filter = country_filter(&prev_rated_providers, mirrors_auto.num_mirrors);
    let limit = Limit::Limit(mirrors_auto.num_mirrors);

    rate_providers_uncached_retry(mirror_urls,
                                  mirror_config.mirrors_auto.as_ref().unwrap().clone(),
                                  &country_filter,
                                  limit,
    )
}

fn country_filter(prev_rated_providers: &Vec<DownloadProvider>, num_mirrors: usize) -> CountryFilter {
    // If the user already ran a latency test, then we can restrict our latency tests to mirrors that are located at a
    // country that scored well in the previous latency test. For example, for users located in Australia, we will
    // not consider European mirrors because the previous latency test should have revealed that mirrors from
    // Australia have better latency than mirrors from European countries.
    let countries = prev_rated_providers.iter()
        .take(num_mirrors)
        .map(|m| m.country_code.clone())
        .collect::<Vec<String>>();

    CountryFilter::SelectedCountries(countries)
}

pub fn read_client_header<T>(stream: &mut T) -> Result<GetRequest, ClientError> where T: Read {
    let mut buf = [0; MAX_HEADER_SIZE + 1];
    let mut size_read_all = 0;

    loop {
        if size_read_all >= MAX_HEADER_SIZE {
            return Err(ClientError::BufferSizeExceeded);
        }
        let size = match stream.read(&mut buf[size_read_all..]) {
            Ok(0) => {
                // we need this branch in case the socket is closed: Otherwise, we would read a size of 0 indefinitely.
                return Err(ClientError::SocketClosed);
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
                break(Ok(GetRequest::new(req)?))
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
    use super::*;
    use std::io::Error;

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
