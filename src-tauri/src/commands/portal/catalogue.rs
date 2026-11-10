//! The portal's dataset catalogue API: the registry, search and facets.

use crate::commands::portal_http;
use reqwest::Client;

/// Registry metadata for a reanalysed dataset on the quantms portal.
///
/// The source API uses snake_case keys; the app's frontend uses camelCase, so
/// deserialization reads the snake_case fields and serialization emits camelCase.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PortalDataset {
    pub accession: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub organism: Option<String>,
    #[serde(default)]
    pub instrument: Option<String>,
    #[serde(default)]
    pub samples: Option<u64>,
    #[serde(default)]
    pub proteins: Option<u64>,
    #[serde(default)]
    pub peptides: Option<u64>,
    #[serde(default, rename(deserialize = "psm_count", serialize = "psmCount"))]
    pub psm_count: Option<u64>,
    #[serde(default, rename(deserialize = "total_size", serialize = "totalSize"))]
    pub total_size: Option<u64>,
    /// `accession/run-hash`, used to resolve the view URLs directly.
    #[serde(default, rename(deserialize = "dataset_ref", serialize = "datasetRef"))]
    pub dataset_ref: Option<String>,
    #[serde(default, rename(deserialize = "px_url", serialize = "pxUrl"))]
    pub px_url: Option<String>,
    #[serde(
        default,
        rename(deserialize = "collection_title", serialize = "collectionTitle")
    )]
    pub collection_title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub keywords: Option<Vec<String>>,
    #[serde(default)]
    pub tissues: Option<Vec<String>>,
    #[serde(default)]
    pub publication: Option<Publication>,
    #[serde(default)]
    pub source: Option<String>,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Publication {
    #[serde(default)]
    pub doi: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct DatasetSearchPage {
    #[serde(default)]
    total_pages: usize,
    #[serde(default)]
    datasets: Vec<PortalDataset>,
}

/// List the searchable registry of reanalysed datasets from the quantms portal
/// (fetched from `api.quantms.org/dataset-search/datasets`, paging through all
/// results). This gives a rich, searchable catalogue rather than a bare list.
#[tauri::command]
pub async fn portal_registry(page_size: Option<usize>) -> Result<Vec<PortalDataset>, String> {
    registry(&portal_http::client()?, page_size).await
}

/// GET JSON, retrying transport and server failures after 0.5 s and 1 s.
async fn get_json<T: serde::de::DeserializeOwned>(client: &Client, url: &str) -> Result<T, String> {
    let mut last = "no attempt".to_string();
    for attempt in 0..3 {
        if attempt > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(500 * attempt)).await;
        }
        match client.get(url).send().await {
            Ok(resp) => {
                if let Err(e) = resp.error_for_status_ref() {
                    last = format!("http: {e}");
                    if resp.status().is_server_error() {
                        continue;
                    }
                    break;
                }
                match resp.bytes().await {
                    Ok(body) => {
                        return serde_json::from_slice(&body)
                            .map_err(|e| format!("{url}: parse: {e}"))
                    }
                    Err(e) => last = format!("body: {e}"),
                }
            }
            Err(e) => {
                last = format!("request: {e:?}");
                // Connect, timeout and transport errors are request errors;
                // builder and redirect errors would fail the same way again.
                if !e.is_request() {
                    break;
                }
            }
        }
    }
    Err(format!("{url}: {last}"))
}

pub(super) async fn registry(
    client: &Client,
    page_size: Option<usize>,
) -> Result<Vec<PortalDataset>, String> {
    let page_size = page_size.unwrap_or(50).clamp(1, 100);

    let first: DatasetSearchPage = get_json(
        client,
        &format!("https://api.quantms.org/dataset-search/datasets?page_size={page_size}&page=1"),
    )
    .await?;
    let total_pages = first.total_pages.max(1);
    let mut all = first.datasets;
    for p in 2..=total_pages {
        let page: DatasetSearchPage = get_json(
            client,
            &format!(
                "https://api.quantms.org/dataset-search/datasets?page_size={page_size}&page={p}"
            ),
        )
        .await?;
        all.extend(page.datasets);
    }
    // One accession can hold several runs (e.g. PXD014877 has one per
    // organism), so the run ref, not the accession, identifies an entry.
    Ok(dedup_runs(all))
}

