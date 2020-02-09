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
use std::io::{Read, ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::fs::File;

#[cfg(test)]
use tempfile::tempfile;

use std::os::unix::io::AsRawFd;
use httparse::{Status, Header};

// Since a restriction for the size of header fields is also implemented by web servers like NGINX or Apache,
// we keep things simple by just setting a fixed buffer length.
// TODO return 414 (Request-URI Too Large) if this size is exceeded, instead of just panicking.
const MAX_HEADER_SIZE: usize = 8192;

const MAX_HEADER_COUNT: usize = 64;

#[cfg(test)]
const PATH_PREFIX: &str = "./";

#[cfg(not (test))]
const PATH_PREFIX: &str = "./";

#[cfg(test)]
const TEST_CHUNK_SIZE: usize = 128;

#[cfg(test)]
const TEST_REQUEST_HEADER: &[u8] = "GET / HTTP/1.1\r\nHost: www.example.com\r\n\r\n".as_bytes();

// man 2 read: read() (and similar system calls) will transfer at most 0x7ffff000 bytes.
#[cfg(not(test))]
const MAX_SENDFILE_COUNT: usize = 0x7ffff000;

// Choose a smaller size in test, this makes it easier to have fast tests.
#[cfg(test)]
const MAX_SENDFILE_COUNT: usize = 128;

#[derive(Debug, PartialEq, Eq)]
enum StreamReadError {
    BufferSizeExceeded,
    TimedOut,
    SocketClosed,
    Other(ErrorKind)
}

#[derive(Debug, PartialEq, Eq)]
struct GetRequest {
    path: PathBuf,
}

impl GetRequest {
    fn new(request: httparse::Request) -> Self {
        match request.method {
            Some("GET") => {},
            method => panic!("Unexpected method: #{:?}", method)
        }
        let p = &request.path.unwrap()[1..]; // Skip the leading "/"
        let path = Path::new(p).to_path_buf();
        Self {
            path,
        }
    }
}

fn reply_header(content_length: u64) -> String {
    let now = time::now_utc();
    let timestamp = now.rfc822();
    let header = format!("\
        HTTP/1.1 200 OK\r\n\
        Server: webserver_test\r\n\
        Date: {}\r\n\
        Content-Length: {}\r\n\r\n", timestamp, content_length);
    println!("header: {:?}", header);

    return header.to_owned();
}

fn read_header<T>(stream: &mut T) -> Result<GetRequest, StreamReadError> where T: Read {
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

        println!("size: {:?}", size);
        println!("buf: {:?}", &buf[0..31]);

        let mut headers: [Header; 64] = [httparse::EMPTY_HEADER; MAX_HEADER_COUNT];
        let mut req: httparse::Request = httparse::Request::new(&mut headers);
        let res: std::result::Result<httparse::Status<usize>, httparse::Error> = req.parse(&buf[..size_read_all]);

        match res {
            Ok(Status::Complete(result)) => {
                println!("done! {:?}", result);
                println!("req: {:?}", req);
                break(Ok(GetRequest::new(req)))
            }
            Ok(Status::Partial) => {
                println!("partial");
            }
            Err(e) => {
                println!("error: #{:?}", e)
            }
        }
    }
}

fn send_payload<T>(source: File, filesize: u64, receiver: &mut T) -> Result<i64, std::io::Error>  where T: AsRawFd {
    let fd = source.as_raw_fd();
    let sfd = receiver.as_raw_fd();
    let size = unsafe {
        let mut bytes_sent: i64 = 0;
        while (bytes_sent as u64) < filesize {
            libc::sendfile(sfd, fd, &mut bytes_sent, MAX_SENDFILE_COUNT);
        }
        bytes_sent
    };
    if size == -1 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(size)
    }
}

fn serve_file_from_cache(file: File,  stream: &mut TcpStream) {
    let filesize = file.metadata().unwrap().len();
    let header = reply_header(filesize);
    stream.write(header.as_bytes()).unwrap();
    send_payload(file, filesize, stream).unwrap();
}

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
                    job_context.schedule(order);
                },
                Err(e) => {
                    println!("error: {:?}", e);
                },
            };
            stream.shutdown(Shutdown::Both).unwrap();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Error, Write};

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
    impl OneByteReader {
        fn new() -> Self {
            OneByteReader {
                size_read: 0,
            }
        }
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
    impl NoDelimiterReader {
        fn new() -> Self {
            NoDelimiterReader {}
        }
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
        let result = read_header(&mut stream);
        assert_eq!(result, Err(StreamReadError::BufferSizeExceeded));
    }

    #[test]
    fn test_one_by_one() {
        let mut stream = OneByteReader::new();
        let result = read_header(&mut stream);
        println!("result: {:?}", result);
        assert!(result.is_ok());
    }

    #[test]
    fn test_no_delimiter() {
        let mut stream = NoDelimiterReader::new();
        let result = read_header(&mut stream);
        assert_eq!(result, Err(StreamReadError::BufferSizeExceeded));
    }

    #[test]
    fn test_filesize_exceeds_sendfile_count() {
        let mut source: File = tempfile().unwrap();
        let mut receiver: File = tempfile().unwrap();
        let array: [u8; MAX_SENDFILE_COUNT * 3] = [b'a'; MAX_SENDFILE_COUNT * 3];
        source.write(&array).unwrap();
        source.flush().unwrap();
        let filesize = source.metadata().unwrap().len();
        let size = send_payload(source, filesize, &mut receiver).unwrap();
        assert_eq!(size, (MAX_SENDFILE_COUNT * 3) as i64);
    }
}
