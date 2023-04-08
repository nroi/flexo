use std::time::SystemTime;

pub fn reply_header_success(content_length: u64, payload_origin: PayloadOrigin) -> String {
    reply_header("200 OK", content_length, None, payload_origin, SystemTime::now())
}

pub fn reply_header_partial(content_length: u64, resume_from: u64, payload_origin: PayloadOrigin) -> String {
    reply_header(
        "206 Partial Content", content_length, Some(resume_from), payload_origin, SystemTime::now()
    )
}

pub fn reply_header_not_found() -> String {
    reply_header("404 Not Found", 0, None, PayloadOrigin::NoPayload, SystemTime::now())
}

pub fn reply_header_bad_request() -> String {
    reply_header("400 Bad Request", 0, None, PayloadOrigin::NoPayload, SystemTime::now())
}

pub fn reply_header_internal_server_error() -> String {
    reply_header("500 Internal Server Error", 0, None, PayloadOrigin::NoPayload, SystemTime::now())
}

pub fn reply_header_forbidden() -> String {
    reply_header("403 Forbidden", 0, None, PayloadOrigin::NoPayload, SystemTime::now())
}

fn reply_header(
    status_line: &str,
    content_length: u64,
    resume_from: Option<u64>,
    payload_origin: PayloadOrigin,
    now: SystemTime,
) -> String {
    let timestamp = httpdate::fmt_http_date(now);
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

pub fn redirect_header(path: &str, now: SystemTime) -> String {
    let timestamp = httpdate::fmt_http_date(now);
    let header = format!("\
        HTTP/1.1 301 Moved Permanently\r\n\
        Server: flexo\r\n\
        Date: {}\r\n\
        Content-Length: 0\r\n\
        Location: {}\r\n\r\n", timestamp, path);

    header
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum PayloadOrigin {
    Cache,
    RemoteMirror,
    NoPayload,
}

#[test]
fn test_reply_header() {
    let timestamp = httpdate::parse_http_date("Thu, 06 Apr 2023 20:00:18 GMT").unwrap();
    let expected = "HTTP/1.1 OK\r\n\
        Server: flexo\r\n\
        Date: Thu, 06 Apr 2023 20:00:18 GMT\r\n\
        Flexo-Payload-Origin: NoPayload\r\n\
        Content-Length: 0\r\n\r\n";
    let actual = reply_header("OK", 0, None, PayloadOrigin::NoPayload, timestamp);

    assert_eq!(expected, actual)
}

#[test]
fn test_redirect_header() {
    let timestamp = httpdate::parse_http_date("Thu, 06 Apr 2023 20:00:18 GMT").unwrap();
    let expected = "HTTP/1.1 301 Moved Permanently\r\n\
        Server: flexo\r\n\
        Date: Thu, 06 Apr 2023 20:00:18 GMT\r\n\
        Content-Length: 0\r\n\
        Location: /new/location\r\n\r\n";
    let actual = redirect_header("/new/location", timestamp);

    assert_eq!(expected, actual)
}
