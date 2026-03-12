use axum::extract::{Path, State};
use axum::response::{IntoResponse, Redirect};
use tower_sessions::Session;

use crate::auth;
use crate::state::AppState;

pub fn routes() -> axum::Router<AppState> {
    axum::Router::new().route("/{environment}", axum::routing::post(publish))
}

async fn publish(
    State(state): State<AppState>,
    session: Session,
    Path(environment): Path<String>,
) -> impl IntoResponse {
    if !matches!(environment.as_str(), "staging" | "production") {
        return Redirect::to("/").into_response();
    }

    let label = if environment == "staging" {
        "Staging build"
    } else {
        "Production publish"
    };

    match crate::webhooks::fire_webhook(
        &state.http_client,
        &state.audit,
        &state.config,
        &environment,
        crate::webhooks::TriggerSource::Manual,
    )
    .await
    {
        Ok(true) => {
            auth::set_flash(&session, "success", &format!("{label} triggered")).await;
        }
        Ok(false) => {
            auth::set_flash(&session, "error", "Webhook URL not configured").await;
        }
        Err(e) => {
            tracing::warn!("Webhook failed for {environment}: {e}");
            auth::set_flash(&session, "error", "Webhook failed \u{2014} check configuration")
                .await;
        }
    }

    Redirect::to("/").into_response()
}
