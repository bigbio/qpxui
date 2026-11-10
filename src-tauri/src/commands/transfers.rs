//! Cooperative cancellation for long-running downloads (big S3 / portal
//! parquet views). The frontend generates a unique `cancelId` per batch, the
//! Rust command registers a shared cancel token under that id, and checks it
//! between every chunk. `cancel_download(id)` flips the token; the loop then
//! aborts promptly instead of streaming the whole file.

use crate::commands::progress;
use bytes::Bytes;
use futures::StreamExt;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use tauri::AppHandle;

static ACTIVE: OnceLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = OnceLock::new();

fn registry() -> &'static Mutex<HashMap<String, Arc<AtomicBool>>> {
    ACTIVE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// A registered download batch. Dropping it, however the command ends,
/// removes its id from the registry.
pub struct Batch {
    id: String,
    pub token: Arc<AtomicBool>,
}

/// Register a download batch under the frontend's `cancelId`.
pub fn begin(id: &str) -> Batch {
    let token = Arc::new(AtomicBool::new(false));
    registry()
        .lock()
        .expect("transfers registry poisoned")
        .insert(id.to_string(), token.clone());
    Batch {
        id: id.to_string(),
        token,
    }
}

impl Drop for Batch {
    fn drop(&mut self) {
        registry()
            .lock()
            .expect("transfers registry poisoned")
            .remove(&self.id);
    }
}

/// Request cancellation for a batch. Returns whether a matching download
/// existed. Setting the token is asynchronous: the running loop observes it
/// at the next chunk boundary.
pub fn cancel(id: &str) -> bool {
    let hit = {
        let guard = registry().lock().expect("transfers registry poisoned");
        guard.get(id).cloned()
    };
    match hit {
        Some(token) => {
            token.store(true, Ordering::SeqCst);
            true
        }
        None => false,
    }
}

/// The error of a cancelled download. The front end recognises a cancel by
/// this exact text (`CANCELLED` in `src/lib/errors.ts`).
pub const CANCELLED: &str = "cancelled";

/// Poll helper for the download loop.
pub fn is_cancelled(token: &AtomicBool) -> bool {
    token.load(Ordering::Relaxed)
}

/// Run `fut`, aborting as soon as the cancel token is set, even while `fut`
/// is still waiting for the server to reply (an in-flight `send()` with no
/// data yet — that phase had no chunk loop to poll it). Wrapping a request in
/// this means cancel always takes effect within [`POLL`], never only at the
/// next body chunk. The two branches of `select!` never both resolve, so a
/// finished future is returned exact, not dropped.
pub async fn cancellable<T, F>(token: &AtomicBool, fut: F) -> Result<T, String>
where
    F: std::future::Future<Output = Result<T, String>>,
{
    tokio::pin!(fut);
    let mut tick = tokio::time::interval(POLL);
    tick.tick().await; // consume the immediate first tick
    loop {
        tokio::select! {
            r = &mut fut => return r,
            _ = tick.tick() => {
                if is_cancelled(token) {
                    return Err(CANCELLED.to_string());
                }
            }
        }
    }
}

/// Tauri command: cancel an in-flight download batch by its `cancelId`. A
/// batch that already finished has nothing left to cancel.
#[tauri::command]
pub fn cancel_download(id: String) {
    cancel(&id);
}

/// How often a stalled download re-checks its cancel token.
pub const POLL: std::time::Duration = std::time::Duration::from_millis(250);

/// Download cache of one source, under the user's cache folder.
#[derive(Clone, Copy, Debug, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CacheKind {
    Portal,
    S3,
}

impl CacheKind {
    /// `<cache dir>/qpxui/portal` or `<cache dir>/qpxui/s3-cache`.
    pub fn root(self) -> Result<PathBuf, String> {
        let base = dirs::cache_dir().ok_or_else(|| "no cache directory available".to_string())?;
        let sub = match self {
            CacheKind::Portal => "portal",
            CacheKind::S3 => "s3-cache",
        };
        Ok(base.join("qpxui").join(sub))
    }
}

/// Space left free on the cache volume after a download, so a big transfer
/// never fills the disk to the last byte.
pub const RESERVE_BYTES: u64 = 1 << 30;

/// Free bytes on the volume holding `dir` (walks up to the nearest existing
/// ancestor, since cache sub-folders may not exist yet).
pub fn free_space(dir: &Path) -> Result<u64, String> {
    let mut p = dir;
    while !p.exists() {
        p = p
            .parent()
            .ok_or_else(|| format!("no existing parent for {}", dir.display()))?;
    }
    fs4::available_space(p).map_err(|e| format!("free space of {}: {e}", p.display()))
}

