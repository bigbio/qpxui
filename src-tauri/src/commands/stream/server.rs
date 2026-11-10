//! The proxy's HTTP side: answers DuckDB's HEAD and range GETs for registered
//! objects.

use super::fetch::read_range;
use super::Proxy;
use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::{header, Method, Request, Response, StatusCode};
use std::convert::Infallible;
use std::sync::Arc;

fn cors(b: hyper::http::response::Builder) -> hyper::http::response::Builder {
    b.header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(header::ACCESS_CONTROL_ALLOW_METHODS, "GET, HEAD, OPTIONS")
        .header(header::ACCESS_CONTROL_ALLOW_HEADERS, "Range, Content-Type")
        .header(
            header::ACCESS_CONTROL_EXPOSE_HEADERS,
            "Content-Length, Content-Range, Accept-Ranges, Last-Modified",
        )
        .header("Cross-Origin-Resource-Policy", "cross-origin")
        .header(header::CACHE_CONTROL, "no-store")
}

fn plain(status: StatusCode, msg: impl Into<String>) -> Response<Full<Bytes>> {
    cors(Response::builder().status(status))
        .header(header::CONTENT_TYPE, "text/plain")
        .body(Full::new(Bytes::from(msg.into())))
        .unwrap()
}

/// Parse a single `bytes=` range into an inclusive (start, end).
fn parse_range(h: &str, size: u64) -> Option<(u64, u64)> {
    let spec = h.trim().strip_prefix("bytes=")?.split(',').next()?.trim();
    let (a, b) = spec.split_once('-')?;
    if size == 0 {
        return None;
    }
    let (start, end) = if a.is_empty() {
        let n: u64 = b.parse().ok()?;
        (size.saturating_sub(n), size - 1)
    } else {
        let s: u64 = a.parse().ok()?;
        let e = if b.is_empty() {
            size - 1
        } else {
            b.parse::<u64>().ok()?.min(size - 1)
        };
        (s, e)
    };
    (start <= end && start < size).then_some((start, end))
}

pub(super) async fn handle(
    p: Arc<Proxy>,
    req: Request<Incoming>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    if req.method() == Method::OPTIONS {
        return Ok(cors(Response::builder().status(StatusCode::NO_CONTENT))
            .body(Full::new(Bytes::new()))
            .unwrap());
    }
    let mut parts = req.uri().path().trim_start_matches('/').splitn(2, '/');
    let (token, id) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    if token != p.token {
        return Ok(plain(StatusCode::FORBIDDEN, "bad token"));
    }
    let id = id.split('/').next().unwrap_or("").to_string();
    let Some(obj) = p.objects.lock().unwrap().get(&id).cloned() else {
        return Ok(plain(StatusCode::NOT_FOUND, "unknown object"));
    };

    let range = req
        .headers()
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
        .map(|r| parse_range(r, obj.size));
    if let Some(None) = range {
        return Ok(
            cors(Response::builder().status(StatusCode::RANGE_NOT_SATISFIABLE))
                .header(header::CONTENT_RANGE, format!("bytes */{}", obj.size))
                .body(Full::new(Bytes::new()))
                .unwrap(),
        );
    }
    let range = range.flatten();
    log::debug!(
        "{} {} range={range:?} size={}",
        req.method(),
        obj.path,
        obj.size
    );
    let (start, end) = range.unwrap_or((0, obj.size.saturating_sub(1)));
    let len = if obj.size == 0 { 0 } else { end - start + 1 };

    let mut b = cors(Response::builder())
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(header::CONTENT_LENGTH, len.to_string());
    if let Some(lm) = &obj.last_modified {
        b = b.header(header::LAST_MODIFIED, lm.as_str());
    }
    b = if range.is_some() {
        b.status(StatusCode::PARTIAL_CONTENT).header(
            header::CONTENT_RANGE,
            format!("bytes {start}-{end}/{}", obj.size),
        )
    } else {
        b.status(StatusCode::OK)
    };

    if req.method() == Method::HEAD {
        return Ok(b.body(Full::new(Bytes::new())).unwrap());
    }
    if req.method() != Method::GET {
        return Ok(plain(StatusCode::METHOD_NOT_ALLOWED, "method not allowed"));
    }
    if len == 0 {
        return Ok(b.body(Full::new(Bytes::new())).unwrap());
    }
    match read_range(&p, &id, &obj, start, end).await {
        Ok(data) => Ok(b.body(Full::new(data)).unwrap()),
        Err(e) => {
            log::warn!("stream {}: {e}", obj.path);
            Ok(plain(StatusCode::BAD_GATEWAY, e))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{start, Object, Source};
    use super::*;
    use std::sync::atomic::AtomicU64;

    #[tokio::test]
    async fn serves_ranges_of_a_registered_object_to_token_holders() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("x.parquet");
        std::fs::write(&file, b"0123456789").unwrap();
        let p = start().await.unwrap();
        p.objects.lock().unwrap().insert(
            "id1".into(),
            Arc::new(Object {
                path: "x.parquet".into(),
                size: 10,
                last_modified: None,
                source: Source::Local { path: file },
                fetched: AtomicU64::new(0),
                requests: AtomicU64::new(0),
                layout: Default::default(),
            }),
        );
        let client = reqwest::Client::new();
        let url = |path: &str| format!("http://127.0.0.1:{}/{path}/x.parquet", p.port);
        let get = |path: &str, range: &str| client.get(url(path)).header("Range", range).send();
        let ok = format!("{}/id1", p.token);

        let r = get(&ok, "bytes=2-5").await.unwrap();
        assert_eq!(r.status(), 206);
        assert_eq!(r.headers()["content-range"], "bytes 2-5/10");
        assert_eq!(&r.bytes().await.unwrap()[..], b"2345");
        let head = client.head(url(&ok)).send().await.unwrap();
        assert_eq!(head.status(), 200);
        assert_eq!(head.headers()["content-length"], "10");
        assert_eq!(get(&ok, "bytes=20-30").await.unwrap().status(), 416);
        assert_eq!(get("wrong/id1", "bytes=0-1").await.unwrap().status(), 403);
        let unknown = format!("{}/id2", p.token);
        assert_eq!(get(&unknown, "bytes=0-1").await.unwrap().status(), 404);
    }

    #[test]
    fn range_headers_parse_to_inclusive_bounds() {
        assert_eq!(parse_range("bytes=-3", 10), Some((7, 9)));
        assert_eq!(parse_range("bytes=4-", 10), Some((4, 9)));
        assert_eq!(parse_range("bytes=20-30", 10), None);
    }
}
