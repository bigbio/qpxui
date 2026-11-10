use crate::commands::transfers::{self as xfers, CacheKind};
use crate::qpx::{self, DatasetInfo, ViewInfo};
use aws_config::meta::region::RegionProviderChain;
use aws_config::BehaviorVersion;
use aws_sdk_s3::error::DisplayErrorContext;
use aws_sdk_s3::Client;
use futures::{StreamExt, TryStreamExt};
use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::Mutex;
use tauri::AppHandle;
use tokio::sync::OnceCell;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct S3Object {
    pub key: String,
    pub size_bytes: Option<u64>,
    pub last_modified: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct S3Listing {
    pub bucket: String,
    pub prefix: String,
    pub objects: Vec<S3Object>,
    pub datasets: Vec<DatasetInfo>,
}

/// A client for the user's AWS settings (credentials chain, region,
/// endpoint), which are loaded once: on a machine without a configured region
/// the region lookup alone waits about a second for the EC2 metadata service.
/// A custom endpoint (`AWS_ENDPOINT_URL`, e.g. MinIO / Ceph / a local test
/// server) switches to path-style addressing, which those servers expect.
async fn client() -> Client {
    static CONFIG: OnceCell<aws_config::SdkConfig> = OnceCell::const_new();
    let config = CONFIG
        .get_or_init(|| async {
            aws_config::defaults(BehaviorVersion::latest())
                .region(RegionProviderChain::default_provider().or_else("us-east-1"))
                .load()
                .await
        })
        .await;
    let mut b = aws_sdk_s3::config::Builder::from(config);
    if std::env::var_os("AWS_ENDPOINT_URL").is_some()
        || std::env::var_os("AWS_ENDPOINT_URL_S3").is_some()
    {
        b = b.force_path_style(true);
    }
    Client::from_conf(b.build())
}

/// A client in the bucket's own region, kept for the session. Range reads are
/// many small requests, so the region is resolved once (HeadBucket reports it
/// even on a redirect) and the client keeps its connections.
pub(crate) async fn client_for_bucket(bucket: &str) -> Client {
    static CLIENTS: Mutex<BTreeMap<String, Client>> = Mutex::new(BTreeMap::new());
    bucket_client(&CLIENTS, client().await, bucket).await
}

/// [`client_for_bucket`] over the given clients and base client. A bucket
/// whose region could not be read is not kept, so the next call asks again.
async fn bucket_client(
    clients: &Mutex<BTreeMap<String, Client>>,
    base: Client,
    bucket: &str,
) -> Client {
    if let Some(client) = clients.lock().unwrap().get(bucket) {
        return client.clone();
    }
    let region = match base.head_bucket().bucket(bucket).send().await {
        Ok(out) => out.bucket_region().map(str::to_string),
        Err(e) => e
            .raw_response()
            .and_then(|r| r.headers().get("x-amz-bucket-region"))
            .map(str::to_string),
    };
    let Some(region) = region else {
        return base;
    };
    let client = if Some(region.as_str()) == base.config().region().map(|r| r.as_ref()) {
        base
    } else {
        let config = base
            .config()
            .to_builder()
            .region(aws_config::Region::new(region));
        Client::from_conf(config.build())
    };
    clients
        .lock()
        .unwrap()
        .insert(bucket.to_string(), client.clone());
    client
}

fn s3_url(bucket: &str, key: &str) -> String {
    format!("s3://{bucket}/{key}")
}

/// List the objects in a bucket/prefix and group the QPX parquet files into
/// datasets (grouped by the `<prefix>` part of their filenames).
#[tauri::command]
pub async fn list_s3(bucket: String, prefix: Option<String>) -> Result<S3Listing, String> {
    let client = client_for_bucket(&bucket).await;
    let prefix = prefix.unwrap_or_default();

    let mut objects: Vec<S3Object> = Vec::new();
    let mut paginator = client
        .list_objects_v2()
        .bucket(&bucket)
        .prefix(prefix.clone())
        .into_paginator()
        .send();

    while let Some(page) = paginator.next().await {
        let page = page.map_err(|e| format!("S3 list error: {e}"))?;
        for o in page.contents() {
            if let Some(key) = o.key() {
                objects.push(S3Object {
                    key: key.to_string(),
                    size_bytes: o.size().map(|size| size as u64),
                    last_modified: o.last_modified().map(|d| d.to_string()),
                });
            }
        }
    }

    let mut by_prefix: BTreeMap<String, Vec<ViewInfo>> = BTreeMap::new();
    for obj in &objects {
        let base = obj.key.rsplit('/').next().unwrap_or(&obj.key);
        if let Some((p, view, format)) = qpx::parse_filename(base) {
            by_prefix.entry(p).or_default().push(ViewInfo {
                name: base.to_string(),
                view,
                format,
                path: s3_url(&bucket, &obj.key),
                size_bytes: obj.size_bytes,
            });
        }
    }

    let datasets: Vec<DatasetInfo> = by_prefix
        .into_iter()
        .map(|(p, mut views)| {
            views.sort_by(|a, b| a.view.cmp(&b.view));
            DatasetInfo {
                prefix: p,
                path: format!("s3://{bucket}/{prefix}"),
                views,
            }
        })
        .collect();

    Ok(S3Listing {
        bucket,
        prefix,
        objects,
        datasets,
    })
}

/// Cache file for `s3://bucket/key`: a folder named by a hash of the bucket
/// and key, holding the object under its file name with unsafe characters
/// replaced. Keys that differ only in `/` versus `_` or in letter case get
/// separate files, and no bucket or key can point outside the cache.
fn cache_path(root: &std::path::Path, bucket: &str, key: &str) -> std::path::PathBuf {
    let name: String = key
        .rsplit_once('/')
        .map_or(key, |(_, name)| name)
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "._-".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    root.join(format!(
        "{:016x}",
        fnv1a(format!("{bucket}/{key}").as_bytes())
    ))
    .join(name)
}

/// FNV-1a: a hash that stays the same across builds, so cache folders do too.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// Remove what the old `<bucket>/<flattened key>` layout left in the cache:
/// every top-level entry not named like a [`cache_path`] folder. Symlinks are
/// removed, not followed.
fn cleanup_legacy_cache(root: &std::path::Path) -> Result<(), String> {
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("{}: {e}", root.display())),
    };
    for entry in entries {
        let entry = entry.map_err(|e| format!("{}: {e}", root.display()))?;
        let name = entry.file_name();
        if name.to_str().is_some_and(|s| {
            s.len() == 16
                && s.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        }) {
            continue;
        }
        let path = entry.path();
        let remove = || -> std::io::Result<()> {
            if entry.file_type()?.is_dir() {
                std::fs::remove_dir_all(&path)
            } else {
                std::fs::remove_file(&path)
            }
        };
        remove().map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(())
}

