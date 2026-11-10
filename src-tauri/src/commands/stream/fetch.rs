//! Reading byte ranges from an object's source (S3, the portal over HTTPS,
//! or a local file), through the shared block cache for remote objects.

use super::{Object, Pending, Proxy, Source};
use bytes::Bytes;
use hyper::header;
use std::collections::BTreeMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

/// Cache granularity. Requests are widened to whole blocks so neighbouring
/// small reads (parquet footers, page headers) share one round trip. Kept
/// small: a narrow column of a file with many row groups is read as many
/// chunks of a few KB each, and every one of them costs a whole block.
pub(super) const BLOCK: u64 = 64 * 1024;
/// Longest single range request (in blocks): 64 x 64 KB = 4 MB.
const CHUNK_BLOCKS: u64 = 64;
/// Range requests issued concurrently for one large read.
const PARALLEL_FETCHES: usize = 8;
/// Range requests issued concurrently by one read-ahead.
pub(super) const PREFETCH_FETCHES: usize = 8;

/// Follow the portal's redirect once to get the signed storage URL.
async fn resolve(client: &reqwest::Client, url: &str) -> Result<String, String> {
    let resp = client
        .get(url)
        .header(header::RANGE.as_str(), "bytes=0-0")
        .send()
        .await
        .map_err(|e| format!("resolve {url}: {e}"))?;
    if resp.status().is_redirection() {
        if let Some(loc) = resp
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|v| v.to_str().ok())
        {
            // Location may be relative to the requested URL.
            return reqwest::Url::parse(url)
                .and_then(|base| base.join(loc))
                .map(|u| u.to_string())
                .map_err(|e| format!("resolve {url}: bad Location {loc}: {e}"));
        }
    }
    if resp.status().is_success() {
        return Ok(url.to_string());
    }
    Err(format!("resolve {url}: HTTP {}", resp.status()))
}

/// Transient HTTP statuses worth retrying a range fetch for. 5xx are server
/// hiccups, 429/408/425 are throttling / timeout — all likely to clear on retry.
fn is_transient_status(st: u16) -> bool {
    matches!(st, 408 | 425 | 429 | 500 | 502 | 503 | 504)
}

/// Short backoff between retries: ~250 ms then ~500 ms. Long enough for the
/// upstream to clear a transient 502/503, short enough not to stall the query.
fn backoff_delay(attempt: usize) -> std::time::Duration {
    std::time::Duration::from_millis(match attempt {
        0 => 250,
        _ => 500,
    })
}

/// One ranged GET against an HTTP source, with bounded retries.
///
/// Two failure modes are handled separately:
///   * the signed URL was rejected/expired (400/401/403) -> re-resolve it and
///     try again;
///   * a transient failure (network/send error, or 408/425/429/5xx such as a
///     remote gateway 502) -> retry with a short backoff, up to a couple of
///     attempts, before giving up.
pub(super) async fn http_range(
    client: &reqwest::Client,
    url: &str,
    resolved: &Mutex<Option<String>>,
    range: &str,
) -> Result<reqwest::Response, String> {
    const MAX_ATTEMPTS: usize = 3; // initial + 2 retries
    let mut attempt = 0;
    loop {
        // Use the cached signed URL on the first try, otherwise resolve fresh.
        let cached = resolved.lock().unwrap().clone();
        let target = match cached {
            Some(u) if attempt == 0 => u,
            _ => {
                let u = resolve(client, url).await?;
                *resolved.lock().unwrap() = Some(u.clone());
                u
            }
        };
        let retry = attempt + 1 < MAX_ATTEMPTS;

        let send = client
            .get(&target)
            .header(header::RANGE.as_str(), range)
            .send()
            .await;

        match send {
            // Transport-level failure (timeout / connection reset): transient.
            Err(_) if retry => tokio::time::sleep(backoff_delay(attempt)).await,
            Err(e) => {
                return Err(format!(
                    "range get {url}: {e} after {MAX_ATTEMPTS} attempts"
                ))
            }
            Ok(resp) => {
                let st = resp.status().as_u16();
                if (st == 403 || st == 400 || st == 401) && attempt == 0 {
                    // Expired / rejected signed URL: drop the cached URL and
                    // resolve fresh.
                    *resolved.lock().unwrap() = None;
                } else if resp.status().is_success() {
                    return Ok(resp);
                } else if is_transient_status(st) && retry {
                    // Retry transient statuses, but fail fast on permanent
                    // client errors (e.g. 404) that retrying will never fix.
                    tokio::time::sleep(backoff_delay(attempt)).await;
                } else {
                    return Err(format!("range get {url}: HTTP {st}"));
                }
            }
        }
        attempt += 1;
    }
}

