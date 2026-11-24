use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::thread::{self, JoinHandle};
use std::time::Duration;

pub(crate) struct ReceivedRequest {
    pub request_line: String,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

pub(crate) fn mock_http(
    status: &'static str,
    header: &'static str,
    body: &'static str,
) -> (String, JoinHandle<ReceivedRequest>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut reader = BufReader::new(&mut stream);
        let mut request_line = String::new();
        assert_ne!(reader.read_line(&mut request_line).unwrap(), 0);

        let mut headers = BTreeMap::new();
        loop {
            let mut line = String::new();
            assert_ne!(reader.read_line(&mut line).unwrap(), 0);
            if line == "\r\n" {
                break;
            }
            let (name, value) = line.split_once(':').unwrap();
            headers.insert(name.to_ascii_lowercase(), value.trim().to_owned());
        }

        let length = headers
            .get("content-length")
            .map_or(0, |value| value.parse().unwrap());
        let mut received_body = vec![0; length];
        reader.read_exact(&mut received_body).unwrap();
        write!(
            stream,
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n{header}\r\n{body}",
            body.len()
        )
        .unwrap();
        ReceivedRequest {
            request_line,
            headers,
            body: received_body,
        }
    });
    (url, server)
}