/// Size of an object still to download, or `None` when it is already cached.
async fn head_size(
    client: &Client,
    bucket: &str,
    key: &str,
    local: &std::path::Path,
    token: &std::sync::atomic::AtomicBool,
) -> Result<Option<u64>, String> {
    if local.exists() {
        return Ok(None);
    }
    let head = xfers::cancellable(token, async {
        client
            .head_object()
            .bucket(bucket)
            .key(key)
            .send()
            .await
            .map_err(|e| format!("S3 head {key}: {}", DisplayErrorContext(&e)))
    })
    .await?;
    Ok(Some(head.content_length().unwrap_or(0).max(0) as u64))
}

/// One S3 object, fetched with ranged `GetObject` requests.
struct S3Source<'a> {
    client: &'a Client,
    bucket: &'a str,
    key: &'a str,
}

impl xfers::Source for S3Source<'_> {
    fn open(
        &self,
        range: Option<(u64, u64)>,
    ) -> futures::future::BoxFuture<'_, Result<xfers::Body, String>> {
        Box::pin(async move {
            let resp = self
                .client
                .get_object()
                .bucket(self.bucket)
                .key(self.key)
                .set_range(range.map(|(s, e)| format!("bytes={s}-{e}")))
                .send()
                .await
                .map_err(|e| format!("S3 get {}: {}", self.key, DisplayErrorContext(&e)))?;
            if range.is_some() && resp.content_range().is_none() {
                return Err(format!("S3 get {}: range ignored by the server", self.key));
            }
            let key = self.key.to_string();
            let body = futures::stream::try_unfold(resp.body, move |mut body| {
                let key = key.clone();
                async move {
                    let chunk = body
                        .try_next()
                        .await
                        .map_err(|e| format!("S3 get {key}: {e}"))?;
                    Ok(chunk.map(|c| (c, body)))
                }
            });
            Ok(Box::pin(body) as xfers::Body)
        })
    }
}