/// Fetch one inclusive byte range from the object's source.
async fn fetch(obj: &Object, start: u64, end: u64) -> Result<Bytes, String> {
    let t0 = std::time::Instant::now();
    let r = fetch_source(obj, start, end).await;
    let ms = t0.elapsed().as_millis();
    match &r {
        // A slow fetch is worth seeing without detailed logging.
        Ok(d) if ms >= 3000 => log::info!(
            "slow fetch {} bytes {start}-{end} ({} KB): {ms} ms",
            obj.path,
            d.len() / 1024
        ),
        Ok(d) => log::debug!(
            "fetched {} bytes {start}-{end} ({} KB) in {ms} ms",
            obj.path,
            d.len() / 1024
        ),
        Err(e) => log::warn!(
            "fetch {} bytes {start}-{end} failed after {ms} ms: {e}",
            obj.path
        ),
    }
    r
}

async fn fetch_source(obj: &Object, start: u64, end: u64) -> Result<Bytes, String> {
    let range = format!("bytes={start}-{end}");
    let data = match &obj.source {
        Source::S3 {
            client,
            bucket,
            key,
        } => {
            let out = client
                .get_object()
                .bucket(bucket)
                .key(key)
                .range(range)
                .send()
                .await
                .map_err(|e| {
                    format!(
                        "S3 range get {key}: {}",
                        aws_sdk_s3::error::DisplayErrorContext(e)
                    )
                })?;
            out.body
                .collect()
                .await
                .map_err(|e| e.to_string())?
                .into_bytes()
        }
        Source::Local { path } => {
            let path = path.clone();
            tokio::task::spawn_blocking(move || read_local(&path, start, end))
                .await
                .map_err(|e| e.to_string())??
        }
        Source::Http {
            client,
            url,
            resolved,
        } => {
            let resp = http_range(client, url, resolved, &range).await?;
            let full = resp.status().as_u16() == 200;
            let body = resp.bytes().await.map_err(|e| format!("read {url}: {e}"))?;
            if full {
                // Server ignored Range; keep only the requested slice.
                let s = (start as usize).min(body.len());
                let e = ((end + 1) as usize).min(body.len());
                body.slice(s..e)
            } else {
                body
            }
        }
    };
    obj.fetched.fetch_add(data.len() as u64, Ordering::Relaxed);
    obj.requests.fetch_add(1, Ordering::Relaxed);
    Ok(data)
}

fn read_local(path: &std::path::Path, start: u64, end: u64) -> Result<Bytes, String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).map_err(|e| format!("open {}: {e}", path.display()))?;
    f.seek(SeekFrom::Start(start)).map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; (end - start + 1) as usize];
    f.read_exact(&mut buf)
        .map_err(|e| format!("read {}: {e}", path.display()))?;
    Ok(Bytes::from(buf))
}

/// Serve [start, end] to DuckDB from the block cache, fetching what is
/// missing, then read ahead the next chunks of the columns it read.
pub(super) async fn read_range(
    p: &Arc<Proxy>,
    id: &str,
    obj: &Arc<Object>,
    start: u64,
    end: u64,
) -> Result<Bytes, String> {
    // Local files are already one seek away (and in the OS page cache); keep
    // the block cache for remote objects.
    if matches!(obj.source, Source::Local { .. }) {
        return fetch(obj, start, end).await;
    }
    let data = read_blocks(p, id, obj, start, end, true)
        .await?
        .ok_or_else(|| format!("short read for {}", obj.path))?;
    super::readahead::read_ahead(p, id, obj, start, end);
    Ok(data)
}

