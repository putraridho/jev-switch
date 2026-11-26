//! Acceptance and security tests for T05 (DeepSeek balance, DESIGN 6.4, 8.5, 8.7).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::disallowed_types,
    clippy::disallowed_macros
)]

use jev_usage_core::meter::{BalanceReading, Meter};
use jev_usage_core::port::{SourceError, UsageSource};
use jev_usage_core::quantity::{Currency, Money};
use jev_usage_core::secret::Secret;
use jev_usage_core::time::UnixMillis;
use jev_usage_core::usage::{Setup, Unavailable};
use jev_usage_io::clock::FakeClock;
use jev_usage_io::diag;
use jev_usage_io::http::{Endpoint, HttpClient};
use jev_usage_io::secrets::{deepseek_key, KeySources};
use jev_usage_io::sources::deepseek::DeepSeekBalanceSource;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Notify;

const CANARY_KEY: &str = "JEVCANARY_KEY_9f3c1b7e_DO_NOT_SURFACE";

#[derive(Clone, Debug)]
struct RecordedRequest {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
}

#[derive(Clone)]
enum StubMode {
    Static {
        status: u16,
        reason: &'static str,
        extra_headers: Vec<(String, String)>,
        body: Vec<u8>,
    },
    SlowBody {
        gate: Arc<Notify>,
    },
    SlowHeaders {
        gate: Arc<Notify>,
    },
    TruncatedBody,
    StreamOneMibNoContentLength {
        bytes_written: Arc<AtomicUsize>,
    },
}

struct StubServer {
    base: String,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    redirect_hits: Arc<AtomicUsize>,
    total_connections: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}

impl StubServer {
    async fn start(mode: StubMode) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let redirect_hits = Arc::new(AtomicUsize::new(0));
        let total_connections = Arc::new(AtomicUsize::new(0));

        let reqs_bg = Arc::clone(&requests);
        let redir_bg = Arc::clone(&redirect_hits);
        let conns_bg = Arc::clone(&total_connections);

        let task = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                conns_bg.fetch_add(1, Ordering::SeqCst);
                let mode = mode.clone();
                let reqs = Arc::clone(&reqs_bg);
                let redir = Arc::clone(&redir_bg);
                tokio::spawn(async move {
                    handle_connection(stream, mode, reqs, redir).await;
                });
            }
        });

        Self {
            base: format!("http://{addr}"),
            requests,
            redirect_hits,
            total_connections,
            task,
        }
    }

    fn take_requests(&self) -> Vec<RecordedRequest> {
        self.requests.lock().unwrap().clone()
    }

    fn connections(&self) -> usize {
        self.total_connections.load(Ordering::SeqCst)
    }

    fn redirect_target_hits(&self) -> usize {
        self.redirect_hits.load(Ordering::SeqCst)
    }
}

impl Drop for StubServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn read_http_request(stream: &mut TcpStream) -> Option<RecordedRequest> {
    let mut buf = Vec::with_capacity(2048);
    let mut chunk = [0_u8; 1024];
    let header_end = loop {
        let n = stream.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos;
        }
        if buf.len() > 16 * 1024 {
            return None;
        }
    };
    let head = std::str::from_utf8(&buf[..header_end]).ok()?;
    let mut lines = head.split("\r\n");
    let request_line = lines.next()?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next()?.to_owned();
    let path = parts.next()?.to_owned();
    let mut headers = Vec::new();
    for line in lines {
        if let Some((name, val)) = line.split_once(':') {
            headers.push((name.trim().to_ascii_lowercase(), val.trim().to_owned()));
        }
    }
    Some(RecordedRequest {
        method,
        path,
        headers,
    })
}

