use std::time::Duration;
use std::net::{TcpStream, TcpListener};

fn main() {
    let listen_host = std::env::var("TCP_PROXY_DELAY_LISTEN_HOST").unwrap();
    let listen_port = std::env::var("TCP_PROXY_DELAY_LISTEN_PORT").unwrap();
    let listen_pair = format!("{}:{}", listen_host, listen_port);
    let connect_to_host = std::env::var("TCP_PROXY_DELAY_CONNECT_TO_HOST").unwrap();
    let connect_to_port = std::env::var("TCP_PROXY_DELAY_CONNECT_TO_PORT").unwrap();
    let connect_to_pair = format!("{}:{}", connect_to_host, connect_to_port);
    let delay_millisecs = std::env::var("TCP_PROXY_DELAY_MILLISECS").unwrap().parse::<u64>().unwrap();

    let listener = TcpListener::bind(&listen_pair).unwrap();
    println!("Listening for new connections.");
    for stream in listener.incoming() {
        println!("New connection establishment to client");
        let stream: TcpStream = stream.unwrap();
        let connect_to_pair = connect_to_pair.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(delay_millisecs));
            println!("Timeout has elapsed, will establish connection to server.");
            proxy_connection(stream, &connect_to_pair)
        });
    }
}

fn proxy_connection(client_stream: TcpStream, connect_to_pair: &str) {
    println!("Connecting to {}", connect_to_pair);
    let server_stream = TcpStream::connect(connect_to_pair).unwrap();
    {
        let mut server_stream = server_stream.try_clone().unwrap();
        let mut client_stream = client_stream.try_clone().unwrap();
        std::thread::spawn(move || {
            std::io::copy(&mut client_stream, &mut server_stream).unwrap();
        });
    }
    {
        let mut server_stream = server_stream.try_clone().unwrap();
        let mut client_stream = client_stream.try_clone().unwrap();
        std::thread::spawn(move || {
            std::io::copy(&mut server_stream, &mut client_stream).unwrap();
        });
    }
}