/// Make the blocks covering [start, end] available and, when `assemble`,
/// return those bytes. Cached blocks are reused, blocks another request is
/// already fetching are waited for, and the rest is fetched in as few
/// contiguous range requests as possible, several at once.
pub(super) async fn read_blocks(
    p: &Arc<Proxy>,
    id: &str,
    obj: &Arc<Object>,
    start: u64,
    end: u64,
    assemble: bool,
) -> Result<Option<Bytes>, String> {
    let (first, last) = (start / BLOCK, end / BLOCK);
    let mut blocks: BTreeMap<u64, Bytes> = BTreeMap::new();
    let mut waits: Vec<(u64, Pending)> = Vec::new();
    {
        // Lock order: cache, then in-flight (as in `pending_run`).
        let mut cache = p.cache.lock().unwrap();
        let mut inflight = p.inflight.lock().unwrap();
        let mut missing: Vec<u64> = Vec::new();
        for b in first..=last {
            let key = (id.to_string(), b);
            if let Some(d) = cache.get(&key) {
                if assemble {
                    blocks.insert(b, d.clone());
                }
            } else if let Some((rb, fut)) = inflight.get(&key) {
                if !waits.iter().any(|(r, _)| r == rb) {
                    waits.push((*rb, fut.clone()));
                }
            } else {
                missing.push(b);
            }
        }
        // Group missing blocks into contiguous runs, and claim them before
        // releasing the lock so no other request fetches them too.
        let mut runs: Vec<(u64, u64)> = Vec::new();
        for b in missing {
            match runs.last_mut() {
                Some((s, e)) if *e + 1 == b && *e + 1 - *s < CHUNK_BLOCKS => *e = b,
                _ => runs.push((b, b)),
            }
        }
        for (rb, re) in runs {
            let fut = pending_run(p.clone(), id.to_string(), obj.clone(), rb, re);
            for b in rb..=re {
                inflight.insert((id.to_string(), b), (rb, fut.clone()));
            }
            waits.push((rb, fut));
        }
    }
    if waits.is_empty() && !assemble {
        return Ok(None);
    }
    // Large reads (e.g. `SELECT *` touching many column chunks) become several
    // chunk-sized requests in flight at once instead of one long serial GET.
    use futures::stream::{self, StreamExt, TryStreamExt};
    let runs: Vec<_> = waits.into_iter().map(|(rb, f)| run_of(rb, f)).collect();
    let fetched: Vec<(u64, Bytes)> = stream::iter(runs)
        .buffered(PARALLEL_FETCHES)
        .try_collect()
        .await?;
    if !assemble {
        return Ok(None);
    }
    for (rb, data) in fetched {
        for (i, chunk) in data.chunks(BLOCK as usize).enumerate() {
            let b = rb + i as u64;
            if (first..=last).contains(&b) {
                blocks.insert(b, data.slice_ref(chunk));
            }
        }
    }

    let mut out = Vec::with_capacity((end - start + 1) as usize);
    for b in first..=last {
        let d = blocks
            .get(&b)
            .ok_or_else(|| format!("short read for {} (block {b})", obj.path))?;
        let base = b * BLOCK;
        let lo = start.saturating_sub(base) as usize;
        let hi = ((end + 1 - base) as usize).min(d.len());
        if lo < hi {
            out.extend_from_slice(&d[lo..hi]);
        }
    }
    Ok(Some(Bytes::from(out)))
}

/// Await a pending run, keeping its first block.
async fn run_of(rb: u64, fut: Pending) -> Result<(u64, Bytes), String> {
    fut.await.map(|d| (rb, d))
}

