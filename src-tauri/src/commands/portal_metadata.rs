use super::portal::catalogue::PortalDataset;
use futures::StreamExt;
use reqwest::Client;
use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, OnceLock};
use tokio::sync::{Mutex, OnceCell};

type MetadataCache = HashMap<String, Arc<OnceCell<PortalMethods>>>;
static CACHE: OnceLock<Mutex<MetadataCache>> = OnceLock::new();
const SQL: &str = "SELECT DISTINCT json_extract_string(to_json(r), '$.acquisition_method') AS acquisition_method, json_extract_string(to_json(r), '$.samples[*].label') AS labels FROM run r";

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortalMethods {
    #[serde(default)]
    pub quantification_types: Vec<String>,
    #[serde(default)]
    pub acquisition_types: Vec<String>,
    #[serde(default)]
    pub quantification_labels: Vec<String>,
    #[serde(default)]
    pub metadata_error: Option<String>,
}

#[derive(serde::Deserialize)]
struct SqlResult {
    available: bool,
    columns: Vec<String>,
    rows: Vec<Vec<serde_json::Value>>,
    truncated: bool,
}

fn term_name(value: &str) -> &str {
    value
        .split(';')
        .find_map(|part| part.strip_prefix("NT="))
        .unwrap_or(value)
        .trim()
}

fn acquisition_type(value: &str) -> String {
    let name = term_name(value);
    let compact = name.to_lowercase().replace(['-', ' '], "");
    match compact.as_str() {
        "" => "Unknown".into(),
        "dda" | "datadependentacquisition" => "DDA".into(),
        "dia" | "dataindependentacquisition" => "DIA".into(),
        _ => name.to_string(),
    }
}

fn quantification_type(label: &str) -> &'static str {
    let name = term_name(label).to_lowercase();
    if name == "lfq" || name == "label free sample" || name == "label-free sample" {
        "Label-free"
    } else if name.starts_with("tmt") {
        "TMT"
    } else if name.starts_with("itraq") {
        "iTRAQ"
    } else if name.starts_with("silac") {
        "SILAC"
    } else {
        "Unknown"
    }
}

fn methods(result: SqlResult) -> Result<PortalMethods, String> {
    if !result.available {
        return Err("No run view is available for this dataset version".into());
    }
    if result.truncated {
        return Err(
            "Run metadata was truncated; method types could not be determined completely".into(),
        );
    }
    if result.columns != ["acquisition_method", "labels"] {
        return Err("Unexpected run metadata response columns".into());
    }
    let mut acquisition = BTreeSet::new();
    let mut quantification = BTreeSet::new();
    let mut labels = BTreeSet::new();
    for row in result.rows {
        if row.len() != 2 {
            return Err("Unexpected run metadata row shape".into());
        }
        match &row[0] {
            serde_json::Value::String(value) => {
                acquisition.insert(acquisition_type(value));
            }
            serde_json::Value::Null => {
                acquisition.insert("Unknown".into());
            }
            _ => return Err("Unexpected acquisition metadata value".into()),
        }
        match &row[1] {
            serde_json::Value::Array(values) if !values.is_empty() => {
                for value in values {
                    match value {
                        serde_json::Value::String(label) => {
                            quantification.insert(quantification_type(label).to_string());
                            labels.insert(label.clone());
                        }
                        serde_json::Value::Null => {
                            quantification.insert("Unknown".into());
                        }
                        _ => return Err("Unexpected quantification label value".into()),
                    }
                }
            }
            serde_json::Value::Null | serde_json::Value::Array(_) => {
                quantification.insert("Unknown".into());
            }
            _ => return Err("Unexpected quantification labels value".into()),
        }
    }
    if acquisition.is_empty() {
        acquisition.insert("Unknown".into());
    }
    if quantification.is_empty() {
        quantification.insert("Unknown".into());
    }
    Ok(PortalMethods {
        acquisition_types: acquisition.into_iter().collect(),
        quantification_types: quantification.into_iter().collect(),
        quantification_labels: labels.into_iter().collect(),
        metadata_error: None,
    })
}

