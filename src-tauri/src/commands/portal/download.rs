//! Copying portal views into the local cache.

use super::{BASE, DATASETS_ROOT};
use crate::commands::portal_http;
use crate::commands::transfers::{self as xfers, CacheKind};
use crate::qpx::ViewInfo;
use futures::{StreamExt, TryStreamExt};
use reqwest::Client;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use tauri::AppHandle;

/// File and folder names taken from URLs must stay inside the cache folder.
fn safe_component(s: &str) -> bool {
    !s.is_empty()
        && s != "."
        && !s.contains("..")
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// Cache folder for one run of a dataset (`<PXD>/<run-hash>`), so runs of the
/// same accession that share file names do not overwrite each other.
fn run_cache_dir(root: &std::path::Path, url: &str) -> PathBuf {
    let rest = url
        .strip_prefix(BASE)
        .and_then(|r| r.strip_prefix(DATASETS_ROOT))
        .unwrap_or("");
    let mut dir = root.to_path_buf();
    for seg in rest.split('/').take(2).filter(|s| safe_component(s)) {
        dir.push(seg);
    }
    dir
}

/// Size of a remote file, and whether its server answers byte ranges. The
/// portal answers HEAD with 405, so fall back to a one-byte ranged GET and
/// read the total from `Content-Range`. Both requests stop as soon as `token`
/// is cancelled.
async fn remote_size(
    client: &Client,
    url: &str,
    token: &AtomicBool,
) -> Result<(u64, bool), String> {
    xfers::cancellable(token, async {
        // reqwest's `.content_length()` returns the *body* length, which is 0
        // for a HEAD reply, so size HEADs by the Content-Length header directly.
        if let Ok(r) = client.head(url).send().await {
            if r.status().is_success() {
                let h = r.headers();
                let cl = h
                    .get(reqwest::header::CONTENT_LENGTH)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.trim().parse::<u64>().ok())
                    .filter(|n| *n > 0);
                let ranged = h
                    .get(reqwest::header::ACCEPT_RANGES)
                    .and_then(|v| v.to_str().ok())
                    .is_some_and(|v| v.eq_ignore_ascii_case("bytes"));
                if let Some(n) = cl {
                    return Ok((n, ranged));
                }
            }
        }
        let r = client
            .get(url)
            .header(reqwest::header::RANGE, "bytes=0-0")
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !r.status().is_success() {
            return Err(format!("HTTP {}", r.status()));
        }
        let total = r
            .headers()
            .get(reqwest::header::CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.rsplit('/').next())
            .and_then(|v| v.trim().parse::<u64>().ok());
        let status = r.status();
        if status == reqwest::StatusCode::PARTIAL_CONTENT {
            // On a 206 without a size the probe read is one byte, which is
            // not a real total, so don't trust it.
            return Ok(total.map_or((0, false), |n| (n, true)));
        }
        // A server that ignores Range sends the whole (200) body: use its
        // length, and fetch the file as one stream.
        Ok((total.or_else(|| r.content_length()).unwrap_or(0), false))
    })
    .await
}

/// Size and range support of a file still to download, or `None` when it is
/// already cached.
async fn probe(
    client: &Client,
    url: &str,
    name: &str,
    local: &Path,
    token: &AtomicBool,
) -> Result<Option<(u64, bool)>, String> {
    if local.exists() {
        return Ok(None);
    }
    remote_size(client, url, token)
        .await
        .map(Some)
        .map_err(|e| format!("size of {name}: {e}"))
}

/// A portal file. Each request goes through the portal URL, which redirects
/// to a freshly signed storage URL, so a long download never outlives its
/// signature.
struct Portal<'a> {
    client: &'a Client,
    url: &'a str,
}