async fn handle_connection(
    mut stream: TcpStream,
    mode: StubMode,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    redirect_hits: Arc<AtomicUsize>,
) {
    let Some(req) = read_http_request(&mut stream).await else {
        return;
    };
    if req.path == "/redirect-target" {
        redirect_hits.fetch_add(1, Ordering::SeqCst);
        let _ = stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}")
            .await;
        return;
    }
    requests.lock().unwrap().push(req);

    match mode {
        StubMode::Static {
            status,
            reason,
            extra_headers,
            body,
        } => {
            let mut head = format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
                body.len()
            );
            for (k, v) in extra_headers {
                let _ = write!(head, "{k}: {v}\r\n");
            }
            head.push_str("\r\n");
            if stream.write_all(head.as_bytes()).await.is_ok() {
                let _ = stream.write_all(&body).await;
            }
        }
        StubMode::SlowBody { gate } => {
            let head = b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\n{\"is_available\":true";
            if stream.write_all(head).await.is_ok() {
                let _ = stream.flush().await;
                gate.notified().await;
            }
        }
        StubMode::SlowHeaders { gate } => {
            gate.notified().await;
        }
        StubMode::TruncatedBody => {
            let head = b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\n{\"is_available\":true";
            let _ = stream.write_all(head).await;
            let _ = stream.flush().await;
        }
        StubMode::StreamOneMibNoContentLength { bytes_written } => {
            let head =
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n";
            if stream.write_all(head).await.is_err() {
                return;
            }
            let payload = vec![b' '; 8192];
            let chunk_header = format!("{:x}\r\n", payload.len());
            for _ in 0..128 {
                if stream.write_all(chunk_header.as_bytes()).await.is_err()
                    || stream.write_all(&payload).await.is_err()
                    || stream.write_all(b"\r\n").await.is_err()
                    || stream.flush().await.is_err()
                {
                    return;
                }
                bytes_written.fetch_add(payload.len(), Ordering::SeqCst);
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
            let _ = stream.write_all(b"0\r\n\r\n").await;
        }
    }
}

fn source_with(base: &str, clock_ms: u64, key: Option<&str>) -> DeepSeekBalanceSource {
    let http = Arc::new(HttpClient::with_base(base).unwrap());
    let clock = FakeClock::at(clock_ms);
    let owned = key.map(str::to_owned);
    let keys: Arc<dyn Fn() -> KeySources + Send + Sync> = Arc::new(move || KeySources {
        env: owned.clone().map(Secret::new),
        env_file: None,
    });
    DeepSeekBalanceSource::new(http, clock, keys)
}

fn header_value<'a>(req: &'a RecordedRequest, name: &str) -> Option<&'a str> {
    req.headers
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
}

#[tokio::test]
async fn happy_path_single_and_two_currencies_and_wire_headers() {
    let stub = StubServer::start(StubMode::Static {
        status: 200,
        reason: "OK",
        extra_headers: Vec::new(),
        body: include_bytes!("../../../fixtures/deepseek/balance_ok.json").to_vec(),
    })
    .await;

    let src = source_with(&stub.base, 1_700_000_000_123, Some("test-deepseek-key"));
    let reading = src.fetch().await.unwrap();
    assert_eq!(reading.plan, None);
    assert_eq!(reading.observed_at, UnixMillis(1_700_000_000_123));
    assert_eq!(
        reading.meters.as_slice(),
        &[Meter::Balance(BalanceReading {
            currency: Currency::Cny,
            total: Money::parse_decimal("110.00").unwrap(),
            granted: Some(Money::parse_decimal("10.00").unwrap()),
            topped_up: Some(Money::parse_decimal("100.00").unwrap()),
            usable: true,
        })]
    );

    let reqs = stub.take_requests();
    assert_eq!(reqs.len(), 1);
    let req = &reqs[0];
    assert_eq!(req.method, "GET");
    assert_eq!(req.path, "/user/balance");
    assert_eq!(
        header_value(req, "authorization"),
        Some("Bearer test-deepseek-key")
    );
    assert_eq!(header_value(req, "user-agent"), Some("jev-dashboard"));
    assert_eq!(header_value(req, "cookie"), None);
    for (name, _) in &req.headers {
        assert!(
            matches!(
                name.as_str(),
                "host" | "authorization" | "user-agent" | "accept"
            ),
            "unexpected header sent: {name}"
        );
    }

    let two = StubServer::start(StubMode::Static {
        status: 200,
        reason: "OK",
        extra_headers: Vec::new(),
        body: include_bytes!("../../../fixtures/deepseek/balance_two_currencies.json").to_vec(),
    })
    .await;
    let src_two = source_with(&two.base, 1_700_000_000_456, Some("key-two"));
    let reading_two = src_two.fetch().await.unwrap();
    assert_eq!(
        reading_two.meters.as_slice(),
        &[
            Meter::Balance(BalanceReading {
                currency: Currency::Cny,
                total: Money::parse_decimal("110.00").unwrap(),
                granted: Some(Money::parse_decimal("10.00").unwrap()),
                topped_up: Some(Money::parse_decimal("100.00").unwrap()),
                usable: true,
            }),
            Meter::Balance(BalanceReading {
                currency: Currency::Usd,
                total: Money::parse_decimal("4.25").unwrap(),
                granted: Some(Money::parse_decimal("0.00").unwrap()),
                topped_up: Some(Money::parse_decimal("4.25").unwrap()),
                usable: true,
            }),
        ]
    );
}