/// A fetch of blocks `rb..=re` that every request needing them can await.
/// When done it stores the blocks in the cache and releases its claim.
fn pending_run(p: Arc<Proxy>, id: String, obj: Arc<Object>, rb: u64, re: u64) -> Pending {
    use futures::FutureExt;
    async move {
        let s = rb * BLOCK;
        let e = ((re + 1) * BLOCK).min(obj.size) - 1;
        let r = fetch(&obj, s, e).await;
        let mut cache = p.cache.lock().unwrap();
        let mut inflight = p.inflight.lock().unwrap();
        if let Ok(data) = &r {
            for (i, chunk) in data.chunks(BLOCK as usize).enumerate() {
                cache.put((id.clone(), rb + i as u64), data.slice_ref(chunk));
            }
        }
        for b in rb..=re {
            let key = (id.clone(), b);
            if inflight.get(&key).is_some_and(|(r, _)| *r == rb) {
                inflight.remove(&key);
            }
        }
        r
    }
    .boxed()
    .shared()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU64;
    use std::sync::Arc;
    use tokio::net::TcpListener;

    #[test]
    fn local_ranges_are_read_exactly() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("x.parquet");
        std::fs::write(&f, b"0123456789").unwrap();
        assert_eq!(&read_local(&f, 2, 5).unwrap()[..], b"2345");
    }

    /// HTTP server whose range requests answer the given statuses in turn;
    /// resolving requests (`bytes=0-0`) always succeed. Returns its URL and
    /// the number of range requests served.
    async fn scripted_server(statuses: Vec<u16>) -> (String, Arc<AtomicU64>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let served = Arc::new(AtomicU64::new(0));
        let count = served.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    return;
                };
                let mut buf = [0u8; 2048];
                let n = sock.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_lowercase();
                let st = if req.contains("range: bytes=0-0") {
                    206
                } else {
                    let i = count.fetch_add(1, Ordering::SeqCst) as usize;
                    statuses[i.min(statuses.len() - 1)]
                };
                let resp =
                    format!("HTTP/1.1 {st} X\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                let _ = sock.write_all(resp.as_bytes()).await;
            }
        });
        (format!("http://{addr}/x.parquet"), served)
    }

    async fn range_with(statuses: Vec<u16>) -> (Result<u16, String>, u64) {
        let (url, served) = scripted_server(statuses).await;
        let resolved = Mutex::new(Some(url.clone()));
        let out = http_range(&reqwest::Client::new(), &url, &resolved, "bytes=0-9")
            .await
            .map(|r| r.status().as_u16());
        (out, served.load(Ordering::SeqCst))
    }

    #[tokio::test]
    async fn range_retries_transient_statuses() {
        assert_eq!(range_with(vec![503, 502, 206]).await, (Ok(206), 3));
        let (out, served) = range_with(vec![503]).await;
        assert_eq!(served, 3);
        assert!(out.unwrap_err().ends_with("HTTP 503"));
    }

    #[tokio::test]
    async fn range_retries_a_failed_send_with_a_fresh_url() {
        let (url, served) = scripted_server(vec![206]).await;
        // The cached signed URL points at a port nothing listens on.
        let closed = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let dead = format!("http://{}/x.parquet", closed.local_addr().unwrap());
        drop(closed);
        let resolved = Mutex::new(Some(dead));
        let resp = http_range(&reqwest::Client::new(), &url, &resolved, "bytes=0-9")
            .await
            .unwrap();
        assert_eq!(resp.status().as_u16(), 206);
        assert_eq!(served.load(Ordering::SeqCst), 1);
        assert_eq!(resolved.lock().unwrap().as_deref(), Some(url.as_str()));
    }

    #[tokio::test]
    async fn range_fails_fast_on_a_permanent_status() {
        let (out, served) = range_with(vec![404]).await;
        assert_eq!(served, 1);
        assert!(out.unwrap_err().ends_with("HTTP 404"));
    }

    #[tokio::test]
    async fn range_resolves_a_rejected_signed_url_again_once() {
        assert_eq!(range_with(vec![403, 206]).await, (Ok(206), 2));
        let (out, served) = range_with(vec![403]).await;
        assert_eq!(served, 2);
        assert!(out.unwrap_err().ends_with("HTTP 403"));
    }
}