/// Human-readable size (binary units), for error messages.
pub fn human(n: u64) -> String {
    const U: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", U[i])
    }
}

/// Fail when `need` bytes (plus [`RESERVE_BYTES`]) do not fit in `free`.
pub fn check_fits(need: u64, free: u64) -> Result<(), String> {
    if need.saturating_add(RESERVE_BYTES) > free {
        return Err(format!(
            "Not enough disk space: the download needs {}, but only {} is free on the cache disk (keeping {} spare). Free some space or use Stream instead.",
            human(need),
            human(free),
            human(RESERVE_BYTES)
        ));
    }
    Ok(())
}

/// Check that `need` bytes fit on the volume holding `dir`.
pub fn ensure_space(dir: &Path, need: u64) -> Result<(), String> {
    check_fits(need, free_space(dir)?)
}

/// Parallel connections used for one download batch when the frontend does
/// not say. Remote storage (the portal's object store in particular) caps
/// throughput per connection at a few MB/s, so several ranged requests at once
/// are many times faster than one stream.
pub const DEFAULT_CONNECTIONS: usize = 8;
/// Upper bound on the user's "parallel connections" setting.
pub const MAX_CONNECTIONS: usize = 16;

/// The user's connection setting, clamped to `1..=MAX_CONNECTIONS`.
pub fn connections(requested: Option<usize>) -> usize {
    requested
        .unwrap_or(DEFAULT_CONNECTIONS)
        .clamp(1, MAX_CONNECTIONS)
}

/// Largest byte range fetched by one request. Big enough that per-request
/// overhead (redirect, TLS reuse, headers) is noise; small enough that the
/// last ranges of a file finish close together.
const SEGMENT_MAX: u64 = 32 << 20;
/// Smallest range worth its own request; files under two of these are fetched
/// as a single stream.
const SEGMENT_MIN: u64 = 4 << 20;
/// Bytes received per connection before they are written to disk in one call.
const WRITE_BUF: usize = 2 << 20;
/// Minimum time between two progress events of one file. Events cross into
/// the webview and re-render the progress bars; one per network chunk
/// (~16 KB) cost more than the transfer itself.
const PROGRESS_EVERY: std::time::Duration = std::time::Duration::from_millis(100);
/// Tries per byte range that makes no progress before the download fails.
/// A range that fails after receiving data resumes where it stopped.
const SEGMENT_ATTEMPTS: usize = 4;

/// A response body: a stream of byte chunks.
pub type Body = futures::stream::BoxStream<'static, Result<Bytes, String>>;

/// Where a download reads from (the portal over HTTPS, or S3).
pub trait Source: Sync {
    /// Open the inclusive byte range `start..=end`, or the whole object for
    /// `None`. A ranged open must fail unless the server honoured the range.
    fn open(
        &self,
        range: Option<(u64, u64)>,
    ) -> futures::future::BoxFuture<'_, Result<Body, String>>;
}

/// Connection slots shared by every file of one batch, so a batch never opens
/// more than the user's setting at once, whatever mix of files it holds.
pub struct Slots {
    sem: tokio::sync::Semaphore,
    pub connections: usize,
}

impl Slots {
    pub fn new(connections: usize) -> Self {
        Self {
            sem: tokio::sync::Semaphore::new(connections),
            connections,
        }
    }
}

/// Byte ranges to fetch `total` bytes over `connections` connections, or an
/// empty list when the file should come as a single stream (no range
/// support, unknown or small size).
fn segments(total: u64, ranged: bool, connections: usize) -> Vec<(u64, u64)> {
    if !ranged || connections < 2 || total < 2 * SEGMENT_MIN {
        return Vec::new();
    }
    // About two ranges per connection, so a slow range near the end does not
    // leave the other connections idle for long.
    let per = total.div_ceil(2 * connections as u64);
    let size = per.clamp(SEGMENT_MIN, SEGMENT_MAX);
    (0..total)
        .step_by(size as usize)
        .map(|s| (s, (s + size).min(total) - 1))
        .collect()
}

