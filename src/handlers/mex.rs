use axum::{
    extract::{Path, State},
    Json,
};

use crate::error::ApiError;
use crate::models::mex::*;
use crate::state::AppState;

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/mex/query",
    tag = "mex",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = MexQueryRequest,
    responses(
        (status = 200, description = "GraphQL query result", body = MexApiResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn mex_query(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<MexQueryRequest>,
) -> Result<Json<MexApiResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;

    let mex_request = whatsapp_rust::MexRequest {
        doc_id: &request.doc_id,
        variables: request.variables,
    };

    let result = client
        .mex()
        .query(mex_request)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let errors = result.errors.map(|errs| {
        errs.into_iter()
            .map(|e| MexGraphQLErrorItem {
                message: e.message,
                error_code: e.extensions.as_ref().and_then(|ext| ext.error_code),
                is_retryable: e.extensions.as_ref().and_then(|ext| ext.is_retryable),
                severity: e
                    .extensions
                    .as_ref()
                    .and_then(|ext| ext.severity.clone()),
            })
            .collect()
    });

    Ok(Json(MexApiResponse {
        data: result.data,
        errors,
    }))
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/mex/mutate",
    tag = "mex",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = MexMutateRequest,
    responses(
        (status = 200, description = "GraphQL mutation result", body = MexApiResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn mex_mutate(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<MexMutateRequest>,
) -> Result<Json<MexApiResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;

    let mex_request = whatsapp_rust::MexRequest {
        doc_id: &request.doc_id,
        variables: request.variables,
    };

    let result = client
        .mex()
        .mutate(mex_request)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let errors = result.errors.map(|errs| {
        errs.into_iter()
            .map(|e| MexGraphQLErrorItem {
                message: e.message,
                error_code: e.extensions.as_ref().and_then(|ext| ext.error_code),
                is_retryable: e.extensions.as_ref().and_then(|ext| ext.is_retryable),
                severity: e
                    .extensions
                    .as_ref()
                    .and_then(|ext| ext.severity.clone()),
            })
            .collect()
    });

    Ok(Json(MexApiResponse {
        data: result.data,
        errors,
    }))
}

fn get_client(
    state: &AppState,
    session_id: &str,
) -> Result<std::sync::Arc<whatsapp_rust::Client>, ApiError> {
    let runtime = state
        .get_session(session_id)
        .ok_or(ApiError::NotConnected)?;

    runtime.get_client().ok_or(ApiError::NotConnected)
}
