extern crate http;
extern crate rand;
extern crate flexo;

use std::io::prelude::*;
use std::sync::{Arc, Mutex};
use http::Uri;
use flexo::*;
use crate::mirror_config::MirrorSelectionMethod;
use crossbeam::crossbeam_channel::Receiver;
use mirror_flexo::*;
use std::os::unix::io::AsRawFd;

mod mirror_config;
mod mirror_fetch;
mod mirror_cache;
mod mirror_flexo;

use std::net::{TcpListener, Shutdown, TcpStream};
use std::thread;
use std::time::Duration;
use std::path::Path;
use std::fs::File;

#[cfg(test)]
use tempfile::tempfile;

// man 2 read: read() (and similar system calls) will transfer at most 0x7ffff000 bytes.
#[cfg(not(test))]
const MAX_SENDFILE_COUNT: usize = 0x7ffff000;

// Choose a smaller size in test, this makes it easier to have fast tests.
#[cfg(test)]
const MAX_SENDFILE_COUNT: usize = 128;


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

    let job_context: JobContext<DownloadJob> = JobContext::new(providers, mirror_config.mirrors_auto);
    let job_context: Arc<Mutex<JobContext<DownloadJob>>> = Arc::new(Mutex::new(job_context));

    let listener = TcpListener::bind("localhost:7878").unwrap();
    for stream in listener.incoming() {
        let mut stream: TcpStream = stream.unwrap();
        println!("connection established!");
        stream.set_read_timeout(Some(Duration::from_millis(500))).unwrap();

        let job_context = job_context.clone();
        let _t = thread::spawn(move || {
            match read_header(&mut stream) {
                Ok(get_request) => {
                    let path = Path::new(PATH_PREFIX).join(&get_request.path);
                    let order = DownloadOrder {
                        filepath: path.to_str().unwrap().to_owned()
                    };
                    let mut job_context = job_context.lock().unwrap();
                    let result = job_context.schedule(order.clone());
                    // This is a new concept to be introduced to the flexo library: The job size. The library has to
                    // be able to tell us the job size if the job has to be fetched from the provider.
                    match result {
                        ScheduleOutcome::Skipped(_) => {
                            todo!("what now?")
                        },
                        ScheduleOutcome::Scheduled(ScheduledItem { join_handle: _, rx: _, rx_progress, }) => {
                            let content_length = receive_content_length(rx_progress);
                            let path = DIRECTORY.to_owned() + &order.filepath;
                            let file: File = File::open(&path).unwrap();
                            serve_from_growing_file(file, content_length, &mut stream);
                        },
                        ScheduleOutcome::Cached => {
                            let path = DIRECTORY.to_owned() + &order.filepath;
                            let file: File = File::open(Path::new(DIRECTORY).join(&path)).unwrap();
                            serve_from_complete_file(file, &mut stream);
                        }
                    }
//                    stream.shutdown(Shutdown::Both).unwrap();
                },
                Err(e) => {
                    println!("error: {:?}", e);
                },
            };
        });
    }
}

fn receive_content_length(rx: Receiver<FlexoProgress>) -> u64 {
    loop {
        match rx.recv() {
            Ok(FlexoProgress::JobSize(content_length)) => {
                break content_length;
            }
            Ok(_) => {},
            Err(_) => {},
        }
    }
}

fn serve_from_growing_file(mut file: File, content_length: u64, stream: &mut TcpStream) {
    let header = reply_header(content_length);
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

fn serve_from_complete_file(mut file: File, stream: &mut TcpStream) {
    let filesize = file.metadata().unwrap().len();
    let header = reply_header(filesize);
    stream.write(header.as_bytes()).unwrap();
    send_payload(&mut file, filesize, 0, stream).unwrap();
}

fn send_payload<T>(source: &mut File, filesize: u64, bytes_sent: i64, receiver: &mut T) -> Result<i64, std::io::Error> where T: AsRawFd {
    let fd = source.as_raw_fd();
    let sfd = receiver.as_raw_fd();
    let size = unsafe {
        let mut offset = bytes_sent;
        while (offset as u64) < filesize {
            libc::sendfile(sfd, fd, &mut offset, MAX_SENDFILE_COUNT);
        }
        println!("offset: {}", offset);
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