#[tokio::test]
async fn status_codes_and_transport_failures_map_to_expected_unavailable() {
    let cases = [
        (
            401,
            "Unauthorized",
            include_bytes!("../../../fixtures/deepseek/unauthorized.json").to_vec(),
            Unavailable::Unauthorized,
        ),
        (403, "Forbidden", b"{}".to_vec(), Unavailable::Unauthorized),
        (
            429,
            "Too Many Requests",
            b"{}".to_vec(),
            Unavailable::RateLimited,
        ),
        (
            500,
            "Internal Server Error",
            b"{}".to_vec(),
            Unavailable::ServerError,
        ),
        (502, "Bad Gateway", b"".to_vec(), Unavailable::ServerError),
        (
            503,
            "Service Unavailable",
            b"".to_vec(),
            Unavailable::ServerError,
        ),
        (
            200,
            "OK",
            b"not-json-at-all".to_vec(),
            Unavailable::Protocol,
        ),
        (
            200,
            "OK",
            br#"{"is_available":true,"balance_infos":[]}"#.to_vec(),
            Unavailable::Protocol,
        ),
    ];

    for (status, reason, body, expected) in cases {
        let stub = StubServer::start(StubMode::Static {
            status,
            reason,
            extra_headers: Vec::new(),
            body,
        })
        .await;
        let src = source_with(&stub.base, 1, Some("k"));
        assert_eq!(
            src.fetch().await,
            Err(SourceError::Unavailable(expected)),
            "status {status}"
        );
    }

    let truncated = StubServer::start(StubMode::TruncatedBody).await;
    let src_trunc = source_with(&truncated.base, 1, Some("k"));
    assert_eq!(
        src_trunc.fetch().await,
        Err(SourceError::Unavailable(Unavailable::Protocol))
    );

    // Connect failure on a closed local port -> Offline.
    let closed = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let closed_addr = closed.local_addr().unwrap();
    drop(closed);
    let offline_src = source_with(&format!("http://{closed_addr}"), 1, Some("k"));
    assert_eq!(
        offline_src.fetch().await,
        Err(SourceError::Unavailable(Unavailable::Offline))
    );
}

#[tokio::test]
async fn redirects_are_not_followed() {
    for status in [301, 302, 307, 308] {
        let stub = StubServer::start(StubMode::Static {
            status,
            reason: "Redirect",
            extra_headers: vec![("Location".to_owned(), "/redirect-target".to_owned())],
            body: Vec::new(),
        })
        .await;
        let src = source_with(&stub.base, 1, Some("k"));
        assert_eq!(
            src.fetch().await,
            Err(SourceError::Unavailable(Unavailable::Protocol))
        );
        assert_eq!(stub.redirect_target_hits(), 0, "status {status}");
        assert_eq!(stub.connections(), 1, "status {status}");
    }
}

