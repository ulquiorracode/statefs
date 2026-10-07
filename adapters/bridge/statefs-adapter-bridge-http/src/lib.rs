//! # StateFS HTTP & Axum Bridge
//!
//! Provides a live HTTP router and REST API for inspecting, querying, and updating
//! StateFS hierarchical trees in real-time.

use std::sync::Arc;
use tokio::sync::RwLock;

use axum::extract::{Path as AxumPath, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::Value as JsonValue;

use statefs_codec_json::ingest_json_str;
use statefs_core::{MemStore, Path, Store, Value};

/// Shared thread-safe handle to a StateFS memory store.
pub type SharedStore = Arc<RwLock<MemStore>>;

/// Creates a new Axum [`Router`] mounted at the root serving the StateFS Live API.
pub fn statefs_router(store: SharedStore) -> Router {
    Router::new()
        .route("/", get(handle_get_root).post(handle_post_root))
        .route(
            "/{*path}",
            get(handle_get_path)
                .post(handle_post_path)
                .delete(handle_delete_path),
        )
        .with_state(store)
}

/// Recursively converts a StateFS node/subtree into a `serde_json::Value`.
fn node_to_json(store: &MemStore, current_path: &Path) -> JsonValue {
    let children = store.list_children(current_path);
    let self_val = store.get(current_path).map(|n| &n.value);

    if children.is_empty() {
        return match self_val {
            Some(v) => match v {
                Value::Null => JsonValue::Null,
                Value::Bool(b) => JsonValue::Bool(*b),
                Value::Int(i) => JsonValue::from(*i),
                Value::Float(f) => JsonValue::from(*f),
                Value::String(s) => JsonValue::String(s.clone()),
                Value::Bytes(b) => {
                    JsonValue::Array(b.iter().map(|&byte| JsonValue::from(byte)).collect())
                }
                Value::Array(arr) => JsonValue::Array(arr.iter().map(value_to_json).collect()),
                Value::Map(m) => {
                    let mut map = serde_json::Map::new();
                    for (k, val) in m {
                        map.insert(k.clone(), value_to_json(val));
                    }
                    JsonValue::Object(map)
                }
            },
            None => JsonValue::Null,
        };
    }

    let mut map = serde_json::Map::new();
    if let Some(val) = self_val
        && !val.is_null()
    {
        map.insert("_value".to_string(), value_to_json(val));
    }

    for child_path in children {
        if let Some(segment) = child_path.segments().last() {
            let child_json = node_to_json(store, &child_path);
            map.insert(segment.to_string(), child_json);
        }
    }

    JsonValue::Object(map)
}

fn value_to_json(val: &Value) -> JsonValue {
    match val {
        Value::Null => JsonValue::Null,
        Value::Bool(b) => JsonValue::Bool(*b),
        Value::Int(i) => JsonValue::from(*i),
        Value::Float(f) => JsonValue::from(*f),
        Value::String(s) => JsonValue::String(s.clone()),
        Value::Bytes(b) => JsonValue::Array(b.iter().map(|&byte| JsonValue::from(byte)).collect()),
        Value::Array(arr) => JsonValue::Array(arr.iter().map(value_to_json).collect()),
        Value::Map(m) => {
            let mut map = serde_json::Map::new();
            for (k, val) in m {
                map.insert(k.clone(), value_to_json(val));
            }
            JsonValue::Object(map)
        }
    }
}

async fn handle_get_root(State(store): State<SharedStore>) -> Response {
    let store_guard = store.read().await;
    let json_val = node_to_json(&store_guard, &Path::root());

    let mut headers = HeaderMap::new();
    let rev = store_guard.global_revision();
    headers.insert(
        "X-StateFS-Revision",
        HeaderValue::from_str(&rev.to_string()).unwrap_or(HeaderValue::from_static("0")),
    );

    (StatusCode::OK, headers, Json(json_val)).into_response()
}

async fn handle_post_root(
    State(store): State<SharedStore>,
    Json(payload): Json<JsonValue>,
) -> Response {
    let mut store_guard = store.write().await;
    let payload_str = payload.to_string();

    match ingest_json_str(&mut store_guard, "/", &payload_str) {
        Ok(()) => {
            let rev = store_guard.global_revision();
            let mut headers = HeaderMap::new();
            headers.insert(
                "X-StateFS-Revision",
                HeaderValue::from_str(&rev.to_string()).unwrap_or(HeaderValue::from_static("0")),
            );
            (
                StatusCode::OK,
                headers,
                Json(serde_json::json!({"status": "updated", "revision": rev})),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

async fn handle_get_path(
    AxumPath(path_str): AxumPath<String>,
    State(store): State<SharedStore>,
) -> Response {
    let clean_path_str = if path_str.starts_with('/') {
        path_str
    } else {
        format!("/{path_str}")
    };

    let p = Path::parse(&clean_path_str);
    let store_guard = store.read().await;

    if store_guard.get(&p).is_none() && store_guard.list_children(&p).is_empty() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Path not found"})),
        )
            .into_response();
    }

    let json_val = node_to_json(&store_guard, &p);
    let rev = store_guard
        .subtree_revision(&p)
        .unwrap_or_else(|| store_guard.global_revision());

    let mut headers = HeaderMap::new();
    headers.insert(
        "X-StateFS-Revision",
        HeaderValue::from_str(&rev.to_string()).unwrap_or(HeaderValue::from_static("0")),
    );

    (StatusCode::OK, headers, Json(json_val)).into_response()
}

async fn handle_post_path(
    AxumPath(path_str): AxumPath<String>,
    State(store): State<SharedStore>,
    Json(payload): Json<JsonValue>,
) -> Response {
    let clean_path_str = if path_str.starts_with('/') {
        path_str
    } else {
        format!("/{path_str}")
    };

    let p = Path::parse(&clean_path_str);
    let mut store_guard = store.write().await;
    let payload_str = payload.to_string();

    match ingest_json_str(&mut store_guard, &clean_path_str, &payload_str) {
        Ok(()) => {
            let rev = store_guard
                .subtree_revision(&p)
                .unwrap_or_else(|| store_guard.global_revision());
            let mut headers = HeaderMap::new();
            headers.insert(
                "X-StateFS-Revision",
                HeaderValue::from_str(&rev.to_string()).unwrap_or(HeaderValue::from_static("0")),
            );
            (StatusCode::OK, headers, Json(serde_json::json!({"status": "updated", "path": clean_path_str, "revision": rev}))).into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

async fn handle_delete_path(
    AxumPath(path_str): AxumPath<String>,
    State(store): State<SharedStore>,
) -> Response {
    let clean_path_str = if path_str.starts_with('/') {
        path_str
    } else {
        format!("/{path_str}")
    };

    let p = Path::parse(&clean_path_str);
    let mut store_guard = store.write().await;

    match store_guard.remove(&p) {
        Ok(Some(_)) => {
            let rev = store_guard.global_revision();
            (StatusCode::OK, Json(serde_json::json!({"status": "deleted", "path": clean_path_str, "revision": rev}))).into_response()
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Path not found"})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    #[tokio::test]
    async fn test_http_api_crud_roundtrip() {
        let mut store = MemStore::new();
        store
            .insert(&Path::parse("/server/tickrate"), Value::from(128))
            .unwrap();
        store
            .insert(&Path::parse("/server/motd"), Value::from("Welcome!"))
            .unwrap();

        let shared = Arc::new(RwLock::new(store));
        let router = statefs_router(shared.clone());

        // 1. GET /server/tickrate
        let req = Request::builder()
            .method("GET")
            .uri("/server/tickrate")
            .body(Body::empty())
            .unwrap();

        let res = router.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = res.into_body().collect().await.unwrap().to_bytes();
        let json: JsonValue = serde_json::from_slice(&body).unwrap();
        assert_eq!(json, serde_json::json!(128));

        // 2. POST /server/tickrate update to 144
        let update_req = Request::builder()
            .method("POST")
            .uri("/server/tickrate")
            .header("Content-Type", "application/json")
            .body(Body::from("144"))
            .unwrap();

        let update_res = router.clone().oneshot(update_req).await.unwrap();
        assert_eq!(update_res.status(), StatusCode::OK);

        // Verify update reflected in memory store
        {
            let guard = shared.read().await;
            assert_eq!(
                guard
                    .get_str("/server/tickrate")
                    .and_then(|n| n.value.as_int()),
                Some(144)
            );
        }

        // 3. DELETE /server/motd
        let delete_req = Request::builder()
            .method("DELETE")
            .uri("/server/motd")
            .body(Body::empty())
            .unwrap();

        let delete_res = router.clone().oneshot(delete_req).await.unwrap();
        assert_eq!(delete_res.status(), StatusCode::OK);

        {
            let guard = shared.read().await;
            assert!(guard.get_str("/server/motd").is_none());
        }
    }
}
