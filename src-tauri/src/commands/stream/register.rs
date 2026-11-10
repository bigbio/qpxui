//! Registering files with the stream proxy: each gets a 127.0.0.1 URL that
//! DuckDB can range-read. Also the Tauri commands and the per-file stats.

use super::fetch::http_range;
use super::{proxy, Object, Proxy, Source, PROXY};
use crate::commands::portal_http;
use crate::commands::s3::client_for_bucket;
use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamHandle {
    pub url: String,
    pub size_bytes: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamStat {
    pub size_bytes: u64,
    pub fetched_bytes: u64,
    pub requests: u64,
}

fn object_id(path: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut h);
    format!("{:016x}", h.finish())
}

/// The proxy URL of a registered object. Only the token and the id route a
/// request; the last segment names the file, kept URL-safe.
fn handle_for(p: &Proxy, id: &str, path: &str, size: u64) -> StreamHandle {
    StreamHandle {
        url: format!(
            "http://127.0.0.1:{}/{}/{id}/{}",
            p.port,
            p.token,
            url_name(path)
        ),
        size_bytes: size,
    }
}

fn url_name(path: &str) -> String {
    let name = path.rsplit_once(['/', '\\']).map_or(path, |(_, name)| name);
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "._-".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Register `s3://bucket/key` views with the local stream proxy and return a
/// 127.0.0.1 URL per view that DuckDB can range-read. Only a HEAD request is
/// made here; the data itself is fetched lazily as queries need it.
pub async fn register(
    bucket: &str,
    paths: &[String],
) -> Result<BTreeMap<String, StreamHandle>, String> {
    let p = proxy().await?;
    let client = client_for_bucket(bucket).await;
    let mut out = BTreeMap::new();
    for path in paths {
        let key = path
            .trim_start_matches(&format!("s3://{bucket}/"))
            .to_string();
        let id = object_id(path);
        let existing = p.objects.lock().unwrap().get(&id).cloned();
        let obj = match existing {
            Some(o) => o,
            None => {
                let head = client
                    .head_object()
                    .bucket(bucket)
                    .key(&key)
                    .send()
                    .await
                    .map_err(|e| {
                        format!(
                            "S3 head {key}: {}",
                            aws_sdk_s3::error::DisplayErrorContext(e)
                        )
                    })?;
                let o = Arc::new(Object {
                    path: path.clone(),
                    size: head.content_length().unwrap_or(0).max(0) as u64,
                    last_modified: head
                        .last_modified()
                        .and_then(|d| d.fmt(aws_sdk_s3::primitives::DateTimeFormat::HttpDate).ok()),
                    source: Source::S3 {
                        client: client.clone(),
                        bucket: bucket.to_string(),
                        key: key.clone(),
                    },
                    fetched: AtomicU64::new(0),
                    requests: AtomicU64::new(0),
                    layout: Default::default(),
                });
                log::info!("streaming {} ({} MB)", o.path, o.size >> 20);
                p.objects.lock().unwrap().insert(id.clone(), o.clone());
                o
            }
        };
        out.insert(path.clone(), handle_for(&p, &id, path, obj.size));
    }
    Ok(out)
}

/// Register public HTTPS files (quantms portal) with the stream proxy. Only a
/// 1-byte range GET is made per file, to learn its size; data is fetched lazily.
pub async fn register_http(urls: &[String]) -> Result<BTreeMap<String, StreamHandle>, String> {
    let p = proxy().await?;
    let client = portal_http::stream_client()?;
    let mut out = BTreeMap::new();
    for url in urls {
        // Only the quantms portal: the proxy must not fetch arbitrary hosts.
        if !url.starts_with("https://browse.quantms.org/") {
            return Err(format!(
                "{url}: only https://browse.quantms.org files can be streamed"
            ));
        }
        let id = object_id(url);
        let existing = p.objects.lock().unwrap().get(&id).cloned();
        let obj = match existing {
            Some(o) => o,
            None => {
                let resolved = Mutex::new(None);
                let resp = http_range(&client, url, &resolved, "bytes=0-0").await?;
                let h = resp.headers();
                let total = h
                    .get(reqwest::header::CONTENT_RANGE)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.rsplit('/').next())
                    .and_then(|v| v.parse::<u64>().ok())
                    .or_else(|| {
                        (resp.status().as_u16() == 200)
                            .then(|| resp.content_length())
                            .flatten()
                    })
                    .ok_or_else(|| format!("{url}: server does not support range requests"))?;
                let last_modified = h
                    .get(reqwest::header::LAST_MODIFIED)
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_string);
                let o = Arc::new(Object {
                    path: url.clone(),
                    size: total,
                    last_modified,
                    source: Source::Http {
                        client: client.clone(),
                        url: url.clone(),
                        resolved,
                    },
                    fetched: AtomicU64::new(0),
                    requests: AtomicU64::new(0),
                    layout: Default::default(),
                });
                log::info!("streaming {} ({} MB)", o.path, o.size >> 20);
                p.objects.lock().unwrap().insert(id.clone(), o.clone());
                o
            }
        };
        out.insert(url.clone(), handle_for(&p, &id, url, obj.size));
    }
    Ok(out)
}

