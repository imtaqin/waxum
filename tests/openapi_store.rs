//! Every route in `src/api/shared/store/routes.rs` must be documented, or it
//! is invisible in Swagger UI (issue #171).
use utoipa::OpenApi;

#[test]
fn store_routes_are_documented() {
    let doc = serde_json::to_value(waxum::api::openapi::ApiDoc::openapi()).unwrap();
    let paths = doc["paths"].as_object().unwrap();
    for suffix in [
        "store/chats",
        "store/chats/{jid}",
        "store/chats/{jid}/messages",
        "store/chats/{jid}/messages/{id}",
        "store/chats/{jid}/messages/{id}/reactions",
        "store/chats/{jid}/messages/{id}/receipts",
        "store/chats/{jid}/kinds",
        "store/chats/{jid}/links",
        "store/recent",
        "store/starred",
        "store/search",
        "store/contacts",
        "store/contacts/{jid}",
        "store/events",
    ] {
        let path = format!("/api/v1/sessions/{{session_id}}/{suffix}");
        assert!(paths.contains_key(&path), "missing from OpenAPI: {path}");
    }
}
