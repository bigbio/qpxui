//! Lazy remote access for DuckDB-wasm (S3 buckets and the quantms portal).
//!
//! A tiny HTTP server bound to 127.0.0.1 serves registered remote files with
//! HTTP Range support. DuckDB (running in the webview) treats each object as a
//! remote parquet file and only requests the byte ranges a query needs (footer,
//! then the column chunks / row groups it touches). Each range is fetched from
//! S3 with the backend's AWS credentials (or over HTTPS for public portal
//! files) and kept in an in-memory block cache,
//! so:
//!   * nothing is downloaded up front, however large the object is;
//!   * the bucket needs no CORS configuration and credentials never reach the
//!     webview;
//!   * re-reading the same pages (paging, re-sorting) is served from memory.
//!
//! URLs carry a random per-session token so other local processes can't use
//! the proxy to read the user's buckets.
//!
//! - [`register`]: registering files with the proxy, and the commands;
//! - `server`: the HTTP side, answering DuckDB's range requests;
//! - `fetch`: reading ranges from S3, HTTPS or disk through the block cache.

mod fetch;
mod readahead;
pub mod register;
mod server;

use aws_sdk_s3::Client;
use bytes::Bytes;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use lru::LruCache;
use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;
use tokio::sync::OnceCell;

/// In-memory cache budget (in blocks) shared by all streamed objects:
/// 8192 x 64 KB = 512 MB.
const CACHE_BLOCKS: usize = 8192;

/// A range fetch in flight, shared by every request that needs its blocks.
type Pending = futures::future::Shared<futures::future::BoxFuture<'static, Result<Bytes, String>>>;

/// Where a streamed object's bytes come from.
enum Source {
    /// An S3 object read with the user's AWS credentials.
    S3 {
        client: Client,
        bucket: String,
        key: String,
    },
    /// A public HTTPS URL (e.g. the quantms portal). The portal answers with a
    /// redirect to a short-lived signed URL on S3-compatible storage; we keep
    /// the resolved URL and re-resolve when it expires.
    Http {
        client: reqwest::Client,
        url: String,
        resolved: Mutex<Option<String>>,
    },
    /// A local parquet file (or one already downloaded to the cache). Served
    /// straight from disk, so DuckDB reads only the ranges it needs instead of
    /// the whole file being copied into the webview.
    Local { path: std::path::PathBuf },
}

struct Object {
    /// The app-facing path (`s3://bucket/key` or the portal https URL).
    path: String,
    size: u64,
    last_modified: Option<String>,
    source: Source,
    fetched: AtomicU64,
    requests: AtomicU64,
    /// Column chunk layout from the parquet footer, read on first use.
    layout: OnceCell<Option<Arc<readahead::Layout>>>,
}

struct Proxy {
    port: u16,
    token: String,
    objects: Mutex<HashMap<String, Arc<Object>>>,
    cache: Mutex<LruCache<(String, u64), Bytes>>,
    /// Blocks being fetched: block -> (first block of its run, the fetch).
    inflight: Mutex<HashMap<(String, u64), (u64, Pending)>>,
}

static PROXY: OnceCell<Arc<Proxy>> = OnceCell::const_new();

fn random_token() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    let mut out = String::new();
    for i in 0..2u64 {
        let mut h = RandomState::new().build_hasher();
        h.write_u64(i);
        h.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default(),
        );
        out.push_str(&format!("{:016x}", h.finish()));
    }
    out
}

/// The app's proxy, started on first use.
async fn proxy() -> Result<Arc<Proxy>, String> {
    PROXY.get_or_try_init(start).await.cloned()
}

/// Bind a proxy to a free local port and serve it on the current runtime.
async fn start() -> Result<Arc<Proxy>, String> {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .map_err(|e| format!("stream proxy bind: {e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let p = Arc::new(Proxy {
        port,
        token: random_token(),
        objects: Mutex::new(HashMap::new()),
        cache: Mutex::new(LruCache::new(NonZeroUsize::new(CACHE_BLOCKS).unwrap())),
        inflight: Mutex::new(HashMap::new()),
    });
    let served = p.clone();
    tokio::spawn(async move {
        loop {
            let stream = match listener.accept().await {
                Ok((stream, _)) => stream,
                Err(e) => {
                    // E.g. out of file descriptors: the connection
                    // stays queued, so retrying at once would spin.
                    log::warn!("stream proxy accept: {e}");
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    continue;
                }
            };
            let p = served.clone();
            tokio::spawn(async move {
                let svc = service_fn(move |req| server::handle(p.clone(), req));
                let _ = http1::Builder::new()
                    .keep_alive(true)
                    .serve_connection(TokioIo::new(stream), svc)
                    .await;
            });
        }
    });
    log::info!("S3 stream proxy listening on 127.0.0.1:{port}");
    Ok(p)
}
