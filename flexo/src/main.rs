extern crate http;
extern crate rand;
extern crate flexo;

use std::io::prelude::*;
use std::sync::{Arc, Mutex};
use http::Uri;
use flexo::*;
use crate::mirror_config::{MirrorSelectionMethod, MirrorConfig};
use crossbeam::crossbeam_channel::Receiver;
use mirror_flexo::*;
use std::os::unix::io::AsRawFd;

mod mirror_config;
mod mirror_fetch;
mod mirror_cache;
mod mirror_flexo;

use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::Duration;
use std::path::Path;
use std::fs::File;
use crossbeam::crossbeam_channel::RecvTimeoutError;
use std::ffi::OsString;


#[cfg(test)]
use tempfile::tempfile;

// man 2 read: read() (and similar system calls) will transfer at most 0x7ffff000 bytes.
#[cfg(not(test))]
const MAX_SENDFILE_COUNT: usize = 0x7ffff000;

// Choose a smaller size in test, this makes it easier to have fast tests.
#[cfg(test)]
const MAX_SENDFILE_COUNT: usize = 128;


fn main() {
    let job_context: Arc<Mutex<JobContext<DownloadJob>>> = Arc::new(Mutex::new(initialize_job_context()));

    let listener = TcpListener::bind("localhost:7878").unwrap();
    for stream in listener.incoming() {
        let stream: TcpStream = stream.unwrap();
        println!("Established connection with client.");
        stream.set_read_timeout(Some(Duration::from_millis(500))).unwrap();

        let job_context = job_context.clone();

        let _t = thread::spawn(move || {
            serve_file(job_context, stream);
        });
    }
}

fn serve_file(job_context: Arc<Mutex<JobContext<DownloadJob>>>, mut stream: TcpStream) {
    match read_client_header(&mut stream) {
        Ok(get_request) => {
            println!("Got header, GET request is: {:?}", get_request);
            let path = Path::new(PATH_PREFIX).join(&get_request.path);
            let order = DownloadOrder {
                filepath: path.to_str().unwrap().to_owned()
            };
            println!("Attempt to schedule new job");
            let result = job_context.lock().unwrap().schedule(order.clone());
            match result {
                ScheduleOutcome::AlreadyInProgress => {
                    println!("Job is already in progress");
                    // TODO this hasn't been tested yet.
                    let path = Path::new(DIRECTORY).join(&order.filepath);
                    let content_length: u64 = try_content_length_from_path(&path).unwrap();
                    let file: File = File::open(&path).unwrap();
                    serve_from_growing_file(file, content_length, &mut stream);
                }
                ScheduleOutcome::Scheduled(ScheduledItem { join_handle: _, rx: _, rx_progress, }) => {
                    // TODO this branch is also executed when the server returns 404.
                    println!("Job was scheduled, will serve from growing file");
                    match receive_content_length(rx_progress) {
                        Ok(content_length) => {
                            println!("Received content length via channel: {}", content_length);
                            let path = Path::new(DIRECTORY).join(&order.filepath);
                            let file: File = File::open(&path).unwrap();
                            serve_from_growing_file(file, content_length, &mut stream);
                        },
                        Err(ContentLengthError::Unavailable) => {
                            println!("Will send 404 reply to client.");
                            serve_404_header(&mut stream);
                        }
                        Err(e) => {
                            panic!("Error: {:?}", e)
                        },
                    }
                },
                ScheduleOutcome::Cached => {
                    println!("Serve file from cache.");
                    let path = Path::new(DIRECTORY).join(&order.filepath);
                    let file: File = File::open(path).unwrap();
                    serve_from_complete_file(file, &mut stream);
                },
                ScheduleOutcome::Uncacheable(p) => {
                    println!("Serve file via redirect.");
                    let uri_string = format!("{}{}", p.uri, order.filepath);
                    serve_via_redirect(uri_string, &mut stream);
                }
            }
        },
        Err(e) => {
            println!("error: {:?}", e);
        },
    };
}

fn initialize_job_context() -> JobContext<DownloadJob> {
    let mirror_config = mirror_config::load_config();
    let mirrors_auto = mirror_config.mirrors_auto;
    let providers: Vec<DownloadProvider> = fetch_providers(mirror_config);
    println!("{:#?}", providers);
    let urls: Vec<String> = providers.iter().map(|x| x.uri.to_string()).collect();
    mirror_cache::store(&urls);

    JobContext::new(providers, mirrors_auto)
}

