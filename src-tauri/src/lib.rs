mod commands;
mod logging;
mod qpx;

pub use commands::stream::register::{
    register as stream_register, register_http as stream_register_http, stats as stream_stats,
};

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            use tauri::Manager;
            // Detailed from the start when asked for (QPX_LOG=debug); the
            // Diagnostics setting switches it later.
            let detailed = std::env::var("QPX_LOG").is_ok_and(|v| v.eq_ignore_ascii_case("debug"));
            logging::init(app.path().app_log_dir().ok(), detailed);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::sources::open_file,
            commands::sources::scan_collection,
            commands::io::save_export,
            commands::s3::list_s3,
            commands::s3::cache_s3_views,
            commands::stream::register::stream_s3_urls,
            commands::stream::register::stream_s3_stats,
            commands::stream::register::stream_http_urls,
            commands::stream::register::stream_local_files,
            commands::transfers::cancel_download,
            commands::transfers::cache_free_space,
            commands::portal::catalogue::portal_registry,
            commands::portal::catalogue::portal_search,
            commands::portal::catalogue::portal_facets,
            commands::portal::listing::portal_list_views,
            commands::portal::download::portal_cache_views,
            commands::peptide_search::ps_search,
            commands::peptide_search::ps_modifications,
            logging::log_read,
            logging::log_set_detailed,
            logging::log_write,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
