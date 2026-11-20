//! The gateway's front gate: loopback-only hosts, no browser origins, and a cap on
//! requests in flight that is held until the response body has been read.

use std::sync::Arc;

use anyhow::Result;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, Response, StatusCode};
use axum::middleware::Next;
use futures_util::StreamExt as _;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

#[derive(Clone)]
pub(super) struct GatewayGuard {
    pub(super) loopback: bool,
    pub(super) permits: Arc<Semaphore>,
}

pub(super) fn validate_bind_addresses(addresses: &[std::net::SocketAddr], allow_remote: bool) -> Result<()> {
    anyhow::ensure!(!addresses.is_empty(), "JEV_GATEWAY_ADDR resolved to no addresses");
    anyhow::ensure!(allow_remote || addresses.iter().all(|address| address.ip().is_loopback()),
        "JEV_GATEWAY_ADDR resolves to a non-loopback address; set JEV_GATEWAY_ALLOW_REMOTE=1 to allow remote access");
    Ok(())
}

fn allowed_headers(headers: &HeaderMap, loopback: bool) -> bool {
    if headers.contains_key("origin") {
        return false;
    }
    if !loopback {
        return true;
    }
    let mut hosts = headers.get_all("host").iter();
    let Some(host) = hosts.next() else { return false };
    if hosts.next().is_some() {
        return false;
    }
    host.to_str().ok().and_then(|value| value.parse::<axum::http::uri::Authority>().ok())
        .is_some_and(|authority| {
            let hostname = authority.host();
            let suffix = authority.as_str().strip_prefix(hostname).unwrap_or("invalid");
            matches!(hostname.to_ascii_lowercase().as_str(), "localhost" | "127.0.0.1" | "[::1]")
                && (suffix.is_empty() || suffix.strip_prefix(':').is_some_and(|port| port.parse::<u16>().is_ok()))
        })
}

impl GatewayGuard {
    fn acquire(&self, headers: &HeaderMap) -> std::result::Result<OwnedSemaphorePermit, StatusCode> {
        if !allowed_headers(headers, self.loopback) {
            return Err(StatusCode::FORBIDDEN);
        }
        self.permits.clone().try_acquire_owned().map_err(|_| StatusCode::SERVICE_UNAVAILABLE)
    }
}

fn limit_response(response: Response<Body>, permit: OwnedSemaphorePermit) -> Response<Body> {
    let (parts, body) = response.into_parts();
    let stream = futures_util::stream::unfold((body.into_data_stream(), permit), |(mut stream, permit)| async move {
        stream.next().await.map(|chunk| (chunk, (stream, permit)))
    });
    Response::from_parts(parts, Body::from_stream(stream))
}

pub(super) async fn guard_request(State(guard): State<GatewayGuard>, request: Request, next: Next) -> Response<Body> {
    let permit = match guard.acquire(request.headers()) {
        Ok(permit) => permit,
        Err(status) => return Response::builder().status(status).body(Body::empty()).unwrap(),
    };
    limit_response(next.run(request).await, permit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    #[test]
    fn only_loopback_hosts_without_origin_are_allowed() {
        let mut headers = HeaderMap::new();
        assert!(!allowed_headers(&headers, true));
        for host in ["127.0.0.1", "127.0.0.1:18780", "localhost:443", "LOCALHOST", "[::1]:18780", "[::1]"] {
            headers.insert("host", host.parse().unwrap());
            assert!(allowed_headers(&headers, true), "{host}");
        }
        for host in ["evil.example", "localhost.evil.example", "127.0.0.1.evil.example", "localhost@evil.example", "evil.example@localhost", "127.0.0.2", "localhost:bad", "localhost:"] {
            headers.insert("host", host.parse().unwrap());
            assert!(!allowed_headers(&headers, true), "{host}");
        }
        headers.insert("host", "localhost".parse().unwrap());
        headers.append("host", "localhost".parse().unwrap());
        assert!(!allowed_headers(&headers, true));
        headers.remove("host");
        assert!(allowed_headers(&headers, false));
        headers.insert("origin", "null".parse().unwrap());
        assert!(!allowed_headers(&headers, true));
        assert!(!allowed_headers(&headers, false));
    }

    #[test]
    fn remote_binding_requires_explicit_permission() {
        let local = "127.0.0.1:18780".parse().unwrap();
        let ipv6 = "[::1]:18780".parse().unwrap();
        let remote = "0.0.0.0:18780".parse().unwrap();
        assert!(validate_bind_addresses(&[local, ipv6], false).is_ok());
        assert!(validate_bind_addresses(&[local, remote], false).is_err());
        assert!(validate_bind_addresses(&[remote], true).is_ok());
        assert!(validate_bind_addresses(&[], true).is_err());
    }

    #[tokio::test]
    async fn request_limit_covers_the_response_body_and_releases_on_drop() {
        let guard = GatewayGuard { loopback: true, permits: Arc::new(Semaphore::new(1)) };
        let mut headers = HeaderMap::new();
        headers.insert("host", "localhost".parse().unwrap());
        let permit = guard.acquire(&headers).unwrap();
        assert_eq!(guard.acquire(&headers).unwrap_err(), StatusCode::SERVICE_UNAVAILABLE);
        headers.insert("origin", "null".parse().unwrap());
        assert_eq!(guard.acquire(&headers).unwrap_err(), StatusCode::FORBIDDEN);
        headers.remove("origin");
        let response = limit_response(Response::new(Body::from("stream")), permit);
        assert_eq!(guard.acquire(&headers).unwrap_err(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(to_bytes(response.into_body(), 100).await.unwrap(), "stream");
        let permit = guard.acquire(&headers).unwrap();
        let response = limit_response(Response::new(Body::from("stream")), permit);
        drop(response);
        assert!(guard.acquire(&headers).is_ok());
    }
}