impl xfers::Source for Portal<'_> {
    fn open(
        &self,
        range: Option<(u64, u64)>,
    ) -> futures::future::BoxFuture<'_, Result<xfers::Body, String>> {
        Box::pin(async move {
            let mut req = self.client.get(self.url);
            if let Some((s, e)) = range {
                req = req.header(reqwest::header::RANGE, format!("bytes={s}-{e}"));
            }
            let resp = req.send().await.map_err(|e| e.to_string())?;
            let st = resp.status();
            if !st.is_success() || (range.is_some() && st != reqwest::StatusCode::PARTIAL_CONTENT) {
                return Err(format!("HTTP {st}"));
            }
            Ok(resp
                .bytes_stream()
                .map(|c| c.map_err(|e| e.to_string()))
                .boxed())
        })
    }
}

/// Only portal dataset files may be downloaded: the URLs come from the
/// webview, and the command must not fetch arbitrary hosts (cloud metadata
/// endpoints, the local network) on its behalf.
fn check_portal_url(url: &str) -> Result<(), String> {
    let rest = url
        .strip_prefix(BASE)
        .and_then(|r| r.strip_prefix(DATASETS_ROOT))
        .ok_or_else(|| format!("{url}: only {BASE}{DATASETS_ROOT} files can be downloaded"))?;
    if rest.contains(['?', '#', '\\']) || rest.split('/').any(|seg| seg == "..") {
        return Err(format!("{url}: not a portal file URL"));
    }
    Ok(())
}

/// Download the given public parquet URLs into the local cache and return
/// their `ViewInfo` with `path` pointing at the local file. Reports live
/// progress over the `download-progress` event for each file. `connections`
/// is the user's parallel-connections setting.
#[tauri::command]
pub async fn portal_cache_views(
    app: AppHandle,
    cancel_id: String,
    urls: Vec<String>,
    connections: Option<usize>,
) -> Result<Vec<ViewInfo>, String> {
    for url in &urls {
        check_portal_url(url)?;
    }
    let client = portal_http::client()?;
    portal_cache_views_impl(
        Some(&app),
        &client,
        &CacheKind::Portal.root()?,
        cancel_id,
        urls,
        xfers::connections(connections),
    )
    .await
}

