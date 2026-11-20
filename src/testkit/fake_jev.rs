//! Fake Jev HTTP servers on a loopback port: one that answers each request from a function
//! and records its body, and one that accepts connections and never answers.

use serde_json::Value;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// The bodies of the requests a fake Jev received, in order.
pub type Seen = Arc<Mutex<Vec<Value>>>;

/// Read one HTTP request off `sock` and return its JSON body.
async fn read_json_request(sock: &mut tokio::net::TcpStream) -> Option<Value> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let n = sock.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
        let Some(head_end) = buf.windows(4).position(|w| w == b"\r\n\r\n") else { continue };
        let head = String::from_utf8_lossy(&buf[..head_end]).to_lowercase();
        let len = head.lines().find_map(|l| l.strip_prefix("content-length:")).and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(0);
        if buf.len() >= head_end + 4 + len {
            return serde_json::from_slice(&buf[head_end + 4..head_end + 4 + len]).ok();
        }
    }
}

/// A fake Jev: for each request, `reply` makes (status, body). Returns its URL and
/// the request bodies seen so far.
pub async fn fake_jev(reply: fn(&Value) -> (u16, String)) -> (String, Seen) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1/systemone", listener.local_addr().unwrap());
    let seen: Seen = Arc::default();
    let log = seen.clone();
    tokio::spawn(async move {
        while let Ok((mut sock, _)) = listener.accept().await {
            let log = log.clone();
            tokio::spawn(async move {
                let Some(req) = read_json_request(&mut sock).await else { return };
                let (status, body) = reply(&req);
                log.lock().unwrap().push(req);
                let resp = format!(
                    "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = sock.write_all(resp.as_bytes()).await;
            });
        }
    });
    (url, seen)
}

/// A fake Jev that accepts the connection, then never answers. Returns its URL.
pub async fn stalled_jev() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1/systemone", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((sock, _)) = listener.accept().await {
            held.push(sock);
        }
    });
    url
}