async fn fetch_methods(client: &Client, dataset_ref: &str) -> Result<PortalMethods, String> {
    let parts: Vec<&str> = dataset_ref.split('/').collect();
    if parts.len() != 2
        || parts.iter().any(|p| {
            p.is_empty()
                || !p
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        })
    {
        return Err("Dataset version reference is unavailable or invalid".into());
    }
    let url = format!("https://api.quantms.org/qpx-explore/datasets/{dataset_ref}/sql");
    let response = client
        .post(&url)
        .timeout(std::time::Duration::from_secs(30))
        .json(&serde_json::json!({ "sql": SQL, "limit": 500 }))
        .send()
        .await
        .map_err(|e| format!("request run metadata: {e:?}"))?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(PortalMethods {
            metadata_error: Some(
                "Run metadata is unavailable for this dataset version (HTTP 404)".into(),
            ),
            ..PortalMethods::default()
        });
    }
    let result = response
        .error_for_status()
        .map_err(|e| format!("request run metadata: {e}"))?
        .json::<SqlResult>()
        .await
        .map_err(|e| format!("parse run metadata: {e}"))?;
    if !result.available {
        return Ok(PortalMethods {
            metadata_error: Some("No run view is available for this dataset version".into()),
            ..PortalMethods::default()
        });
    }
    methods(result)
}

async fn for_ref(client: &Client, dataset_ref: &str) -> PortalMethods {
    let cell = {
        let mut cache = CACHE
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .await;
        cache.entry(dataset_ref.to_string()).or_default().clone()
    };
    match cell
        .get_or_try_init(|| fetch_methods(client, dataset_ref))
        .await
    {
        Ok(metadata) => metadata.clone(),
        Err(error) => {
            log::warn!("Portal run metadata for {dataset_ref}: {error}");
            PortalMethods {
                metadata_error: Some(error),
                ..PortalMethods::default()
            }
        }
    }
}

pub async fn enrich(datasets: &mut [PortalDataset], client: &Client) {
    futures::stream::iter(datasets)
        .for_each_concurrent(4, |dataset| async {
            dataset.methods = match dataset.dataset_ref.as_deref() {
                Some(dataset_ref) => for_ref(client, dataset_ref).await,
                None => PortalMethods {
                    metadata_error: Some("Dataset version reference is unavailable".into()),
                    ..PortalMethods::default()
                },
            };
        })
        .await;
}

pub async fn apply_cached(datasets: &mut [PortalDataset]) {
    let cache = CACHE
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .await;
    for dataset in datasets {
        if let Some(metadata) = dataset
            .dataset_ref
            .as_ref()
            .and_then(|id| cache.get(id))
            .and_then(|cell| cell.get())
        {
            dataset.methods = metadata.clone();
        }
    }
}

