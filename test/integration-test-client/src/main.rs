use crate::http_client::{Uri, http_get, http_get_with_header};

mod http_client;

fn main() {
    let uri = Uri {
        host: "files.xnet.space",
        port: 80u16,
        path: "/random",
    };
    let result = http_get(uri);
    println!("content_length: {:?}", result.header_result.content_length);
    println!("status code: {}", result.header_result.status_code);
    match result.sha256 {
        Some(sha) => {
            let hash = &sha[..];
            println!("hash: {:x?}", hash);
        },
        None => {},
    }
    test_malformed_header();
}

fn test_malformed_header() {
    let uri1 = Uri {
        host: "flexo-server",
        path: "/",
        port: 7878
    };
    let malformed_header = "this is not a valid http header";
    let result = http_get_with_header(uri1, malformed_header);
    println!("result: {:?}", &result);
    assert_eq!(result.header_result.status_code, 400);
    // Test if the server is still up, i.e., the previous request hasn't crashed it:
    let uri2 = Uri {
        host: "flexo-server",
        path: "/status",
        port: 7878,
    };
    let result = http_get(uri2);
    println!("result: {:?}", &result);
    assert_eq!(result.header_result.status_code, 200);
    println!("test_malformed_header: [PASS]")
}

