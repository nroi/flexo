use crate::http_client::{Uri, http_get, http_get_with_header, http_get_with_header_chunked, ChunkPattern};
use std::time::Duration;

mod http_client;

fn main() {
    test_malformed_header();
    test_partial_header();
}

fn test_partial_header() {
    // Sending the header in multiple TCP segments does not cause the server to crash
    let uri = Uri {
        host: "flexo-server-slow-primary".to_owned(),
        path: "/community/os/x86_64/lostfiles-4.03-1-any.pkg.tar.xz".to_owned(),
        port: 7878,
    };
    let malformed_header = "GET /foobar HTTP/1.1".to_owned();
    let pattern = ChunkPattern {
        chunk_size: 3,
        wait_interval: Duration::from_millis(300),
    };
    let result = http_get_with_header_chunked(uri, malformed_header, pattern);
    assert_eq!(result.header_result.status_code, 200);
    println!("test_partial_header: [SUCCESS]")
}

fn test_malformed_header() {
    let uri1 = Uri {
        host: "flexo-server".to_owned(),
        path: "/".to_owned(),
        port: 7878
    };
    let malformed_header = "this is not a valid http header".to_owned();
    let result = http_get_with_header(uri1, malformed_header);
    println!("result: {:?}", &result);
    assert_eq!(result.header_result.status_code, 400);
    // Test if the server is still up, i.e., the previous request hasn't crashed it:
    let uri2 = Uri {
        host: "flexo-server".to_owned(),
        path: "/status".to_owned(),
        port: 7878,
    };
    let result = http_get(uri2);
    println!("result: {:?}", &result);
    assert_eq!(result.header_result.status_code, 200);
    println!("test_malformed_header: [SUCCESS]")
}

