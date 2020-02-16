extern crate http;
extern crate rand;
extern crate flexo;

use std::io::prelude::*;
use std::sync::{Arc, Mutex};
use http::Uri;
use flexo::*;
use crate::mirror_config::MirrorSelectionMethod;
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
                    match job_context.schedule(order.clone()) {
                        ScheduleOutcome::Skipped(_) => {
                            todo!("what now?")
                        },
                        ScheduleOutcome::Scheduled(_) => {
//                            todo!("download_from_growing_file")
                        },
                        ScheduleOutcome::Cached => {
                            let path = DIRECTORY.to_owned() + &order.filepath;
                            let file: File = File::open(&path).unwrap();
                            serve_file_from_cache(file, &mut stream);
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

fn serve_from_cache(order: DownloadOrder, stream: &mut TcpStream) {
    let file = File::open(&order.filepath).expect(&format!("Unable to open file {:?}", &order.filepath));
    let filesize = file.metadata().unwrap().len();
    let header = reply_header(filesize);
    stream.write(header.as_bytes()).unwrap();
    match send_payload(file, filesize, stream) {
        Ok(_) => {
        }
        Err(_) => {
            unimplemented!()
        },
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