/// Download the given `s3://bucket/key` parquet views to the local cache so
/// the rest of the app can query them as ordinary local files. Reports live
/// progress over the `download-progress` event for each file. `connections`
/// is the user's parallel-connections setting.
#[tauri::command]
pub async fn cache_s3_views(
    app: AppHandle,
    cancel_id: String,
    bucket: String,
    paths: Vec<String>,
    connections: Option<usize>,
) -> Result<Vec<ViewInfo>, String> {
    let client = client_for_bucket(&bucket).await;
    let root = CacheKind::S3.root()?;
    // Once per run; a failed cleanup is reported and tried again next time.
    static CLEANED: Mutex<bool> = Mutex::new(false);
    {
        let mut cleaned = CLEANED.lock().unwrap();
        if !*cleaned {
            cleanup_legacy_cache(&root)?;
            *cleaned = true;
        }
    }
    let connections = xfers::connections(connections);
    cache_s3_views_impl(
        Some(&app),
        &client,
        &root,
        cancel_id,
        bucket,
        paths,
        connections,
    )
    .await
}

/// Testable core of [`cache_s3_views`]: `app` may be `None`, and `client`
/// and `root` point at a test server and folder in tests.
async fn cache_s3_views_impl(
    app: Option<&AppHandle>,
    client: &Client,
    root: &std::path::Path,
    cancel_id: String,
    bucket: String,
    paths: Vec<String>,
    connections: usize,
) -> Result<Vec<ViewInfo>, String> {
    std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let batch = xfers::begin(&cancel_id);
    let token = &batch.token;

    let mut plan = Vec::new();
    for p in &paths {
        let key = p.trim_start_matches(&format!("s3://{bucket}/")).to_string();
        let name = key.rsplit_once('/').map_or(key.as_str(), |(_, n)| n);
        let (_, view, format) =
            qpx::parse_filename(name).ok_or_else(|| format!("not a QPX view file: {p}"))?;
        let name = name.to_string();
        let local = cache_path(root, &bucket, &key);
        plan.push((key, name, view, format, local));
    }
    // Size everything still to fetch first (in parallel), so a download that
    // cannot fit on the cache disk is refused before any byte is written.
    // `None` marks a file that is already cached.
    let bucket = bucket.as_str();
    let probes: Vec<_> = plan
        .iter()
        .map(|(key, .., local)| head_size(client, bucket, key, local, token))
        .collect();
    let sizes: Vec<Option<u64>> = futures::stream::iter(probes)
        .buffered(connections)
        .try_collect()
        .await?;
    let need = sizes
        .iter()
        .flatten()
        .fold(0u64, |a, b| a.saturating_add(*b));
    xfers::ensure_space(root, need)?;

    // All files share the batch's connections; small files go first.
    let slots = xfers::Slots::new(connections);
    let mut jobs: Vec<_> = plan.into_iter().zip(sizes).enumerate().collect();
    jobs.sort_by_key(|(_, (_, size))| size.unwrap_or(0));
    let (batch, slots) = (&batch, &slots);
    let mut out = futures::future::try_join_all(jobs.into_iter().map(
        |(i, ((key, name, view, format, local), size))| async move {
            let size_bytes = match size {
                None => std::fs::metadata(&local)
                    .map_err(|e| format!("{}: {e}", local.display()))?
                    .len(),
                Some(total) => {
                    if let Some(parent) = local.parent() {
                        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                    }
                    let src = S3Source {
                        client,
                        bucket,
                        key: &key,
                    };
                    // S3 always answers ranges; a size of 0 (unknown) streams.
                    xfers::download(app, batch, slots, &src, &name, &local, total, total > 0)
                        .await?
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
    use std::collections::HashMap;
    use std::path::{Component, Path};
    use std::sync::Arc;

    #[cfg(unix)]
    #[test]
    fn legacy_cache_entries_are_removed() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("s3-cache");
        for dir in ["bucket", "ABCDEF0123456789", "0123456789abcdef"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
            std::fs::write(root.join(dir).join("A.pg.parquet"), b"x").unwrap();
        }
        std::fs::write(root.join("bucket_A.pg.parquet"), b"x").unwrap();
        let outside = tmp.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("keep"), b"x").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("link")).unwrap();
        cleanup_legacy_cache(&root).unwrap();
        let left: Vec<_> = std::fs::read_dir(&root)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(left, ["0123456789abcdef"]);
        assert!(outside.join("keep").exists());
    }

    type Requests = Arc<Mutex<Vec<String>>>;

    /// Bare S3 stand-in for path-style requests in `region`: HEAD and GET of
    /// `/<bucket>[/<key>]` answer with the object. Every request is recorded
    /// as "METHOD path".
    async fn fake_s3(
        region: &'static str,
        objects: HashMap<&'static str, Vec<u8>>,
    ) -> (Client, Requests) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let requests = Requests::default();
        let (objects, log) = (Arc::new(objects), requests.clone());
        tokio::spawn(async move {
            loop {
                let (mut sock, _) = listener.accept().await.unwrap();
                let (objects, log) = (objects.clone(), log.clone());
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 16 * 1024];
                    let n = sock.read(&mut buf).await.unwrap();
                    let head = String::from_utf8_lossy(&buf[..n]).to_string();
                    let mut line = head.split_whitespace();
                    let (method, target) = (line.next().unwrap(), line.next().unwrap());
                    let path = target.split('?').next().unwrap();
                    log.lock().unwrap().push(format!("{method} {path}"));
                    let (status, body) = match objects.get(path) {
                        Some(body) => ("200 OK", body.as_slice()),
                        None => ("404 Not Found", &[][..]),
                    };
                    // An empty region leaves the header out.
                    let region = match region {
                        "" => String::new(),
                        r => format!("x-amz-bucket-region: {r}\r\n"),
                    };
                    let mut resp = format!(
                        "HTTP/1.1 {status}\r\nContent-Length: {}\r\n{region}Connection: close\r\n\r\n",
                        body.len()
                    )
                    .into_bytes();
                    if method == "GET" {
                        resp.extend_from_slice(body);
                    }
                    sock.write_all(&resp).await.unwrap();
                });
            }
        });
        let conf = aws_sdk_s3::config::Builder::new()
            .behavior_version(BehaviorVersion::latest())
            .endpoint_url(format!("http://{addr}"))
            .force_path_style(true)
            .region(aws_config::Region::new("us-east-1"))
            .credentials_provider(aws_sdk_s3::config::Credentials::new(
                "test", "test", None, None, "test",
            ))
            .build();
        (Client::from_conf(conf), requests)
    }

    fn count(requests: &Requests, method: &str) -> usize {
        let prefix = format!("{method} ");
        requests
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.starts_with(&prefix))
            .count()
    }

    #[tokio::test]
    async fn bucket_region_is_looked_up_once() {
        for region in ["us-east-1", "eu-west-1"] {
            let objects = HashMap::from([("/bkt", Vec::new()), ("/bkt/k", b"x".to_vec())]);
            let (base, requests) = fake_s3(region, objects).await;
            let clients = Mutex::new(BTreeMap::new());
            for _ in 0..2 {
                let client = bucket_client(&clients, base.clone(), "bkt").await;
                assert_eq!(client.config().region().unwrap().as_ref(), region);
            }
            assert_eq!(count(&requests, "HEAD"), 1, "{region}");
            // The bucket's client keeps the endpoint and path-style addressing.
            let client = bucket_client(&clients, base.clone(), "bkt").await;
            client
                .get_object()
                .bucket("bkt")
                .key("k")
                .send()
                .await
                .unwrap();
            assert_eq!(requests.lock().unwrap().last().unwrap(), "GET /bkt/k");
        }
    }

    #[tokio::test]
    async fn bucket_without_a_region_is_asked_again() {
        let (base, requests) = fake_s3("", HashMap::new()).await;
        let clients = Mutex::new(BTreeMap::new());
        for _ in 0..2 {
            bucket_client(&clients, base.clone(), "bkt").await;
        }
        assert_eq!(count(&requests, "HEAD"), 2);
    }

    #[tokio::test]
    async fn downloads_each_object_once_into_its_own_file() {
        let (a, b) = (vec![1u8; 300_000], vec![2u8; 1_000]);
        let (client, requests) = fake_s3(
            "us-east-1",
            HashMap::from([
                ("/bkt/x/y_z.psm.parquet", a.clone()),
                ("/bkt/x_y/z.psm.parquet", b.clone()),
            ]),
        )
        .await;
        let tmp = tempfile::tempdir().unwrap();
        let paths = vec![
            "s3://bkt/x/y_z.psm.parquet".to_string(),
            "s3://bkt/x_y/z.psm.parquet".to_string(),
        ];
        let open = |id: &str| {
            cache_s3_views_impl(
                None,
                &client,
                tmp.path(),
                id.to_string(),
                "bkt".to_string(),
                paths.clone(),
                8,
            )
        };
        let views = open("s3-first").await.unwrap();
        assert_eq!(std::fs::read(&views[0].path).unwrap(), a);
        assert_eq!(std::fs::read(&views[1].path).unwrap(), b);
        assert_eq!(
            (views[0].view.as_str(), views[0].name.as_str()),
            ("psm", "y_z.psm.parquet")
        );
        // Opening the same views again is served from the cache.
        let again = open("s3-second").await.unwrap();
        assert_eq!(count(&requests, "GET"), 2);
        assert_eq!(again[1].size_bytes, Some(b.len() as u64));
    }

    #[test]
    fn every_key_gets_its_own_cache_file() {
        let root = Path::new("/cache");
        let a = cache_path(root, "b", "x/y_z.psm.parquet");
        let b = cache_path(root, "b", "x_y/z.psm.parquet");
        assert_ne!(a, b);
        // macOS and Windows file systems ignore letter case.
        let c = cache_path(root, "b", "X/y_z.psm.parquet");
        let lower = |p: &Path| p.to_string_lossy().to_lowercase();
        assert_ne!(lower(&a), lower(&c));
        assert_ne!(
            cache_path(root, "b1", "k.psm.parquet"),
            cache_path(root, "b2", "k.psm.parquet")
        );
    }

    #[test]
    fn cache_paths_stay_inside_the_cache() {
        let root = Path::new("/cache");
        for (bucket, key) in [
            ("../..", "x.psm.parquet"),
            ("b", "..\\..\\x.psm.parquet"),
            ("b", "../../x.psm.parquet"),
        ] {
            let p = cache_path(root, bucket, key);
            assert!(p.starts_with(root), "{}", p.display());
            assert!(
                p.components()
                    .all(|c| matches!(c, Component::RootDir | Component::Normal(_))),
                "{}",
                p.display()
            );
            assert!(!p.to_string_lossy().contains('\\'), "{}", p.display());
            assert!(
                p.to_string_lossy().ends_with(".psm.parquet"),
                "{}",
                p.display()
            );
        }
    }
}