#[tauri::command]
pub async fn portal_method_metadata(
    dataset_refs: Vec<String>,
) -> Result<HashMap<String, PortalMethods>, String> {
    let client = super::portal_http::client()?;
    let refs: BTreeSet<String> = dataset_refs.into_iter().collect();
    Ok(futures::stream::iter(refs)
        .map(|id| {
            let client = &client;
            async move {
                let methods = for_ref(client, &id).await;
                (id, methods)
            }
        })
        .buffer_unordered(4)
        .collect()
        .await)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(rows: serde_json::Value) -> SqlResult {
        serde_json::from_value(serde_json::json!({
            "available": true,
            "columns": ["acquisition_method", "labels"],
            "rows": rows,
            "truncated": false,
        }))
        .unwrap()
    }

    #[test]
    fn portal_metadata_unions_all_runs_and_normalizes_declared_types() {
        let data = methods(result(serde_json::json!([
            ["Data-dependent acquisition", ["TMT126", "TMT127N"]],
            ["Data-Dependent Acquisition", ["LFQ"]],
            [
                "NT=data-independent acquisition;AC=MS:1003221",
                ["iTRAQ114", "SILAC heavy"]
            ],
        ])))
        .unwrap();
        assert_eq!(data.acquisition_types, ["DDA", "DIA"]);
        assert_eq!(
            data.quantification_types,
            ["Label-free", "SILAC", "TMT", "iTRAQ"]
        );
        assert_eq!(data.quantification_labels.len(), 5);
    }

    #[test]
    fn portal_metadata_never_guesses_missing_or_unrecognized_labels_are_label_free() {
        let data = methods(result(serde_json::json!([
            [null, ["unrecognized", null]],
            ["", []],
            ["DDA", ["NT=label free sample;AC=MS:1002038"]],
        ])))
        .unwrap();
        assert_eq!(data.acquisition_types, ["DDA", "Unknown"]);
        assert_eq!(data.quantification_types, ["Label-free", "Unknown"]);
        assert_eq!(
            methods(result(serde_json::json!([])))
                .unwrap()
                .quantification_types,
            ["Unknown"]
        );
    }

    #[test]
    fn portal_metadata_rejects_unavailable_truncated_and_malformed_responses() {
        let mut unavailable = result(serde_json::json!([]));
        unavailable.available = false;
        assert!(methods(unavailable).is_err());
        let mut truncated = result(serde_json::json!([]));
        truncated.truncated = true;
        assert!(methods(truncated).is_err());
        assert!(methods(result(serde_json::json!([["DDA", "not a list"]]))).is_err());
    }

    #[tokio::test]
    #[ignore = "queries public quantms run metadata"]
    async fn portal_metadata_live_projection_handles_current_and_older_run_schemas() {
        let client = super::super::portal_http::client_builder()
            .unwrap()
            .build()
            .unwrap();
        let tmt = fetch_methods(&client, "PXD023662/5a9ea0c090e2")
            .await
            .unwrap();
        assert_eq!(tmt.acquisition_types, ["DDA"]);
        assert_eq!(tmt.quantification_types, ["TMT"]);
        let older = fetch_methods(&client, "PXD021013/1ed4852abc60")
            .await
            .unwrap();
        assert_eq!(older.acquisition_types, ["Unknown"]);
    }

    #[tokio::test]
    #[ignore = "enriches the public quantms catalogue and verifies combined filtering"]
    async fn portal_metadata_live_catalog_filtering() {
        let mut datasets = super::super::portal::catalogue::portal_registry(Some(100))
            .await
            .unwrap();
        let client = super::super::portal_http::client_builder()
            .unwrap()
            .build()
            .unwrap();
        enrich(&mut datasets, &client).await;
        assert!(datasets.len() > 100);
        assert!(datasets
            .iter()
            .any(|d| d.methods.quantification_types.contains(&"TMT".to_string())));
        let cached = super::super::portal::catalogue::portal_registry(Some(100))
            .await
            .unwrap();
        assert!(cached
            .iter()
            .any(|d| d.methods.quantification_types.contains(&"TMT".to_string())));
        assert!(cached
            .iter()
            .any(|d| d.methods.acquisition_types.contains(&"DDA".to_string())));
        let result = super::super::portal::catalogue::portal_search(
            None,
            None,
            None,
            None,
            None,
            Some("TMT".into()),
            Some("DDA".into()),
            Some("peptides".into()),
            Some(1),
            Some(10),
        )
        .await
        .unwrap();
        assert!(result.total > 0);
        assert!(result.datasets.iter().all(|d| d
            .methods
            .quantification_types
            .contains(&"TMT".to_string())
            && d.methods.acquisition_types.contains(&"DDA".to_string())));
        assert_eq!(result.datasets.len(), result.total.min(10));
    }
}