/// A download in progress: bytes go to `<final>.part`, written at their
/// offsets by any number of connections, and the file is renamed to the final
/// name by [`PartFile::finish`]. Dropping it unfinished (error, cancel)
/// deletes the partial file, so the cache never holds a truncated parquet.
pub struct PartFile {
    part: PathBuf,
    dest: PathBuf,
    file: Option<Arc<std::fs::File>>,
}

fn write_error(part: &Path, e: std::io::Error) -> String {
    if e.kind() == std::io::ErrorKind::StorageFull {
        "Not enough disk space: the cache disk filled up during the download; the partial file was removed.".to_string()
    } else {
        format!("write {}: {e}", part.display())
    }
}

#[cfg(unix)]
fn write_all_at(f: &std::fs::File, buf: &[u8], off: u64) -> std::io::Result<()> {
    std::os::unix::fs::FileExt::write_all_at(f, buf, off)
}

#[cfg(windows)]
fn write_all_at(f: &std::fs::File, mut buf: &[u8], mut off: u64) -> std::io::Result<()> {
    use std::os::windows::fs::FileExt;
    while !buf.is_empty() {
        match f.seek_write(buf, off)? {
            0 => return Err(std::io::ErrorKind::WriteZero.into()),
            n => {
                buf = &buf[n..];
                off += n as u64;
            }
        }
    }
    Ok(())
}

impl PartFile {
    /// Create `<dest>.part`, sized to `size` bytes when known so ranges can
    /// be written anywhere in it.
    pub async fn create(dest: &Path, size: u64) -> Result<Self, String> {
        let mut name = dest.file_name().unwrap_or_default().to_os_string();
        name.push(".part");
        let part = dest.with_file_name(name);
        let path = part.clone();
        let file = tokio::task::spawn_blocking(move || {
            let f = std::fs::File::create(&path)?;
            if size > 0 {
                f.set_len(size)?;
            }
            Ok::<_, std::io::Error>(f)
        })
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("create {}: {e}", part.display()))?;
        Ok(Self {
            part,
            dest: dest.to_path_buf(),
            file: Some(Arc::new(file)),
        })
    }

    /// Write `buf` at byte offset `off`, off the async runtime.
    pub async fn write_at(&self, off: u64, buf: Vec<u8>) -> Result<(), String> {
        let f = self.file.clone().ok_or("download already finished")?;
        tokio::task::spawn_blocking(move || write_all_at(&f, &buf, off))
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| write_error(&self.part, e))
    }

    /// Flush to disk and move the file to its final name.
    pub async fn finish(mut self) -> Result<(), String> {
        if let Some(f) = self.file.take() {
            tokio::task::spawn_blocking(move || f.sync_all())
                .await
                .map_err(|e| e.to_string())?
                .map_err(|e| e.to_string())?;
        }
        tokio::fs::rename(&self.part, &self.dest)
            .await
            .map_err(|e| format!("rename {}: {e}", self.part.display()))?;
        self.part = PathBuf::new();
        Ok(())
    }
}

impl Drop for PartFile {
    fn drop(&mut self) {
        self.file.take();
        if !self.part.as_os_str().is_empty() {
            let _ = std::fs::remove_file(&self.part);
        }
    }
}

/// Bytes received for one file, reported to the frontend at most every
/// [`PROGRESS_EVERY`].
struct Meter<'a> {
    app: Option<&'a AppHandle>,
    batch: &'a str,
    name: &'a str,
    total: u64,
    done: AtomicU64,
    last: Mutex<std::time::Instant>,
}

impl Meter<'_> {
    fn add(&self, n: u64) {
        let done = self.done.fetch_add(n, Ordering::Relaxed) + n;
        let Some(app) = self.app else { return };
        let mut last = self.last.lock().expect("progress meter poisoned");
        if last.elapsed() >= PROGRESS_EVERY {
            *last = std::time::Instant::now();
            drop(last);
            progress::chunk(app, self.batch, self.name, done, self.total);
        }
    }
}

/// Why a range transfer stopped.
enum Stop {
    /// Cancelled, or a local failure (disk full): give up at once.
    Fatal(String),
    /// A network failure: worth retrying from where it stopped.
    Retry(String),
}

