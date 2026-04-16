use axum::extract::FromRequestParts;
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use sha2::{Digest, Sha256};

use crate::db::models;
use crate::state::AppState;

/// Bearer token extractor for API routes.
/// Validates the token via allowthem, then checks app scoping via substrukt's app_tokens table.
pub struct BearerToken {
    pub user: allowthem_core::User,
    pub role: String,
    pub app_id: Option<i64>,
}

impl FromRequestParts<AppState> for BearerToken {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let unauthorized = || {
            (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"error": "Unauthorized"})),
            )
                .into_response()
        };

        // Extract bearer token
        let auth_header = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .ok_or_else(unauthorized)?;

        let raw_token = auth_header
            .strip_prefix("Bearer ")
            .ok_or_else(unauthorized)?;

        // Validate via allowthem
        let user_id = state
            .ath
            .db()
            .validate_api_token(raw_token)
            .await
            .map_err(|_| unauthorized())?
            .ok_or_else(unauthorized)?;

        let user = state
            .ath
            .db()
            .get_user(user_id)
            .await
            .map_err(|_| unauthorized())?;

        if !user.is_active {
            return Err(unauthorized());
        }

        // Look up app scoping via token hash
        let hash = hex::encode(Sha256::digest(raw_token.as_bytes()));
        let app_id = models::find_app_for_token_hash(&state.pool, &hash)
            .await
            .unwrap_or(None)
            .map(|(_, aid)| aid);

        let role = crate::auth::resolve_user_role(state, &user.id).await;

        Ok(BearerToken { user, role, app_id })
    }
}

pub fn require_api_role(token: &BearerToken, min_role: &str) -> Result<(), Response> {
    let role_level = |r: &str| -> u8 {
        match r {
            "admin" => 3,
            "editor" => 2,
            "viewer" => 1,
            _ => 0,
        }
    };
    if role_level(&token.role) >= role_level(min_role) {
        Ok(())
    } else {
        Err((
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": "Forbidden"})),
        )
            .into_response())
    }
}

pub fn require_token_app(token: &BearerToken, app_id: i64) -> Result<(), Response> {
    match token.app_id {
        Some(tid) if tid == app_id => Ok(()),
        _ => Err((
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": "Token not scoped to this app"})),
        )
            .into_response()),
    }
}
