//! Public "quantms portal" example source.
//!
//! The quantms portal (https://portal.quantms.org) publishes reanalysed QPX
//! datasets as public parquet files, browsable through a small HTTP service at
//! `https://browse.quantms.org`. The layout is:
//!
//! ```text
//! /quantms/datasets/<PXD-code>/<run-hash>/qpx/<PXD-code>.<view>.parquet
//! ```
//!
//! These files are freely downloadable over HTTPS (no AWS credentials needed),
//! so this module provides the app with a ready-made example source to open:
//!
//! - [`catalogue`]: the dataset registry, search and facets (JSON API);
//! - [`listing`]: the browse service's listings, which give a dataset's runs
//!   and QPX views;
//! - [`download`]: copying views into the local cache.

pub mod catalogue;
pub mod download;
pub mod listing;

const BASE: &str = "https://browse.quantms.org";
const DATASETS_ROOT: &str = "/quantms/datasets/";

/// Against the live portal: `cargo test -- --ignored`.
#[cfg(test)]
mod tests {
    use super::catalogue::registry;
    use super::download::portal_cache_views_impl;
    use super::listing::list_views;
    use crate::commands::portal_http;
    use reqwest::Client;

    /// A client configured like the app's, owned by one test's runtime.
    fn portal_client() -> Client {
        portal_http::client_builder().unwrap().build().unwrap()
    }

    #[tokio::test]
    #[ignore = "needs network access to the quantms portal"]
    async fn portal_lists_and_downloads_pxd014414() {
        let ds = "PXD014414";
        let client = portal_client();
        let views = list_views(&client, ds, None, None)
            .await
            .expect("list views");
        assert!(!views.is_empty(), "expected views for {ds}");
        assert!(
            views
                .iter()
                .any(|v| v.size_bytes.is_some_and(|size| size > 0)),
            "sizes should be > 0"
        );

        let url = views
            .iter()
            .find(|v| v.view == "dataset")
            .map(|v| v.path.clone())
            .expect("dataset view present");
        let tmp = tempfile::tempdir().unwrap();
        let cached = portal_cache_views_impl(
            None,
            &client,
            tmp.path(),
            "test-cancel".to_string(),
            vec![url],
            8,
        )
        .await
        .expect("cache");
        assert_eq!(cached.len(), 1);
        assert!(cached[0].size_bytes.is_some_and(|size| size > 0));
        assert!(std::path::Path::new(&cached[0].path).exists());
    }

    #[tokio::test]
    #[ignore = "needs network access to the quantms portal"]
    async fn stale_registry_ref_falls_back_to_a_live_run() {
        let views = list_views(&portal_client(), "IPX0001804002/0e8d80a52200", None, None)
            .await
            .expect("fallback run");
        assert!(views.iter().any(|v| v.view == "psm"));
        assert!(!views[0].path.contains("0e8d80a52200"));
    }

    #[tokio::test]
    #[ignore = "needs network access to the quantms portal"]
    async fn registry_lists_and_ref_resolves() {
        let client = portal_client();
        let rows = registry(&client, Some(100)).await.expect("registry");
        assert!(
            rows.len() >= 80,
            "expected ~83 unique datasets, got {}",
            rows.len()
        );
        let with_ref = rows
            .iter()
            .find(|r| r.dataset_ref.is_some())
            .expect("some dataset has a run ref");
        let views = list_views(
            &client,
            with_ref.dataset_ref.as_deref().unwrap(),
            None,
            None,
        )
        .await
        .expect("list views from ref");
        assert!(!views.is_empty());
        assert!(views
            .iter()
            .any(|v| v.size_bytes.is_some_and(|size| size > 0)));
    }
}