/// Fetch `start..=end` (or the whole object when `end` is `None`) into
/// `part`, retrying network failures and resuming after the bytes already
/// received.
async fn fetch_range(
    src: &dyn Source,
    part: &PartFile,
    meter: &Meter<'_>,
    token: &AtomicBool,
    start: u64,
    end: Option<u64>,
) -> Result<(), String> {
    let mut pos = start;
    let mut failures = 0;
    loop {
        if is_cancelled(token) {
            return Err(CANCELLED.to_string());
        }
        let before = pos;
        match pump(src, part, meter, token, &mut pos, end).await {
            Ok(()) => return Ok(()),
            Err(Stop::Fatal(e)) => return Err(e),
            // Without a range the stream cannot resume part-way.
            Err(Stop::Retry(e)) if end.is_none() => return Err(e),
            Err(Stop::Retry(e)) => {
                failures = if pos > before { 1 } else { failures + 1 };
                if failures >= SEGMENT_ATTEMPTS {
                    return Err(format!("{e} (after {SEGMENT_ATTEMPTS} attempts)"));
                }
                let wait = std::time::Duration::from_millis(500 << failures.min(4));
                cancellable(token, async {
                    tokio::time::sleep(wait).await;
                    Ok(())
                })
                .await?;
            }
        }
    }
}

/// One request of [`fetch_range`]: stream from `*pos` to `end`, writing in
/// [`WRITE_BUF`] blocks and advancing `*pos` past every byte received (and
/// written, also when the request fails).
async fn pump(
    src: &dyn Source,
    part: &PartFile,
    meter: &Meter<'_>,
    token: &AtomicBool,
    pos: &mut u64,
    end: Option<u64>,
) -> Result<(), Stop> {
    let range = end.map(|e| (*pos, e));
    let mut body = cancellable(token, src.open(range)).await.map_err(|e| {
        if e == CANCELLED {
            Stop::Fatal(e)
        } else {
            Stop::Retry(e)
        }
    })?;
    let mut buf: Vec<u8> = Vec::with_capacity(WRITE_BUF);
    let mut buf_at = *pos;
    let result = loop {
        let next = match tokio::time::timeout(POLL, body.next()).await {
            Ok(next) => next,
            Err(_) if is_cancelled(token) => break Err(Stop::Fatal(CANCELLED.to_string())),
            Err(_) => continue,
        };
        if is_cancelled(token) {
            break Err(Stop::Fatal(CANCELLED.to_string()));
        }
        let chunk = match next {
            None => break Ok(()),
            Some(Err(e)) => break Err(Stop::Retry(e)),
            Some(Ok(c)) => c,
        };
        if end.is_some_and(|e| *pos + chunk.len() as u64 > e + 1) {
            break Err(Stop::Fatal(
                "server sent more bytes than the range requested".to_string(),
            ));
        }
        buf.extend_from_slice(&chunk);
        *pos += chunk.len() as u64;
        meter.add(chunk.len() as u64);
        if buf.len() >= WRITE_BUF {
            let full = std::mem::replace(&mut buf, Vec::with_capacity(WRITE_BUF));
            part.write_at(buf_at, full).await.map_err(Stop::Fatal)?;
            buf_at = *pos;
        }
    };
    // Keep what was received, also before a retry resumes after it.
    if !buf.is_empty() && !matches!(result, Err(Stop::Fatal(_))) {
        part.write_at(buf_at, buf).await.map_err(Stop::Fatal)?;
    }
    result?;
    match end {
        Some(e) if *pos != e + 1 => Err(Stop::Retry(format!(
            "connection closed at byte {} of {}",
            *pos,
            e + 1
        ))),
        _ => Ok(()),
    }
}

