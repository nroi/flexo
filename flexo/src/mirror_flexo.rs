extern crate flexo;

use crate::mirror_config::MirrorConfig;
use crate::mirror_fetch;
use crate::mirror_fetch::MirrorUrl;

use flexo::*;
use http::Uri;
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
use xattr;
use std::ffi::OsString;
use httparse::{Status, Header};
use std::io::{Read, ErrorKind, Write};
use std::path::{Path, PathBuf};

// Since a restriction for the size of header fields is also implemented by web servers like NGINX or Apache,
// we keep things simple by just setting a fixed buffer length.
// TODO return 414 (Request-URI Too Large) if this size is exceeded, instead of just panicking.
const MAX_HEADER_SIZE: usize = 8192;

const MAX_HEADER_COUNT: usize = 64;

#[cfg(test)]
pub const PATH_PREFIX: &str = "";

#[cfg(not (test))]
pub const PATH_PREFIX: &str = "";

#[cfg(test)]
const TEST_CHUNK_SIZE: usize = 128;

#[cfg(test)]
const TEST_REQUEST_HEADER: &[u8] = "GET / HTTP/1.1\r\nHost: www.example.com\r\n\r\n".as_bytes();

#[derive(Debug, PartialEq, Eq)]
pub enum StreamReadError {
    BufferSizeExceeded,
    TimedOut,
    SocketClosed,
    Other(ErrorKind)
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
    pub path: PathBuf,
}

impl GetRequest {
    fn new(request: httparse::Request) -> Self {
        let range_header = request.headers
            .iter()
            .find(|h| h.name.to_lowercase() == "range");
        let resume_from = range_header.map(|h| parse_range_header_value(std::str::from_utf8(h.value).unwrap()));
        match request.method {
            Some("GET") => {},
            method => panic!("Unexpected method: #{:?}", method)
        }
        let p = &request.path.unwrap()[1..]; // Skip the leading "/"
        let path = Path::new(p).to_path_buf();
        Self {
            path,
            resume_from,
        }
    }
}

#[derive(PartialEq, Eq, Hash, Clone, Debug)]
pub struct DownloadProvider {
    pub uri: Uri,
    pub mirror_results: MirrorResults,
    pub country: String,
}

impl Provider for DownloadProvider {
    type J = DownloadJob;

    fn new_job(&self, properties: &<<Self as Provider>::J as Job>::PR, order: DownloadOrder) -> DownloadJob {
        let uri_string = format!("{}{}", self.uri, order.filepath);
        let uri = uri_string.parse::<Uri>().unwrap();
        let provider = self.clone();
        let properties = properties.clone();
        DownloadJob {
            properties,
            provider,
            uri,
            order,
        }
    }

    fn identifier(&self) -> &Uri {
        &self.uri
    }

    fn score(&self) -> MirrorResults {
        self.mirror_results
    }
}

#[derive(PartialEq, Eq, Hash, Copy, Clone, Debug, Default)]
pub struct MirrorResults {
    pub namelookup_duration: Duration,
    pub connect_duration: Duration,
}