fn fetch_providers(mirror_config: MirrorConfig) -> Vec<DownloadProvider> {
    if mirror_config.mirror_selection_method == MirrorSelectionMethod::Auto {
        match mirror_fetch::fetch_providers_from_json_endpoint() {
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
    }
}

#[derive(Debug)]
enum ContentLengthError {
    TransmissionError(RecvTimeoutError),
    Unavailable,
}

fn receive_content_length(rx: Receiver<FlexoProgress>) -> Result<u64, ContentLengthError> {
    loop {
        match rx.recv_timeout(std::time::Duration::from_secs(5)) {
            Ok(FlexoProgress::JobSize(content_length)) => {
                break Ok(content_length);
            }
            Ok(FlexoProgress::Unavailable) => {
                break Err(ContentLengthError::Unavailable);
            }
            Err(e) => break Err(ContentLengthError::TransmissionError(e)),
            Ok(_) => {
                // TODO we're just ignoring whatever is being send on this channel. This is kind of dangerous and
                // not clean: If there is a message on this channel that is not intended for us, than ignoring
                // it will cause problems. If no such messages are sent on the channel, then we wouldn't need
                // this branch.
            },
        }
    }
}

fn try_content_length_from_path(path: &Path) -> Option<u64> {
    let mut num_attempts = 0;
    // Timeout after 2 seconds.
    while num_attempts < 2_000 * 2 {
        match content_length_from_path(path) {
            None => {
                std::thread::sleep(std::time::Duration::from_micros(500));
            },
            Some(v) => return Some(v),
        }
        num_attempts += 1;
    }

    println!("Number of attempts exceeded: File {:?} not found.", &path);
    None
}

fn content_length_from_path(path: &Path) -> Option<u64> {
    let key = OsString::from("user.content_length");
    let value = xattr::get(&path, &key);
    match value {
        Ok(Some(value)) => {
            let content_length = String::from_utf8(value).unwrap().parse::<u64>().unwrap();
            println!("Found file! content length is {}", content_length);
            Some(content_length)
        },
        Ok(None) => {
            println!("file exists, but no content length is set.");
            None
        }
        Err(_) => {
            // println!("file does not exist yet.");
            None
        }
    }
}

fn serve_from_growing_file(mut file: File, content_length: u64, stream: &mut TcpStream) {
    let header = reply_header_success(content_length);
    stream.write(header.as_bytes()).unwrap();
    let mut bytes_sent = 0;
    while bytes_sent < content_length {
        let filesize = file.metadata().unwrap().len();
        if filesize > bytes_sent {
            // TODO note that this while loop runs indefinitely if the file stops growing for whatever reason.
            let result = send_payload(&mut file, filesize, bytes_sent as i64, stream);
            bytes_sent = result.unwrap() as u64;
        }
        if bytes_sent < content_length {
            std::thread::sleep(std::time::Duration::from_micros(500));
        }
    }
    println!("File completely served from growing file.");
}

fn serve_404_header(stream: &mut TcpStream) {
    let header = reply_header_not_found();
    stream.write(header.as_bytes()).unwrap();
    stream.write("\r\n".as_bytes()).unwrap();
}

fn reply_header_success(content_length: u64) -> String {
    reply_header("200 OK", content_length)
}

fn reply_header_not_found() -> String {
    reply_header("404 Not Found", 0)
}

fn reply_header(status_line: &str, content_length: u64) -> String {
    let now = time::now_utc();
    let timestamp = now.rfc822();
    let header = format!("\
        HTTP/1.1 {}\r\n\
        Server: flexo\r\n\
        Date: {}\r\n\
        Content-Length: {}\r\n\r\n", status_line, timestamp, content_length);
    println!("header: {:?}", header);

    return header.to_owned();
}

fn redirect_header(path: &str) -> String {
    let now = time::now_utc();
    let timestamp = now.rfc822();
    let header = format!("\
        HTTP/1.1 301 Moved Permanently\r\n\
        Server: flexo\r\n\
        Date: {}\r\n\
        Location: {}\r\n\r\n", timestamp, path);
    println!("header: {:?}", header);

    return header.to_owned();
}

fn serve_from_complete_file(mut file: File, stream: &mut TcpStream) {
    let filesize = file.metadata().unwrap().len();
    let header = reply_header_success(filesize);
    stream.write(header.as_bytes()).unwrap();
    send_payload(&mut file, filesize, 0, stream).unwrap();
}

fn serve_via_redirect(uri: String, stream: &mut TcpStream) {
    let header = redirect_header(&uri);
    stream.write(header.as_bytes()).unwrap();
}

fn send_payload<T>(source: &mut File, filesize: u64, bytes_sent: i64, receiver: &mut T) -> Result<i64, std::io::Error> where T: AsRawFd {
    let fd = source.as_raw_fd();
    let sfd = receiver.as_raw_fd();
    let size = unsafe {
        let mut offset = bytes_sent;
        while (offset as u64) < filesize {
            libc::sendfile(sfd, fd, &mut offset, MAX_SENDFILE_COUNT);
        }
        offset
    };
    if size == -1 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(size)
    }
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