/// Download one file from `src` to `dest` through its `.part` file, reporting
/// progress for `name` in `batch`. `total` is the expected size (0 if
/// unknown); when `ranged`, the file is fetched as byte ranges over the
/// batch's connection `slots`, otherwise as one stream.
#[allow(clippy::too_many_arguments)]
pub async fn download(
    app: Option<&AppHandle>,
    batch: &Batch,
    slots: &Slots,
    src: &dyn Source,
    name: &str,
    dest: &Path,
    total: u64,
    ranged: bool,
) -> Result<u64, String> {
    let token = &batch.token;
    let part = PartFile::create(dest, total).await?;
    let meter = Meter {
        app,
        batch: &batch.id,
        name,
        total,
        done: AtomicU64::new(0),
        last: Mutex::new(
            std::time::Instant::now()
                .checked_sub(PROGRESS_EVERY)
                .unwrap_or_else(std::time::Instant::now),
        ),
    };
    let ranges = segments(total, ranged, slots.connections);
    let fetch = |start: u64, end: Option<u64>| {
        let (part, meter) = (&part, &meter);
        async move {
            let _slot = slots.sem.acquire().await.map_err(|e| e.to_string())?;
            fetch_range(src, part, meter, token, start, end).await
        }
    };
    if ranges.is_empty() {
        fetch(0, None).await?;
    } else {
        futures::future::try_join_all(ranges.iter().map(|&(s, e)| fetch(s, Some(e)))).await?;
    }
    let got = meter.done.load(Ordering::Relaxed);
    if total > 0 && got != total {
        return Err(format!("{name}: got {got} of {total} bytes"));
    }
    part.finish().await?;
    if let Some(app) = app {
        progress::done(app, &batch.id, name, got);
    }
    Ok(got)
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskSpace {
    pub free: u64,
    pub reserve: u64,
}

/// Tauri command: free space on the download cache volume, so the UI can
/// warn before starting a download.
#[tauri::command]
pub fn cache_free_space(kind: CacheKind) -> Result<DiskSpace, String> {
    Ok(DiskSpace {
        free: free_space(&kind.root()?)?,
        reserve: RESERVE_BYTES,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn space_check_keeps_a_reserve() {
        let gb = 1u64 << 30;
        assert!(check_fits(10 * gb, 20 * gb).is_ok());
        let e = check_fits(24 * gb, 20 * gb).unwrap_err();
        assert!(e.starts_with("Not enough disk space"), "{e}");
        assert!(e.contains("24.0 GB") && e.contains("20.0 GB"), "{e}");
        // Fits exactly only with the reserve left over.
        assert!(check_fits(19 * gb, 20 * gb).is_ok());
        assert!(check_fits(19 * gb + 1, 20 * gb).is_err());
    }

    #[test]
    fn cancel_reaches_a_batch_only_while_it_runs() {
        let batch = begin("test-cancel");
        assert!(!is_cancelled(&batch.token));
        assert!(cancel("test-cancel"));
        assert!(is_cancelled(&batch.token));
        assert!(!cancel("test-unknown"));
        drop(batch);
        assert!(!cancel("test-cancel"));
    }

    #[tokio::test]
    async fn cancellable_ends_a_stalled_request() {
        let batch = begin("test-cancellable");
        assert_eq!(cancellable(&batch.token, async { Ok(7) }).await, Ok(7));
        cancel("test-cancellable");
        let stalled = std::future::pending::<Result<(), String>>();
        assert_eq!(
            cancellable(&batch.token, stalled).await,
            Err(CANCELLED.to_string())
        );
    }

    #[test]
    fn free_space_walks_up_to_an_existing_dir() {
        let p = std::env::temp_dir().join("qpxui-nope").join("deeper");
        assert!(free_space(&p).unwrap() > 0);
    }

    #[tokio::test]
    async fn part_file_is_removed_unless_finished() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("x.feature.parquet");
        let part = dir.path().join("x.feature.parquet.part");

        let f = PartFile::create(&dest, 3).await.unwrap();
        f.write_at(0, b"abc".to_vec()).await.unwrap();
        assert!(part.exists());
        drop(f); // cancelled / failed
        assert!(!part.exists() && !dest.exists());

        // Ranges may land in any order.
        let f = PartFile::create(&dest, 6).await.unwrap();
        f.write_at(3, b"def".to_vec()).await.unwrap();
        f.write_at(0, b"abc".to_vec()).await.unwrap();
        f.finish().await.unwrap();
        assert!(!part.exists());
        assert_eq!(std::fs::read(&dest).unwrap(), b"abcdef");
    }

    #[test]
    fn ranges_cover_the_file_exactly() {
        let mb = 1u64 << 20;
        assert!(segments(100 * mb, false, 8).is_empty(), "no range support");
        assert!(segments(100 * mb, true, 1).is_empty(), "one connection");
        assert!(segments(7 * mb, true, 8).is_empty(), "small file");
        for (total, conns) in [(8 * mb, 8), (100 * mb + 7, 8), (7_000 * mb + 1, 16)] {
            let r = segments(total, true, conns);
            assert!(r.len() >= 2);
            assert_eq!(r[0].0, 0);
            assert_eq!(r.last().unwrap().1, total - 1);
            for w in r.windows(2) {
                assert_eq!(w[0].1 + 1, w[1].0);
            }
            assert!(r.iter().all(|(s, e)| e - s < SEGMENT_MAX));
        }
    }

    #[test]
    fn connection_setting_is_clamped() {
        assert_eq!(connections(None), DEFAULT_CONNECTIONS);
        assert_eq!(connections(Some(0)), 1);
        assert_eq!(connections(Some(99)), MAX_CONNECTIONS);
        assert_eq!(connections(Some(4)), 4);
    }

    /// In-memory source: serves ranges in small chunks, counts concurrent
    /// opens, and can drop the connection once after `fail_after` bytes.
    struct Mem {
        data: Arc<Vec<u8>>,
        ranged: bool,
        fail_after: Mutex<Option<usize>>,
        open_now: Arc<std::sync::atomic::AtomicUsize>,
        open_max: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl Mem {
        fn new(data: Vec<u8>, ranged: bool) -> Self {
            Self {
                data: Arc::new(data),
                ranged,
                fail_after: Mutex::new(None),
                open_now: Default::default(),
                open_max: Default::default(),
            }
        }
    }

    struct Open(Arc<std::sync::atomic::AtomicUsize>);
    impl Drop for Open {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }

    impl Source for Mem {
        fn open(
            &self,
            range: Option<(u64, u64)>,
        ) -> futures::future::BoxFuture<'_, Result<Body, String>> {
            Box::pin(async move {
                if range.is_some() && !self.ranged {
                    return Err("HTTP 200: range ignored".into());
                }
                let (s, e) = range.unwrap_or((0, self.data.len() as u64 - 1));
                let n = self.open_now.fetch_add(1, Ordering::SeqCst) + 1;
                self.open_max.fetch_max(n, Ordering::SeqCst);
                let guard = Open(self.open_now.clone());
                let mut bytes = self.data[s as usize..=e as usize].to_vec();
                let fail = self.fail_after.lock().unwrap().take();
                let mut chunks: Vec<Result<Bytes, String>> = Vec::new();
                if let Some(cut) = fail {
                    bytes.truncate(cut);
                }
                for c in bytes.chunks(64 * 1024) {
                    chunks.push(Ok(Bytes::copy_from_slice(c)));
                }
                if fail.is_some() {
                    chunks.push(Err("connection reset".into()));
                }
                let body = futures::stream::iter(chunks).then(move |c| {
                    let _keep = &guard;
                    async move {
                        tokio::task::yield_now().await;
                        c
                    }
                });
                Ok(body.boxed())
            })
        }
    }

    fn pattern(n: usize) -> Vec<u8> {
        (0..n).map(|i| (i * 31 % 251) as u8).collect()
    }

    #[tokio::test]
    async fn ranged_download_uses_parallel_connections() {
        let data = pattern((40 << 20) + 12_345);
        let src = Mem::new(data.clone(), true);
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("big.psm.parquet");
        let batch = begin("test-ranged");
        let slots = Slots::new(4);
        let n = download(
            None,
            &batch,
            &slots,
            &src,
            "big",
            &dest,
            data.len() as u64,
            true,
        )
        .await
        .unwrap();
        assert_eq!(n, data.len() as u64);
        assert_eq!(std::fs::read(&dest).unwrap(), data);
        let max = src.open_max.load(Ordering::SeqCst);
        assert!((2..=4).contains(&max), "{max} connections at once");
    }

    #[tokio::test]
    async fn dropped_range_resumes_where_it_stopped() {
        let data = pattern(20 << 20);
        let src = Mem::new(data.clone(), true);
        *src.fail_after.lock().unwrap() = Some(3 << 20);
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("x.pg.parquet");
        let batch = begin("test-resume");
        download(
            None,
            &batch,
            &Slots::new(2),
            &src,
            "x",
            &dest,
            data.len() as u64,
            true,
        )
        .await
        .unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), data);
    }

    #[tokio::test]
    async fn unranged_source_streams_once() {
        let data = pattern(9 << 20);
        let src = Mem::new(data.clone(), false);
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("x.pg.parquet");
        let batch = begin("test-unranged");
        download(None, &batch, &Slots::new(8), &src, "x", &dest, 0, false)
            .await
            .unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), data);
        assert_eq!(src.open_max.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn short_download_fails_and_leaves_no_file() {
        let data = pattern(1000);
        let src = Mem::new(data, false);
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("x.pg.parquet");
        let batch = begin("test-short");
        let e = download(None, &batch, &Slots::new(8), &src, "x", &dest, 5000, false)
            .await
            .unwrap_err();
        assert!(e.contains("got 1000 of 5000"), "{e}");
        assert!(!dest.exists() && !dir.path().join("x.pg.parquet.part").exists());
    }
}