/// One entry per run. A run listed in several collections can appear more
/// than once, with the counts filled in only in some entries: keep the one
/// that knows the most.
fn dedup_runs(mut all: Vec<PortalDataset>) -> Vec<PortalDataset> {
    let key = |d: &PortalDataset| d.dataset_ref.clone().unwrap_or_else(|| d.accession.clone());
    let known = |d: &PortalDataset| {
        [d.samples, d.proteins, d.peptides, d.psm_count]
            .iter()
            .map(|v| v.unwrap_or(0))
            .fold(0u64, u64::saturating_add)
    };
    all.sort_by(|a, b| {
        a.accession
            .cmp(&b.accession)
            .then_with(|| key(a).cmp(&key(b)))
            .then_with(|| known(b).cmp(&known(a)))
    });
    all.dedup_by(|later, first| key(later) == key(first));
    all
}

/// One page of search results from the quantms portal dataset catalogue.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortalSearchResult {
    pub datasets: Vec<PortalDataset>,
    pub total: usize,
    pub page: usize,
    pub total_pages: usize,
    pub sort: String,
}

#[derive(Debug, serde::Deserialize)]
struct SearchPageRaw {
    #[serde(default)]
    datasets: Vec<PortalDataset>,
    #[serde(default)]
    total: usize,
    #[serde(default)]
    page: usize,
    #[serde(default)]
    total_pages: usize,
    #[serde(default)]
    sort: String,
}

/// Facet options shown in the discover UI. Organisms and instruments come from
/// `/facets`, collections from `/collections` (name + title).
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortalFacets {
    pub organisms: Vec<String>,
    pub instruments: Vec<String>,
    pub collections: Vec<CollectionOption>,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionOption {
    pub name: String,
    #[serde(default)]
    pub title: Option<String>,
}

/// Server-side search over the quantms portal dataset catalogue. Only the
/// filters the registry API actually supports are forwarded (`q`, collection,
/// organism, instrument, sort); count/lower-bound filters are not available
/// server-side, so the caller combines this with client-side sorting if needed.
#[tauri::command]
pub async fn portal_search(
    query: Option<String>,
    collection: Option<String>,
    organism: Option<String>,
    instrument: Option<String>,
    sort: Option<String>,
    page: Option<usize>,
    page_size: Option<usize>,
) -> Result<PortalSearchResult, String> {
    let client = portal_http::client()?;

    let mut url = reqwest::Url::parse("https://api.quantms.org/dataset-search/datasets")
        .map_err(|e| format!("bad search URL: {e}"))?;
    {
        let mut qp = url.query_pairs_mut();
        let mut add = |param: &str, value: Option<&str>| {
            if let Some(v) = value.filter(|v| !v.trim().is_empty()) {
                qp.append_pair(param, v);
            }
        };
        add("q", query.as_deref());
        add("collection", collection.as_deref());
        add("organism", organism.as_deref());
        add("instrument", instrument.as_deref());
        add("sort", sort.as_deref());
        qp.append_pair("page", &page.unwrap_or(1).max(1).to_string());
        qp.append_pair(
            "page_size",
            &page_size.unwrap_or(50).clamp(1, 100).to_string(),
        );
    }

    let raw = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("request: {e:?}"))?
        .error_for_status()
        .map_err(|e| format!("http: {e}"))?
        .json::<SearchPageRaw>()
        .await
        .map_err(|e| format!("parse: {e}"))?;

    Ok(PortalSearchResult {
        datasets: raw.datasets,
        total: raw.total,
        page: raw.page,
        total_pages: raw.total_pages.max(1),
        sort: raw.sort,
    })
}

/// Facet options (organisms, instruments, collections) for the discover UI.
#[tauri::command]
pub async fn portal_facets() -> Result<PortalFacets, String> {
    let client = portal_http::client()?;

    let values = |raw: &serde_json::Value, bucket: &str| -> Vec<String> {
        raw.get(bucket)
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|f| f.get("value").and_then(|v| v.as_str()))
                    .map(|s| s.to_string())
                    .collect()
            })
            .unwrap_or_default()
    };

    let fac: serde_json::Value =
        get_json(&client, "https://api.quantms.org/dataset-search/facets").await?;
    let mut organisms = values(&fac, "organism");
    let mut instruments = values(&fac, "instrument");
    organisms.sort();
    instruments.sort();

    #[derive(serde::Deserialize)]
    struct CollectionsRaw {
        #[serde(default)]
        collections: Vec<CollectionOption>,
    }
    let colls: CollectionsRaw = get_json(
        &client,
        "https://api.quantms.org/dataset-search/collections",
    )
    .await?;

    Ok(PortalFacets {
        organisms,
        instruments,
        collections: colls.collections,
    })
}

#[cfg(test)]
mod tests {

