use std::io::prelude::*;
use std::io::BufReader;
use std::net::{TcpStream};
use sha2::{Sha256, Digest};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

pub struct  Uri {
    pub host: String,
    pub path: String,
    pub port: u16,
}

const BUF_SIZE: usize = 4096;

const HEADER_SEPARATOR: &[u8; 4] = b"\r\n\r\n";
const HEADER_SEPARATOR_STR: &str = "\r\n\r\n";

#[derive(Debug, Hash, Clone, PartialEq, Eq)]
pub struct HeaderResult {
    pub status_code: u32,
    pub content_length: usize,
}

#[derive(Debug, Hash, Clone, PartialEq, Eq)]
pub struct HttpGetResult {
    pub header_result: HeaderResult,
    pub sha256: Option<Vec<u8>>,
}

pub fn http_get(uri: Uri) -> HttpGetResult {
    let header = format!("GET {} HTTP/1.1\r\nHost: {}{}", uri.path, uri.host, HEADER_SEPARATOR_STR);
    http_get_with_header(uri, header)
}

pub fn http_get_with_header(uri: Uri, header: String) -> HttpGetResult {
    let (_, receiver) = mpsc::channel::<HttpGetResult>();
    thread::spawn(move || {
        let mut stream = TcpStream::connect((uri.host, uri.port)).unwrap();
        stream.write(header.as_bytes()).unwrap();
        let mut reader = BufReader::new(stream);
        let header_result = read_header(&mut reader);
        let sha256 = match header_result.content_length {
            0 => None,
            content_length => Some(sha256(&mut reader, content_length)),
        };
        HttpGetResult {
            header_result,
            sha256,
        }
    });
    match receiver.recv_timeout(Duration::from_millis(5000)) {
        Ok(r) => r,
        Err(e) => panic!("HTTP GET request timeout: {:?}", e),
    }
}

fn read_header(reader: &mut BufReader<TcpStream>) -> HeaderResult {
    let payload = &mut[0; BUF_SIZE];
    let mut size_read = 0;
    loop {
        match reader.read(&mut payload[size_read..size_read + 1]) {
            Ok(1) => {
                size_read += 1;
                if size_read >= HEADER_SEPARATOR.len() {
                    if &payload[size_read - HEADER_SEPARATOR.len()..size_read] == HEADER_SEPARATOR {
                        break;
                    }
                }
            },
            Ok(s) => panic!("Unexpected size while reading from socket: {}", s),
            Err(e) => panic!("Unable to read header: {:?}", e),
        }
    }
    let header_bytes = &payload[..size_read - HEADER_SEPARATOR.len()];
    let content_length = content_length(header_bytes);
    let status_code = status_code(header_bytes);
    HeaderResult {
        status_code,
        content_length,
    }
}

fn sha256(reader: &mut BufReader<TcpStream>, content_length: usize) -> Vec<u8> {
    let mut hasher = Sha256::new();
    let payload = &mut[0; BUF_SIZE];
    let mut size_read = 0;
    while size_read < content_length {
        match reader.read(payload) {
            Ok(size) => {
                size_read += size;
                hasher.update(&payload[..size]);
            }
            Err(e) => panic!("Unable to read header: {:?}", e),
        }
    }
    hasher.finalize().to_vec()
}

fn content_length(header: &[u8]) -> usize {
    let keyword = b"Content-Length: ";
    let start_idx = header
        .windows(keyword.len())
        .position(|header_part| header_part == keyword).unwrap() + keyword.len();
    let end_idx = header[start_idx..]
        .iter()
        .position(|header_part| header_part == &b'\r').map(|i| i + start_idx)
        .unwrap_or(header.len());
    let content_length = &header[start_idx..end_idx];
    std::string::String::from_utf8(Vec::from(content_length)).unwrap().parse::<usize>().unwrap()
}

fn status_code(header: &[u8]) -> u32 {
    let keyword = b" ";
    let start_idx = header
        .iter()
        .position(|header_part| header_part == &b' ')
        .unwrap() + keyword.len();
    let end_idx = header[start_idx..]
        .iter()
        .position(|header_part| header_part == &b' ')
        .unwrap() + start_idx;
    let status_code = &header[start_idx..end_idx];
    std::string::String::from_utf8(Vec::from(status_code)).unwrap().parse::<u32>().unwrap()
}