#[tokio::test]
async fn body_over_64_kib_and_one_mib_are_cut_off() {
    // Exactly 64 KiB passes the HTTP layer.
    let exact_limit = StubServer::start(StubMode::Static {
        status: 200,
        reason: "OK",
        extra_headers: Vec::new(),
        body: vec![b' '; 64 * 1024],
    })
    .await;
    let client_ok = HttpClient::with_base(&exact_limit.base).unwrap();
    let key = Secret::new("k".to_owned());
    let bytes = client_ok
        .get_json(Endpoint::DeepSeekBalance, &key)
        .await
        .unwrap();
    assert_eq!(bytes.len(), 64 * 1024);

    // 64 KiB + 1 byte is rejected with Protocol.
    let one_over = StubServer::start(StubMode::Static {
        status: 200,
        reason: "OK",
        extra_headers: Vec::new(),
        body: vec![b' '; 64 * 1024 + 1],
    })
    .await;
    let src_over = source_with(&one_over.base, 1, Some("k"));
    assert_eq!(
        src_over.fetch().await,
        Err(SourceError::Unavailable(Unavailable::Protocol))
    );

    // 1 MiB chunked stream without Content-Length is cut off and fails with Protocol.
    let written = Arc::new(AtomicUsize::new(0));
    let one_mib = StubServer::start(StubMode::StreamOneMibNoContentLength {
        bytes_written: Arc::clone(&written),
    })
    .await;
    let src_mib = source_with(&one_mib.base, 1, Some("k"));
    assert_eq!(
        src_mib.fetch().await,
        Err(SourceError::Unavailable(Unavailable::Protocol))
    );
    assert!(
        written.load(Ordering::SeqCst) < 1024 * 1024,
        "client did not cut off the 1 MiB stream early: wrote {}",
        written.load(Ordering::SeqCst)
    );
}

#[tokio::test(start_paused = true)]
async fn slow_body_and_slow_headers_hit_timeout() {
    let gate_body = Arc::new(Notify::new());
    let slow_body = StubServer::start(StubMode::SlowBody {
        gate: Arc::clone(&gate_body),
    })
    .await;
    let src_body = source_with(&slow_body.base, 1, Some("k"));
    assert_eq!(
        src_body.fetch().await,
        Err(SourceError::Unavailable(Unavailable::Timeout))
    );
    gate_body.notify_waiters();

    let gate_headers = Arc::new(Notify::new());
    let slow_headers = StubServer::start(StubMode::SlowHeaders {
        gate: Arc::clone(&gate_headers),
    })
    .await;
    let src_headers = source_with(&slow_headers.base, 1, Some("k"));
    assert_eq!(
        src_headers.fetch().await,
        Err(SourceError::Unavailable(Unavailable::Timeout))
    );
    gate_headers.notify_waiters();
}

#[tokio::test]
async fn canary_key_and_canary_payload_never_leak_on_success_or_any_failure() {
    let diag_guard = diag::capture();

    let ok_stub = StubServer::start(StubMode::Static {
        status: 200,
        reason: "OK",
        extra_headers: Vec::new(),
        body: include_bytes!("../../../fixtures/deepseek/balance_canary.json").to_vec(),
    })
    .await;
    let ok_src = source_with(&ok_stub.base, 42, Some(CANARY_KEY));
    let reading = ok_src.fetch().await.unwrap();
    let reading_dbg = format!("{reading:?}");
    assert!(!reading_dbg.contains("JEVCANARY"));
    assert!(!reading_dbg.contains("canary-email"));
    assert!(!reading_dbg.contains("jevcanary"));

    for (status, reason, body) in [
        (401, "Unauthorized", CANARY_KEY.as_bytes().to_vec()),
        (429, "Too Many Requests", CANARY_KEY.as_bytes().to_vec()),
        (500, "Server Error", CANARY_KEY.as_bytes().to_vec()),
        (302, "Found", CANARY_KEY.as_bytes().to_vec()),
        (200, "OK", CANARY_KEY.as_bytes().to_vec()),
        (200, "OK", vec![b'x'; 70 * 1024]),
    ] {
        let stub = StubServer::start(StubMode::Static {
            status,
            reason,
            extra_headers: vec![("X-Echo".to_owned(), CANARY_KEY.to_owned())],
            body,
        })
        .await;
        let src = source_with(&stub.base, 42, Some(CANARY_KEY));
        let err = src.fetch().await.unwrap_err();
        let err_dbg = format!("{err:?}");
        assert!(!err_dbg.contains("JEVCANARY"), "status {status}: {err_dbg}");
    }

    let closed = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let closed_addr = closed.local_addr().unwrap();
    drop(closed);
    let err_offline = source_with(&format!("http://{closed_addr}"), 42, Some(CANARY_KEY))
        .fetch()
        .await
        .unwrap_err();
    assert!(!format!("{err_offline:?}").contains("JEVCANARY"));

    for event in diag_guard.events() {
        let line = diag::line(event);
        assert!(!line.contains("JEVCANARY"));
        assert!(!line.contains("canary-email"));
        assert!(!line.contains("jevcanary"));
    }
}