/// Testable core of [`portal_cache_views`]: `app` may be `None` and `root`
/// a temporary folder in tests.
pub(super) async fn portal_cache_views_impl(
    app: Option<&AppHandle>,
    client: &Client,
    root: &Path,
    cancel_id: String,
    urls: Vec<String>,
    connections: usize,
) -> Result<Vec<ViewInfo>, String> {
    std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let batch = xfers::begin(&cancel_id);
    let token = &batch.token;

    // Resolve names and cache paths, then size everything still to fetch
    // so a download that cannot fit is refused before it starts.
    let mut plan = Vec::new();
    for url in urls {
        let name = url.rsplit_once('/').map_or(url.as_str(), |(_, n)| n);
        if !safe_component(name) {
            return Err(format!("unsafe file name in {url}"));
        }
        let (_, view, format) = crate::qpx::parse_filename(name)
            .ok_or_else(|| format!("not a QPX view file: {url}"))?;
        let name = name.to_string();
        let dir = run_cache_dir(root, &url);
        let local = dir.join(&name);
        plan.push((url, name, view, format, dir, local));
    }
    // `None` marks a file that is already cached. Sized in parallel: each
    // probe is a round trip (two for the portal, which rejects HEAD).
    let probes: Vec<_> = plan
        .iter()
        .map(|(url, name, .., local)| probe(client, url, name, local, token))
        .collect();
    let sizes: Vec<Option<(u64, bool)>> = futures::stream::iter(probes)
        .buffered(connections)
        .try_collect()
        .await?;
    let need = sizes
        .iter()
        .flatten()
        .fold(0u64, |a, (b, _)| a.saturating_add(*b));
    xfers::ensure_space(root, need)?;

    // All files share the batch's connections. Small files are queued first,
    // so they are not stuck behind the ranges of a big one.
    let slots = xfers::Slots::new(connections);
    let mut jobs: Vec<_> = plan.into_iter().zip(sizes).enumerate().collect();
    jobs.sort_by_key(|(_, (_, size))| size.map_or(0, |(n, _)| n));
    let (batch, slots) = (&batch, &slots);
    let mut out = futures::future::try_join_all(jobs.into_iter().map(
        |(i, ((url, name, view, format, dir, local), size))| async move {
            let size_bytes = match size {
                None => std::fs::metadata(&local)
                    .map_err(|e| format!("{}: {e}", local.display()))?
                    .len(),
                Some((total, ranged)) => {
                    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                    let src = Portal { client, url: &url };
                    xfers::download(app, batch, slots, &src, &name, &local, total, ranged)
                        .await
                        .map_err(|e| {
                            if e == xfers::CANCELLED {
                                e
                            } else {
                                format!("download {name}: {e}")
                            }
                        })?
                }
            };
            Ok::<_, String>((
                i,
                ViewInfo {
                    name,
                    view,
                    format,
                    path: local.to_string_lossy().to_string(),
                    size_bytes: Some(size_bytes),
                },
            ))
        },
    ))
    .await?;
    out.sort_by_key(|(i, _)| *i);
    Ok(out.into_iter().map(|(_, v)| v).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_portal_urls_are_downloaded() {
        let ok = format!("{BASE}{DATASETS_ROOT}PXD1/ab12/PXD1.pg.parquet");
        assert!(check_portal_url(&ok).is_ok());
        for bad in [
            "http://169.254.169.254/latest/x.pg.parquet".to_string(),
            "https://browse.quantms.org.evil.com/quantms/datasets/x.pg.parquet".to_string(),
            "http://browse.quantms.org/quantms/datasets/PXD1/x.pg.parquet".to_string(),
            format!("{BASE}/other/x.pg.parquet"),
            format!("{BASE}{DATASETS_ROOT}../../x.pg.parquet"),
            format!("{BASE}{DATASETS_ROOT}PXD1/x.pg.parquet?u=http://a"),
        ] {
            assert!(check_portal_url(&bad).is_err(), "{bad}");
        }
    }

    #[tokio::test]
    async fn cached_files_stay_in_their_run_folder() {
        let root = Path::new("/cache");
        let url = |rest: &str| format!("{BASE}{DATASETS_ROOT}{rest}");
        assert_eq!(
            run_cache_dir(root, &url("PXD1/ab12/qpx/PXD1.pg.parquet")),
            root.join("PXD1/ab12")
        );
        assert_eq!(
            run_cache_dir(root, &url("../ab12/qpx/x.pg.parquet")),
            root.join("ab12")
        );
        assert_eq!(
            run_cache_dir(root, "http://127.0.0.1/PXD1/ab12/x.pg.parquet"),
            root
        );

        let tmp = tempfile::tempdir().unwrap();
        let e = portal_cache_views_impl(
            None,
            &Client::new(),
            tmp.path(),
            "test-unsafe".into(),
            vec!["http://127.0.0.1/a/..".into()],
            8,
        )
        .await
        .unwrap_err();
        assert!(e.starts_with("unsafe file name"), "{e}");
    }

    /// Tiny HTTP server: HEAD/GET advertise `len` bytes; GET streams 64 KB
    /// chunks slowly so a test can cancel mid-file.
    async fn slow_server(len: u64) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    return;
                };
                tokio::spawn(async move {
                    let mut buf = [0u8; 2048];
                    let n = sock.read(&mut buf).await.unwrap_or(0);
                    let head = std::str::from_utf8(&buf[..n])
                        .unwrap_or("")
                        .starts_with("HEAD");
                    let hdr = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n"
                    );
                    if sock.write_all(hdr.as_bytes()).await.is_err() || head {
                        return;
                    }
                    let chunk = vec![0u8; 64 * 1024];
                    loop {
                        if sock.write_all(&chunk).await.is_err() {
                            return;
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                    }
                });
            }
        });
        format!("http://{addr}")
    }

    /// HTTP server like the portal's: HEAD is 405, GET honours `Range`.
    async fn no_head_server(body: Vec<u8>) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let body = std::sync::Arc::new(body);
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    return;
                };
                let body = body.clone();
                tokio::spawn(async move {
                    let mut buf = [0u8; 4096];
                    let n = sock.read(&mut buf).await.unwrap_or(0);
                    let req = String::from_utf8_lossy(&buf[..n]).to_lowercase();
                    let resp = if req.starts_with("head") {
                        b"HTTP/1.1 405 Method Not Allowed\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
                    } else if req.contains("range: bytes=0-0") {
                        let mut r = format!(
                            "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-0/{}\r\nContent-Length: 1\r\nConnection: close\r\n\r\n",
                            body.len()
                        )
                        .into_bytes();
                        r.push(body[0]);
                        r
                    } else {
                        let mut r = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        )
                        .into_bytes();
                        r.extend_from_slice(&body);
                        r
                    };
                    let _ = sock.write_all(&resp).await;
                });
            }
        });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn downloads_when_server_rejects_head() {
        let body: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        let base = no_head_server(body.clone()).await;
        let client = Client::new();
        assert_eq!(
            remote_size(&client, &format!("{base}/x"), &AtomicBool::new(false))
                .await
                .unwrap()
                .0,
            body.len() as u64
        );

        let name = format!("nohead-test-{}.feature.parquet", std::process::id());
        let url = format!("{base}/{name}");
        let tmp = tempfile::tempdir().unwrap();
        let out = portal_cache_views_impl(None, &client, tmp.path(), "nohead".into(), vec![url], 8)
            .await
            .expect("download");
        assert_eq!(std::fs::read(&out[0].path).unwrap(), body);
    }

    /// HTTP server where HEAD answers 200 with a Content-Length; the size
    /// lookup must use that header (not reqwest's body length, which is 0).
    async fn head_server(body: Vec<u8>) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let body = std::sync::Arc::new(body);
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    return;
                };
                let body = body.clone();
                tokio::spawn(async move {
                    let mut buf = [0u8; 4096];
                    let n = sock.read(&mut buf).await.unwrap_or(0);
                    let req = String::from_utf8_lossy(&buf[..n]).to_lowercase();
                    let resp = if req.starts_with("head") {
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        )
                        .into_bytes()
                    } else {
                        let mut r = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        )
                        .into_bytes();
                        r.extend_from_slice(&body);
                        r
                    };
                    let _ = sock.write_all(&resp).await;
                });
            }
        });
        format!("http://{addr}")
    }

    /// Accepts connections and never answers them.
    async fn silent_server() -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let mut open = Vec::new();
            loop {
                open.push(listener.accept().await.unwrap().0);
            }
        });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn sizing_stops_when_cancelled() {
        let (client, url) = (Client::new(), format!("{}/x", silent_server().await));
        let cancelled = AtomicBool::new(true);
        let size = remote_size(&client, &url, &cancelled);
        let r = tokio::time::timeout(std::time::Duration::from_secs(2), size).await;
        assert_eq!(
            r.expect("cancel is prompt"),
            Err(xfers::CANCELLED.to_string())
        );
    }

    #[tokio::test]
    async fn sizes_by_head_content_length_when_head_supported() {
        let body: Vec<u8> = (0..10_000u32).map(|i| (i % 253) as u8).collect();
        let base = head_server(body.clone()).await;
        let client = Client::new();
        assert_eq!(
            remote_size(&client, &format!("{base}/x"), &AtomicBool::new(false))
                .await
                .unwrap()
                .0,
            body.len() as u64
        );
    }

    #[tokio::test]
    async fn new_download_starts_after_a_cancel() {
        let slow = slow_server(64 << 20).await;
        let big = format!("{slow}/recancel-{}.feature.parquet", std::process::id());
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        let job = tokio::spawn(async move {
            portal_cache_views_impl(None, &Client::new(), &root, "first".into(), vec![big], 8).await
        });
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        assert!(xfers::cancel("first"));
        assert_eq!(job.await.unwrap().unwrap_err(), xfers::CANCELLED);

        let body: Vec<u8> = (0..100_000u32).map(|i| (i % 7) as u8).collect();
        let base = no_head_server(body.clone()).await;
        let url = format!("{base}/after-cancel-{}.pg.parquet", std::process::id());
        let out = portal_cache_views_impl(
            None,
            &Client::new(),
            tmp.path(),
            "second".into(),
            vec![url],
            8,
        )
        .await
        .expect("second download");
        assert_eq!(std::fs::read(&out[0].path).unwrap(), body);
    }

    #[tokio::test]
    async fn cancel_mid_file_leaves_no_partial_file() {
        let base = slow_server(64 << 20).await;
        let name = format!("cancel-test-{}.feature.parquet", std::process::id());
        let tmp = tempfile::tempdir().unwrap();
        let local = tmp.path().join(&name);
        let id = format!("test-cancel-{}", std::process::id());
        let (root, url, task_id) = (
            tmp.path().to_path_buf(),
            format!("{base}/{name}"),
            id.clone(),
        );
        let task = tokio::spawn(async move {
            portal_cache_views_impl(None, &Client::new(), &root, task_id, vec![url], 8).await
        });
        let part = local.with_file_name(format!("{name}.part"));
        for _ in 0..100 {
            if part.exists() && std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0) > 0 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(part.exists(), "download should have started");
        assert!(xfers::cancel(&id));
        let r = tokio::time::timeout(std::time::Duration::from_secs(5), task)
            .await
            .expect("cancel is prompt")
            .unwrap();
        assert_eq!(r.unwrap_err(), xfers::CANCELLED);
        assert!(
            !part.exists() && !local.exists(),
            "partial download must be removed"
        );
    }

    #[tokio::test]
    async fn refuses_download_that_does_not_fit() {
        let base = slow_server(1 << 50).await; // 1 PB
        let name = format!("huge-test-{}.feature.parquet", std::process::id());
        let tmp = tempfile::tempdir().unwrap();
        let r = portal_cache_views_impl(
            None,
            &Client::new(),
            tmp.path(),
            "test-huge".into(),
            vec![format!("{base}/{name}")],
            8,
        )
        .await;
        let e = r.unwrap_err();
        assert!(e.starts_with("Not enough disk space"), "{e}");
        let local = tmp.path().join(&name);
        assert!(!local.exists() && !local.with_file_name(format!("{name}.part")).exists());
    }

    /// Live speed check against the real portal (network, ~250 MB):
    /// `QPX_CONNECTIONS=8 cargo test live_portal_download -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn live_portal_download() {
        let conns = std::env::var("QPX_CONNECTIONS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(xfers::DEFAULT_CONNECTIONS);
        let run = format!("{BASE}{DATASETS_ROOT}PXD004452/eac302ad85f8");
        let urls = [
            "PXD004452-GluC.psm.parquet",
            "PXD004452.run.parquet",
            "PXD004452.sample.parquet",
        ]
        .map(|f| format!("{run}/{f}"))
        .to_vec();
        let tmp = tempfile::tempdir().unwrap();
        let client = portal_http::client().unwrap();
        let t = std::time::Instant::now();
        let out = portal_cache_views_impl(None, &client, tmp.path(), "live".into(), urls, conns)
            .await
            .unwrap();
        let secs = t.elapsed().as_secs_f64();
        let bytes: u64 = out.iter().filter_map(|v| v.size_bytes).sum();
        println!(
            "{conns} connections: {bytes} B in {secs:.1}s = {:.1} MB/s",
            bytes as f64 / secs / 1e6
        );
        assert_eq!(out[0].size_bytes, Some(245_917_608));
        let f = std::fs::read(&out[0].path).unwrap();
        assert_eq!(&f[..4], b"PAR1");
        assert_eq!(&f[f.len() - 4..], b"PAR1");
    }
}