    #[test]
    fn duplicate_runs_keep_the_entry_with_counts() {
        let ds = |acc: &str, run: &str, psms: Option<u64>| {
            let mut d: PortalDataset =
                serde_json::from_value(serde_json::json!({ "accession": acc })).unwrap();
            d.dataset_ref = Some(format!("{acc}/{run}"));
            d.psm_count = psms;
            d
        };
        let out = dedup_runs(vec![
            ds("PXD2", "a", None),
            ds("PXD1", "r", None),
            ds("PXD2", "a", Some(4_290_071)),
            ds("PXD2", "b", None),
        ]);
        let refs: Vec<_> = out.iter().map(|d| d.dataset_ref.clone().unwrap()).collect();
        assert_eq!(refs, ["PXD1/r", "PXD2/a", "PXD2/b"]);
        assert_eq!(out[1].psm_count, Some(4_290_071));
    }

    use super::*;

    async fn json_case(
        responses: Vec<(&str, &str)>,
    ) -> (Result<serde_json::Value, String>, Vec<std::time::Instant>) {
        use std::sync::{Arc, Mutex};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/fixture", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let seen = requests.clone();
        let responses: Vec<_> = responses
            .into_iter()
            .map(|(s, b)| (s.to_string(), b.to_string()))
            .collect();
        let server = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut buf = [0u8; 4096];
                let _ = socket.read(&mut buf).await;
                let index = {
                    let mut seen = seen.lock().unwrap();
                    seen.push(std::time::Instant::now());
                    seen.len() - 1
                };
                let (status, body) = &responses[index.min(responses.len() - 1)];
                if status == "disconnect" {
                    continue;
                }
                let length = body.len() + if status == "short body" { 10 } else { 0 };
                let status = if status == "short body" {
                    "200 OK"
                } else {
                    status
                };
                let reply = format!("HTTP/1.1 {status}\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n{body}");
                socket.write_all(reply.as_bytes()).await.unwrap();
            }
        });
        let result = get_json(&Client::builder().no_proxy().build().unwrap(), &url).await;
        server.abort();
        if let Err(e) = &result {
            assert!(e.starts_with(&format!("{url}: ")), "{e}");
        }
        let seen = requests.lock().unwrap().clone();
        (result, seen)
    }

    #[tokio::test]
    async fn json_does_not_retry_client_errors_or_invalid_json() {
        for reply in [("404 Not Found", "missing"), ("200 OK", "invalid json")] {
            let (result, seen) = json_case(vec![reply]).await;
            assert!(result.is_err());
            assert_eq!(seen.len(), 1, "{reply:?}");
        }
    }

    #[tokio::test]
    async fn json_retries_server_errors_with_backoff() {
        for status in ["500 Internal Server Error", "503 Service Unavailable"] {
            let (result, seen) = json_case(vec![(status, "unavailable")]).await;
            assert!(result
                .unwrap_err()
                .contains(status.split_whitespace().next().unwrap()));
            assert_eq!(seen.len(), 3);
            assert!(seen[1].duration_since(seen[0]) >= std::time::Duration::from_millis(450));
            assert!(seen[2].duration_since(seen[1]) >= std::time::Duration::from_millis(900));
        }
    }

    #[tokio::test]
    async fn json_recovers_on_second_attempt() {
        for first in [
            ("503 Service Unavailable", "unavailable"),
            ("short body", "{"),
            ("disconnect", ""),
        ] {
            let (result, seen) = json_case(vec![first, ("200 OK", "{\"ok\":true}")]).await;
            assert_eq!(result.unwrap(), serde_json::json!({"ok": true}));
            assert_eq!(seen.len(), 2, "{first:?}");
        }
    }

    #[test]
    fn search_results_read_snake_case_and_send_camel_case() {
        let raw: SearchPageRaw = serde_json::from_value(serde_json::json!({
            "datasets": [{ "accession": "PXD1", "psm_count": 5, "dataset_ref": "PXD1/ab" }],
            "total": 1,
            "page": 1,
            "total_pages": 1,
            "sort": "samples"
        }))
        .unwrap();
        let result = PortalSearchResult {
            datasets: raw.datasets,
            total: raw.total,
            page: raw.page,
            total_pages: raw.total_pages,
            sort: raw.sort,
        };
        let out = serde_json::to_value(result).unwrap();
        assert_eq!(out["totalPages"], 1);
        assert_eq!(out["sort"], "samples");
        assert_eq!(out["datasets"][0]["psmCount"], 5);
        assert_eq!(out["datasets"][0]["datasetRef"], "PXD1/ab");
    }
}