#[tokio::test]
async fn no_key_makes_zero_connections_and_returns_not_configured() {
    let stub = StubServer::start(StubMode::Static {
        status: 200,
        reason: "OK",
        extra_headers: Vec::new(),
        body: include_bytes!("../../../fixtures/deepseek/balance_ok.json").to_vec(),
    })
    .await;

    let dir = tempfile::tempdir().unwrap();
    let missing_file = dir.path().join("missing.env");
    let empty_file = dir.path().join("empty.env");
    std::fs::write(
        &empty_file,
        "# DEEPSEEK_API_KEY=ignored\nDEEPSEEK_API_KEY=\nexport DEEPSEEK_API_KEY=\"\"\nDEEPSEEK_API_KEY=\"   \"\n",
    )
    .unwrap();

    for file in [None, Some(missing_file), Some(empty_file)] {
        let http = Arc::new(HttpClient::with_base(&stub.base).unwrap());
        let clock = FakeClock::at(10);
        let keys: Arc<dyn Fn() -> KeySources + Send + Sync> = Arc::new(move || KeySources {
            env: None,
            env_file: file.clone(),
        });
        let src = DeepSeekBalanceSource::new(http, clock, keys);
        assert_eq!(
            src.fetch().await,
            Err(SourceError::NotConfigured(Setup::AddDeepSeekKey))
        );
    }

    assert_eq!(stub.connections(), 0);
}

#[tokio::test]
async fn deepseek_key_env_wins_over_file_and_parses_export_quotes_comments() {
    let dir = tempfile::tempdir().unwrap();
    let env_file = dir.path().join("router.env");
    std::fs::write(
        &env_file,
        "# comment\n# DEEPSEEK_API_KEY=commented\nOTHER=123\nexport DEEPSEEK_API_KEY = \"key_from_double_quotes\"\n",
    )
    .unwrap();

    // 1. File alone: parsed with export, quotes, comments.
    let stub = StubServer::start(StubMode::Static {
        status: 200,
        reason: "OK",
        extra_headers: Vec::new(),
        body: include_bytes!("../../../fixtures/deepseek/balance_ok.json").to_vec(),
    })
    .await;

    let http = Arc::new(HttpClient::with_base(&stub.base).unwrap());
    let file_only_path = env_file.clone();
    let src_file = DeepSeekBalanceSource::new(
        Arc::clone(&http),
        FakeClock::at(1),
        Arc::new(move || KeySources {
            env: None,
            env_file: Some(file_only_path.clone()),
        }),
    );
    src_file.fetch().await.unwrap();

    // Single quotes work too.
    let single_quote_file = dir.path().join("single.env");
    std::fs::write(
        &single_quote_file,
        "  DEEPSEEK_API_KEY='key_from_single_quotes'  \n",
    )
    .unwrap();
    let sq_path = single_quote_file.clone();
    let src_sq = DeepSeekBalanceSource::new(
        Arc::clone(&http),
        FakeClock::at(1),
        Arc::new(move || KeySources {
            env: None,
            env_file: Some(sq_path.clone()),
        }),
    );
    src_sq.fetch().await.unwrap();

    // 2. Env wins over file.
    let both_path = env_file.clone();
    let src_env = DeepSeekBalanceSource::new(
        http,
        FakeClock::at(1),
        Arc::new(move || KeySources {
            env: Some(Secret::new("key_from_env".to_owned())),
            env_file: Some(both_path.clone()),
        }),
    );
    src_env.fetch().await.unwrap();

    let reqs = stub.take_requests();
    assert_eq!(reqs.len(), 3);
    assert_eq!(
        header_value(&reqs[0], "authorization"),
        Some("Bearer key_from_double_quotes")
    );
    assert_eq!(
        header_value(&reqs[1], "authorization"),
        Some("Bearer key_from_single_quotes")
    );
    assert_eq!(
        header_value(&reqs[2], "authorization"),
        Some("Bearer key_from_env")
    );

    // Direct unit checks on deepseek_key returning None for missing/empty.
    assert!(deepseek_key(KeySources {
        env: None,
        env_file: Some(dir.path().join("missing.env")),
    })
    .is_none());
    assert!(deepseek_key(KeySources {
        env: None,
        env_file: None,
    })
    .is_none());
}