impl Ord for MirrorResults {
    fn cmp(&self, other: &Self) -> Ordering {
        self.connect_duration.cmp(&other.connect_duration)
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
    uri: Uri,
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

impl Job for DownloadJob {
    type S = MirrorResults;
    type JS = FileState;
    type C = DownloadChannel;
    type O = DownloadOrder;
    type P = DownloadProvider;
    type E = DownloadJobError;
    type PI = Uri;
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
                let file = File::open(entry.path()).unwrap_or_else(|_| panic!("Unable to open file {:?}", entry.path()));
                let file_size = file.metadata().expect("Unable to fetch file metadata").len();
                sum_size += file_size;
                let complete_size = match xattr::get(entry.path(), &key).expect("Unable to get extended file attributes") {
                    Some(value) => {
                        String::from_utf8(value).unwrap().parse::<u64>().unwrap()
                    },
                    None => {
                        // Flexo sets the extended attributes for all files, but this file lacks this attribute:
                        // We assume that this mostly happens when the user copies files into the directory used
                        // by flexo, and we further assume that users will do this only if this file is complete.
                        // Therefore, we can set the content length attribute of this file to the file size.
                        println!("Set content length for file: #{:?}", entry.path());
                        let value = file_size.to_string();
                        xattr::set(entry.path(), &key, &value.as_bytes())
                            .expect("Unable to set extended file attributes");
                        file_size
                    },
                };
                let sub_path = entry.path().strip_prefix(&properties.cache_directory).unwrap();
                let order = DownloadOrder {
                    filepath: sub_path.to_str().unwrap().to_owned()
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
        println!("Retrieved {} files with a total size of {} from local file system.", hashmap.len(), size_formatted);

        hashmap
    }

    fn serve_from_provider(self, mut channel: DownloadChannel, properties: MirrorConfig, resume_from: u64) -> JobResult<DownloadJob> {
        let url = format!("{}", &self.uri);
        println!("Fetch package from remote mirror: {}", &url);
        channel.handle.url(&url).unwrap();
        channel.handle.resume_from(resume_from).unwrap();
        // we use httparse to parse the headers, but httparse doesn't support HTTP/2 yet. HTTP/2 shouldn't provide
        // any benefit for our use case (afaik), so this setting should not have any downsides.
        channel.handle.http_version(HttpVersion::V11).unwrap();
        match properties.mirrors_auto.low_speed_limit {
            None => {},
            Some(speed) => {
                channel.handle.low_speed_limit(speed).unwrap();
                let low_speed_time_secs = properties.mirrors_auto.low_speed_time_secs.unwrap_or(4);
                channel.handle.low_speed_time(std::time::Duration::from_secs(low_speed_time_secs)).unwrap();
            },
        }
        match properties.mirrors_auto.max_speed_limit {
            None => {
                println!("No speed limit was set.")
            },
            Some(speed) => {
                println!("Apply speed limit of {}/s", size_to_human_readable(speed));
                channel.handle.max_recv_speed(speed).unwrap();
            },
        }
        channel.handle.follow_location(true).unwrap();
        channel.handle.max_redirections(3).unwrap();
        match channel.progress_indicator() {
            None => {},
            Some(start) => {
                channel.handle.resume_from(start).unwrap();
            }
        }
        match channel.handle.perform() {
            Ok(()) => {
                let response_code = channel.handle.response_code().unwrap();
                if response_code >= 200 && response_code < 300 {
                    println!("Received header from provider, status OK");
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
                println!("Error occurred during download from provider: {:?}", e);
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
        dbg!(&error);
        match error {
            OrderError::IoError(e) if e.kind() == ErrorKind::NotFound => {
                // The client has specified a path that does not exist on the local file system. This can happen
                // if the required directory structure has not been created on the device running flexo, or if
                // the client has submitted an invalid request.
                JobResult::ClientError
            }
            _ => {
                unimplemented!()
            }
        }
    }
}

#[derive(PartialEq, Eq, Hash, Clone, Debug)]
pub struct DownloadOrder {
    /// This path is relative to the given root directory.
    pub filepath: String,
}

impl Order for DownloadOrder {
    type J = DownloadJob;

    fn new_channel(self, properties: MirrorConfig, tx: Sender<FlexoProgress>, last_chance: bool) -> Result<DownloadChannel, <Self::J as Job>::OE> {
        let download_state = DownloadState::new(self, properties, tx, last_chance)?;
        Ok(DownloadChannel {
            handle: Easy2::new(download_state)
        })
    }

    fn is_cacheable(&self) -> bool {
        !(self.filepath.ends_with(".db") || self.filepath.ends_with(".sig"))
    }
}

#[derive(Debug)]
pub struct FileState {
    buf_writer: BufWriter<File>,
    size_written: u64,
}

impl JobState for FileState {
    type J = DownloadJob;
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
    job_state: JobStateItem<DownloadJob>,
    // TODO maybe the following items belong to the job state.
    received_header: Vec<u8>,
    last_chance: bool,
    properties: MirrorConfig,
    header_success: Option<HeaderOutcome>,
}

impl DownloadState {
    pub fn new(order: DownloadOrder, properties: MirrorConfig, tx: Sender<FlexoProgress>, last_chance: bool) -> std::io::Result<Self> {
        let path = Path::new(&properties.cache_directory).join(&order.filepath);
        println!("Attempt to create file: {:?}", path);
        let f = OpenOptions::new().create(true).append(true).open(path);
        if f.is_err() {
            debug!("Unable to create file: {:?}", f);
        }
        let f = f?;
        let size_written = f.metadata()?.len();
        let buf_writer = BufWriter::new(f);
        let job_state = JobStateItem {
            order,
            job_state: Some(FileState {
                buf_writer,
                size_written,
            }),
            tx,
        };
        let received_header = Vec::new();
        Ok(DownloadState { job_state, received_header, last_chance, properties, header_success: None })
    }

    pub fn reset(&mut self, order: DownloadOrder, tx: Sender<FlexoProgress>) -> Result<(), OrderError> {
        if order != self.job_state.order {
            let c = DownloadState::new(order, self.properties.clone(), tx, self.last_chance)?;
            self.job_state = c.job_state;
            self.header_success = None;
            self.received_header = Vec::new();
        }
        Ok(())
    }
}

impl Handler for DownloadState {
    fn write(&mut self, data: &[u8]) -> Result<usize, WriteError> {
        match self.header_success {
            Some(HeaderOutcome::Ok(_content_length)) => {},
            Some(HeaderOutcome::Unavailable) => {
                // If the header says the file is not available, we return early without writing anything to
                // the file on disk. The content returned is just the HTML code saying the file is not available,
                // so there is no reason to write this data to disk.
                println!("File unavailable - return content length without writing anything.");
                return Ok(data.len());
            },
            None => {
                unreachable!("The header should have been parsed before this function is called");
            }
        }
        match self.job_state.job_state.iter_mut().next() {
            None => panic!("Expected the state to be initialized."),
            Some(file_state) => {
                file_state.size_written += data.len() as u64;
                match file_state.buf_writer.write(data) {
                    Ok(size) => {
                        let len = file_state.buf_writer.get_ref().metadata().unwrap().len();
                        let _result = self.job_state.tx.send(FlexoProgress::Progress(len));
                        Ok(size)
                    },
                    Err(e) => {
                        println!("Error while writing data: {:?}", e);
                        Err(WriteError::Pause)
                    }
                }
            }
        }
    }

    fn header(&mut self, data: &[u8]) -> bool {
        self.received_header.extend(data);
        dbg!(std::str::from_utf8(data).unwrap());

        let mut headers: [Header; MAX_HEADER_COUNT] = [httparse::EMPTY_HEADER; MAX_HEADER_COUNT];
        let mut req: httparse::Response = httparse::Response::new(&mut headers);
        let result: std::result::Result<httparse::Status<usize>, httparse::Error> =
            req.parse(self.received_header.as_slice());

        match result {
            Ok(Status::Complete(_header_size)) => {
                debug!("Received complete header from remote mirror");
                let code = req.code.unwrap();
                debug!("code is {}", code);
                if code >= 200 && code < 300 {
                    let content_length = req.headers.iter().find_map(|header|
                        if header.name.eq_ignore_ascii_case("content-length") {
                            Some(std::str::from_utf8(header.value).unwrap().parse::<u64>().unwrap())
                            // Some(header.value.parse::<u64>().unwrap())
                        } else {
                            None
                        }
                    ).unwrap();
                    self.header_success = Some(HeaderOutcome::Ok(content_length));
                    let path = Path::new(&self.properties.cache_directory).join(&self.job_state.order.filepath);
                    let key = OsString::from("user.content_length");
                    // TODO it may be safer to obtain the size_written from the job_state, i.e., add a new item to
                    // the job state that stores the size the job should be started with. With the current implementation,
                    // we assume that the header method is always called before anything is written to the file.
                    let size_written = self.job_state.job_state.as_ref().unwrap().size_written;
                    // TODO stick to a consistent terminology, everywhere: client_content_length = the content length
                    // as communicated to the client, i.e., what the client receives in his headers.
                    // provider_content_length = the content length we send to the provider.
                    let client_content_length = size_written + content_length;
                    let value = format!("{}", client_content_length);
                    xattr::set(path, &key, &value.as_bytes())
                        .expect("Unable to set extended file attributes");
                    println!("Sending content length: {}", client_content_length);
                    let message: FlexoProgress = FlexoProgress::JobSize(client_content_length);
                    let _ = self.job_state.tx.send(message);
                } else if code == 404 && !self.last_chance {
                    self.header_success = Some(HeaderOutcome::Unavailable);
                    println!("Hoping that another provider can fulfil this request…");
                } else if code == 404 && self.last_chance {
                    self.header_success = Some(HeaderOutcome::Unavailable);
                    println!("All providers have been unable to fulfil this request.");
                    let message: FlexoProgress = FlexoProgress::Unavailable;
                    let _ = self.job_state.tx.send(message);
                } else {
                    // TODO handle status codes like 500 etc.
                    unimplemented!("TODO don't know what to do with this status code.")
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
        let file_state = self.handle.get_ref().job_state.job_state.as_ref().unwrap();
        let size_written = file_state.size_written;
        if size_written > 0 {
            Some(size_written)
        } else {
            None
        }
    }

    fn reset_order(&mut self, order: DownloadOrder, tx: Sender<FlexoProgress>) -> Result<(), OrderError> {
        self.handle.get_mut().reset(order, tx)
    }

    fn job_state_item(&mut self) -> &mut JobStateItem<DownloadJob> {
        &mut self.handle.get_mut().job_state
    }
}

pub fn rate_providers(mut mirror_urls: Vec<MirrorUrl>, mirror_config: &MirrorConfig) -> Vec<DownloadProvider> {
    mirror_urls.sort_by(|a, b| a.score.partial_cmp(&b.score).unwrap());
    let filtered_mirror_urls: Vec<MirrorUrl> = mirror_urls
        .into_iter()
        .filter(|x| x.filter_predicate(&mirror_config))
        .take(mirror_config.mirrors_auto.num_mirrors)
        .collect();
    let mut mirrors_with_latencies = Vec::new();
    let timeout = Duration::from_millis(mirror_config.mirrors_auto.timeout);
    for mirror in filtered_mirror_urls.into_iter() {
        match mirror_fetch::measure_latency(&mirror.url, timeout) {
            None => {},
            Some(latency) => {
                mirrors_with_latencies.push((mirror, latency));
            }
        }
    }
    mirrors_with_latencies.sort_unstable_by_key(|(_, latency)| {
        *latency
    });

    mirrors_with_latencies.into_iter().map(|(mirror, mirror_results)| {
        DownloadProvider {
            uri: mirror.url.parse::<Uri>().unwrap(),
            mirror_results,
            country: mirror.country,
        }
    }).collect()
}

pub fn read_client_header<T>(stream: &mut T) -> Result<GetRequest, StreamReadError> where T: Read {
    let mut buf = [0; MAX_HEADER_SIZE + 1];
    let mut size_read_all = 0;

    loop {
        if size_read_all >= MAX_HEADER_SIZE {
            return Err(StreamReadError::BufferSizeExceeded);
        }
        let size = match stream.read(&mut buf[size_read_all..]) {
            Ok(s) if s > MAX_HEADER_SIZE => return Err(StreamReadError::BufferSizeExceeded),
            Ok(s) if s > 0 => s,
            Ok(_) => {
                // we need this branch in case the socket is closed: Otherwise, we would read a size of 0
                // indefinitely.
                return Err(StreamReadError::SocketClosed);
            }
            Err(e) => {
                let error = match e.kind() {
                    ErrorKind::TimedOut => StreamReadError::TimedOut,
                    ErrorKind::WouldBlock => StreamReadError::TimedOut,
                    other => StreamReadError::Other(other),
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
                break(Ok(GetRequest::new(req)))
            }
            Ok(Status::Partial) => {
                unimplemented!()
            }
            Err(_e) => {
                unimplemented!()
            }
        }
    }
}

fn size_to_human_readable(size_in_bytes: u64) -> String {
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
            // has received this exact amount of bytes, or if it has received more than 8192 bytes but the returned value
            // is 8192 because that is the maximum buffer size. So we cautiously assume the latter case and return an error.
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
    struct NoDelimiterReader {
    }
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
        assert_eq!(result, Err(StreamReadError::BufferSizeExceeded));
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
