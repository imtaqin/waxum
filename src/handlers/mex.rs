use axum::{
    extract::{Path, State},
    Json,
};

use crate::error::ApiError;
use crate::models::mex::*;
use crate::state::AppState;
use wacore::iq::mex::MexDoc;

/// Builds a `MexDoc` from runtime strings. The `whatsapp-rust` lib wants
/// `&'static str` for both fields (it caches doc descriptors at compile
/// time); `Box::leak` is fine here because the set of distinct mex docs
/// per process is bounded by the WA Web doc registry (~50 entries).
fn build_mex_doc(name: Option<String>, id: String, fallback_name: &'static str) -> MexDoc {
    let name_str: &'static str = match name {
        Some(s) if !s.is_empty() => Box::leak(s.into_boxed_str()),
        _ => fallback_name,
    };
    let id_str: &'static str = Box::leak(id.into_boxed_str());
    MexDoc {
        name: name_str,
        id: id_str,
    }
}

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
        doc: build_mex_doc(request.doc_name, request.doc_id, "WAWebMexCustomQuery"),
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
                severity: e.extensions.as_ref().and_then(|ext| ext.severity.clone()),
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
        doc: build_mex_doc(request.doc_name, request.doc_id, "WAWebMexCustomMutation"),
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
                severity: e.extensions.as_ref().and_then(|ext| ext.severity.clone()),
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