#[test]
fn secret_exit_and_http_client_crate_exist_only_in_http_rs() {
    let dashboard = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let secret_exit_needle = format!("{}_{}(", "expose", "secret");
    let http_crate_needle = format!("{}{}", "req", "west");

    let mut files = Vec::new();
    collect_rs(&dashboard.join("crates/usage-io/src"), &mut files);
    collect_rs(&dashboard.join("crates/usage-io/tests"), &mut files);
    collect_rs(&dashboard.join("src-tauri/src"), &mut files);

    let mut secret_exit_files = Vec::new();
    let mut http_crate_files = Vec::new();
    for path in files {
        let rel = path
            .strip_prefix(&dashboard)
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned();
        let text = std::fs::read_to_string(&path).unwrap();
        if text.contains(&secret_exit_needle) {
            secret_exit_files.push(rel.clone());
        }
        if text.contains(&http_crate_needle) {
            http_crate_files.push(rel);
        }
    }

    assert_eq!(secret_exit_files, vec!["crates/usage-io/src/http.rs"]);
    assert_eq!(http_crate_files, vec!["crates/usage-io/src/http.rs"]);
}

fn collect_rs(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect_rs(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

// ---- Child-process tests for KeySources::system() and no_proxy() -------------------------

const CHILD_PROBE_ENV: &str = "JEV_T05_CHILD_PROBE";

#[test]
fn child_probe_entrypoint() {
    let Ok(mode) = std::env::var(CHILD_PROBE_ENV) else {
        return;
    };
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    match mode.as_str() {
        "system_keys" => {
            let base = std::env::var("JEV_T05_STUB_BASE").unwrap();
            let http = Arc::new(HttpClient::with_base(&base).unwrap());
            let src =
                DeepSeekBalanceSource::new(http, FakeClock::at(1), Arc::new(KeySources::system));
            let res = rt.block_on(src.fetch());
            match res {
                Ok(_) => println_marker("OK"),
                Err(SourceError::NotConfigured(Setup::AddDeepSeekKey)) => {
                    println_marker("NOT_CONFIGURED");
                }
                Err(other) => panic!("unexpected result: {other:?}"),
            }
        }
        "no_proxy" => {
            let base = std::env::var("JEV_T05_STUB_BASE").unwrap();
            let http = HttpClient::with_base(&base).unwrap();
            let key = Secret::new("proxy_test_key".to_owned());
            let bytes = rt
                .block_on(http.get_json(Endpoint::DeepSeekBalance, &key))
                .unwrap();
            assert!(!bytes.is_empty());
            println_marker("DIRECT_OK");
        }
        other => panic!("unknown child mode: {other}"),
    }
}

fn println_marker(marker: &str) {
    use std::io::Write;
    let _ = std::io::stdout().write_all(format!("MARKER:{marker}\n").as_bytes());
}

async fn run_child(mode: &str, envs: &[(&str, &str)]) -> String {
    let exe = std::env::current_exe().unwrap();
    let mode = mode.to_owned();
    let envs: Vec<(String, String)> = envs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    tokio::task::spawn_blocking(move || {
        let mut cmd = Command::new(exe);
        cmd.arg("--exact")
            .arg("child_probe_entrypoint")
            .arg("--nocapture")
            .env_remove("DEEPSEEK_API_KEY")
            .env_remove("JEV_ROUTER_ENV")
            .env_remove("HOME")
            .env_remove("HTTP_PROXY")
            .env_remove("HTTPS_PROXY")
            .env_remove("ALL_PROXY")
            .env_remove("http_proxy")
            .env_remove("https_proxy")
            .env_remove("all_proxy")
            .env(CHILD_PROBE_ENV, mode);
        for (k, v) in envs {
            cmd.env(k, v);
        }
        let out = cmd.output().unwrap();
        assert!(
            out.status.success(),
            "child failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn system_key_sources_and_no_proxy_verified_in_child_processes() {
    let dir = tempfile::tempdir().unwrap();
    let env_file = dir.path().join("custom.env");
    std::fs::write(&env_file, "export DEEPSEEK_API_KEY='from_system_file'\n").unwrap();

    let stub = StubServer::start(StubMode::Static {
        status: 200,
        reason: "OK",
        extra_headers: Vec::new(),
        body: include_bytes!("../../../fixtures/deepseek/balance_ok.json").to_vec(),
    })
    .await;

    // 1. Env wins over JEV_ROUTER_ENV file.
    let out1 = run_child(
        "system_keys",
        &[
            ("JEV_T05_STUB_BASE", &stub.base),
            ("JEV_ROUTER_ENV", env_file.to_str().unwrap()),
            ("DEEPSEEK_API_KEY", "from_system_env"),
        ],
    )
    .await;
    assert!(out1.contains("MARKER:OK"));

    // 2. Empty env falls back to JEV_ROUTER_ENV file.
    let out2 = run_child(
        "system_keys",
        &[
            ("JEV_T05_STUB_BASE", &stub.base),
            ("JEV_ROUTER_ENV", env_file.to_str().unwrap()),
            ("DEEPSEEK_API_KEY", ""),
        ],
    )
    .await;
    assert!(out2.contains("MARKER:OK"));

    // 3. Unset env and missing file -> NOT_CONFIGURED, zero server connections.
    let missing = dir.path().join("absent.env");
    let out3 = run_child(
        "system_keys",
        &[
            ("JEV_T05_STUB_BASE", &stub.base),
            ("JEV_ROUTER_ENV", missing.to_str().unwrap()),
        ],
    )
    .await;
    assert!(out3.contains("MARKER:NOT_CONFIGURED"));

    let reqs = stub.take_requests();
    assert_eq!(reqs.len(), 2);
    assert_eq!(
        header_value(&reqs[0], "authorization"),
        Some("Bearer from_system_env")
    );
    assert_eq!(
        header_value(&reqs[1], "authorization"),
        Some("Bearer from_system_file")
    );

    // 4. Proxy environment variables are ignored (no_proxy).
    let trap = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let trap_addr = trap.local_addr().unwrap();
    let trap_hits = Arc::new(AtomicUsize::new(0));
    let trap_hits_bg = Arc::clone(&trap_hits);
    let trap_task = tokio::spawn(async move {
        while let Ok((_s, _)) = trap.accept().await {
            trap_hits_bg.fetch_add(1, Ordering::SeqCst);
        }
    });
    let trap_url = format!("http://{trap_addr}");
    let out4 = run_child(
        "no_proxy",
        &[
            ("JEV_T05_STUB_BASE", &stub.base),
            ("HTTP_PROXY", &trap_url),
            ("HTTPS_PROXY", &trap_url),
            ("ALL_PROXY", &trap_url),
            ("http_proxy", &trap_url),
            ("https_proxy", &trap_url),
            ("all_proxy", &trap_url),
        ],
    )
    .await;
    assert!(out4.contains("MARKER:DIRECT_OK"));
    assert_eq!(trap_hits.load(Ordering::SeqCst), 0);
    trap_task.abort();
}
