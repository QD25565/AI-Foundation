//! SSE (Server-Sent Events) endpoint for real-time notifications.
//!
//! Uses the teambook `standby` CLI command for event-driven wake —
//! no polling. The standby command blocks until a relevant event occurs
//! (new DM, @mention, task assignment), then we fetch the latest state
//! and push it to the SSE client.

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::sse::{Event, KeepAlive, Sse},
    routing::get,
    Router,
};
use futures::stream;
use std::convert::Infallible;
use std::time::Duration;

use crate::cli_wrapper;
use crate::http_api::ApiState;

pub fn sse_routes() -> Router<ApiState> {
    Router::new().route("/api/events", get(handle_events))
}

async fn handle_events(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<Sse<impl futures::Stream<Item = Result<Event, Infallible>>>, StatusCode> {
    // Extract and validate auth token
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or(StatusCode::UNAUTHORIZED)?;

    let h_id = state
        .pairing
        .resolve_token(token)
        .await
        .ok_or(StatusCode::UNAUTHORIZED)?;

    // Send initial state immediately
    let initial_dms = cli_wrapper::teambook_as(&["read-dms", "10"], &h_id).await;
    let initial_broadcasts = cli_wrapper::teambook_as(&["broadcasts", "10"], &h_id).await;
    let initial_status = cli_wrapper::teambook_as(&["status"], &h_id).await;

    let initial_data = serde_json::json!({
        "type": "connected",
        "dms": initial_dms,
        "broadcasts": initial_broadcasts,
        "status": initial_status,
    });

    // Stream starts with initial state, then waits for events via standby
    let event_stream = stream::unfold(
        (h_id, Some(initial_data)),
        |(h_id, initial)| async move {
            if let Some(data) = initial {
                // First iteration: send initial state
                let event = Event::default().event("init").data(data.to_string());
                return Some((Ok::<_, Infallible>(event), (h_id, None)));
            }

            // Subsequent iterations: wait for events via standby (event-driven, NOT polling)
            let wake = cli_wrapper::teambook_as(&["standby", "30"], &h_id).await;

            // After wake, fetch current state
            let dms = cli_wrapper::teambook_as(&["read-dms", "5"], &h_id).await;
            let broadcasts = cli_wrapper::teambook_as(&["broadcasts", "3"], &h_id).await;

            let data = serde_json::json!({
                "type": "update",
                "wake_reason": wake,
                "dms": dms,
                "broadcasts": broadcasts,
            });

            let event = Event::default().event("message").data(data.to_string());
            Some((Ok(event), (h_id, None)))
        },
    );

    Ok(Sse::new(event_stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("ping"),
    ))
}
