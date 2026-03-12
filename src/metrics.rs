use std::time::Instant;

use axum::{
    body::Body,
    extract::{MatchedPath, State},
    http::Request,
    middleware::Next,
    response::{IntoResponse, Response},
};
use metrics::{counter, gauge, histogram};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};

use crate::state::AppState;

pub fn setup_recorder() -> PrometheusHandle {
    PrometheusBuilder::new()
        .install_recorder()
        .expect("failed to install Prometheus recorder")
}

pub async fn track_metrics(
    matched_path: Option<MatchedPath>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let method = req.method().clone();
    let path = matched_path
        .map(|p| p.as_str().to_string())
        .unwrap_or_else(|| req.uri().path().to_string());

    gauge!("http_connections_active").increment(1);
    let start = Instant::now();

    let response = next.run(req).await;

    let duration = start.elapsed().as_secs_f64();
    let status = response.status().as_u16().to_string();

    counter!("http_requests_total", "method" => method.to_string(), "path" => path.clone(), "status" => status).increment(1);
    histogram!("http_request_duration_seconds", "method" => method.to_string(), "path" => path).record(duration);
    gauge!("http_connections_active").decrement(1);

    response
}

pub async fn metrics_handler(State(state): State<AppState>) -> impl IntoResponse {
    // Update content and upload gauges before rendering
    update_content_gauges(&state);
    state.metrics_handle.render()
}

fn update_content_gauges(state: &AppState) {
    let schemas = crate::schema::list_schemas(&state.config.schemas_dir()).unwrap_or_default();
    for schema in &schemas {
        let entries = crate::content::list_entries(&state.config.content_dir(), schema)
            .map(|e| e.len())
            .unwrap_or(0);
        gauge!("content_entries_total", "schema" => schema.meta.slug.clone()).set(entries as f64);
    }

    let uploads_dir = state.config.uploads_dir();
    if uploads_dir.exists() {
        let mut count: u64 = 0;
        let mut total_size: u64 = 0;
        if let Ok(entries) = std::fs::read_dir(&uploads_dir) {
            for prefix_entry in entries.flatten() {
                if prefix_entry.path().is_dir() {
                    if let Ok(files) = std::fs::read_dir(prefix_entry.path()) {
                        for file in files.flatten() {
                            let path = file.path();
                            if path.is_file()
                                && !path
                                    .file_name()
                                    .is_some_and(|n| n.to_string_lossy().ends_with(".meta.json"))
                            {
                                count += 1;
                                total_size += file.metadata().map(|m| m.len()).unwrap_or(0);
                            }
                        }
                    }
                }
            }
        }
        gauge!("uploads_total").set(count as f64);
        gauge!("uploads_size_bytes").set(total_size as f64);
    }
}