/// Register local parquet files with the proxy so DuckDB can range-read them
/// lazily. Only regular `.parquet` files are accepted. A file that changed on
/// disk since it was registered (e.g. re-downloaded) gets a fresh entry.
pub async fn register_local(paths: &[String]) -> Result<BTreeMap<String, StreamHandle>, String> {
    let p = proxy().await?;
    let mut out = BTreeMap::new();
    for path in paths {
        let pb = std::path::PathBuf::from(path);
        let is_parquet = pb
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("parquet"));
        if !is_parquet {
            return Err(format!("{path}: only .parquet files can be opened lazily"));
        }
        let meta = std::fs::metadata(&pb).map_err(|e| format!("{path}: {e}"))?;
        if !meta.is_file() {
            return Err(format!("{path}: not a file"));
        }
        let last_modified = meta.modified().ok().map(httpdate::fmt_http_date);
        let id = object_id(&format!("file://{path}"));
        let fresh = p
            .objects
            .lock()
            .unwrap()
            .get(&id)
            .is_some_and(|o| o.size == meta.len() && o.last_modified == last_modified);
        if !fresh {
            let o = Arc::new(Object {
                path: path.clone(),
                size: meta.len(),
                last_modified,
                source: Source::Local { path: pb.clone() },
                fetched: AtomicU64::new(0),
                requests: AtomicU64::new(0),
                layout: Default::default(),
            });
            p.objects.lock().unwrap().insert(id.clone(), o);
        }
        out.insert(path.clone(), handle_for(&p, &id, path, meta.len()));
    }
    Ok(out)
}

/// Bytes actually pulled per streamed view (keyed by its `s3://` / https path).
pub fn stats() -> BTreeMap<String, StreamStat> {
    let Some(p) = PROXY.get() else {
        return BTreeMap::new();
    };
    let objects = p.objects.lock().unwrap();
    objects
        .values()
        .filter(|o| !matches!(o.source, Source::Local { .. }))
        .map(|o| {
            (
                o.path.clone(),
                StreamStat {
                    size_bytes: o.size,
                    fetched_bytes: o.fetched.load(Ordering::Relaxed),
                    requests: o.requests.load(Ordering::Relaxed),
                },
            )
        })
        .collect()
}

#[tauri::command]
pub async fn stream_s3_urls(
    bucket: String,
    paths: Vec<String>,
) -> Result<BTreeMap<String, StreamHandle>, String> {
    register(&bucket, &paths).await
}

#[tauri::command]
pub async fn stream_http_urls(urls: Vec<String>) -> Result<BTreeMap<String, StreamHandle>, String> {
    register_http(&urls).await
}

#[tauri::command]
pub async fn stream_local_files(
    paths: Vec<String>,
) -> Result<BTreeMap<String, StreamHandle>, String> {
    register_local(&paths).await
}

#[tauri::command]
pub fn stream_s3_stats() -> BTreeMap<String, StreamStat> {
    stats()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_names_are_url_safe() {
        assert_eq!(
            url_name("s3://b/dir/a b#1?.psm.parquet"),
            "a_b_1_.psm.parquet"
        );
        assert_eq!(url_name(r"C:\data\x y.parquet"), "x_y.parquet");
        assert_eq!(url_name("/data/PXD1.pg.parquet"), "PXD1.pg.parquet");
    }

    #[tokio::test]
    async fn register_local_rejects_non_parquet() {
        let err = register_local(&["/etc/hosts".to_string()])
            .await
            .unwrap_err();
        assert!(err.contains("only .parquet"));
    }

    /// Live benchmark: native DuckDB on one thread (like duckdb-wasm) reading
    /// a 41.8 GB portal PSM file through the proxy. Needs the `duckdb` CLI.
    /// `cargo test live_stream_scan -- --ignored --nocapture`
    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn live_stream_scan() {
        let _ = env_logger::builder().is_test(true).try_init();
        let url = "https://browse.quantms.org/quantms/datasets/HuiYan-E480/392295b50151/HuiYan-E480-P1.psm.parquet".to_string();
        let handles = register_http(std::slice::from_ref(&url)).await.unwrap();
        let proxied = handles[&url].url.clone();
        // `QPX_SQL` overrides the query; `{t}` stands for the proxied file.
        let query = std::env::var("QPX_SQL").unwrap_or_else(|_| {
            "SELECT charge, count(*) FROM {t} WHERE NOT is_decoy GROUP BY 1 ORDER BY 1;".into()
        });
        let sql = format!(
            "SET threads=1; SET enable_http_metadata_cache=false; {}",
            query.replace("{t}", &format!("read_parquet('{proxied}')"))
        );
        let t = std::time::Instant::now();
        let out = tokio::task::spawn_blocking(move || {
            std::process::Command::new("duckdb")
                .arg("-c")
                .arg(&sql)
                .output()
                .unwrap()
        })
        .await
        .unwrap();
        let secs = t.elapsed().as_secs_f64();
        println!("{}", String::from_utf8_lossy(&out.stdout));
        eprintln!("{}", String::from_utf8_lossy(&out.stderr));
        let stats = stats();
        let s = &stats[&url];
        println!(
            "charge scan: {secs:.1}s, {} requests, {:.1} MB fetched",
            s.requests,
            s.fetched_bytes as f64 / 1e6
        );
        assert!(out.status.success());
    }
}
