use axum::{
    extract::Json,
    http::{header, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::process::Command;
use tower_http::{services::ServeDir, set_header::SetResponseHeaderLayer};

#[derive(Deserialize)]
struct SendCommand {
    command: String,
    session: String,
}

#[derive(Serialize)]
struct ApiResponse {
    success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Serialize)]
struct TmuxWindow {
    target: String,
    name: String,
}

#[derive(Deserialize)]
struct CaptureRequest {
    target: String,
}

#[derive(Serialize)]
struct CaptureResponse {
    content: String,
}

async fn capture_pane(Json(payload): Json<CaptureRequest>) -> impl IntoResponse {
    let target = if payload.target.is_empty() {
        "0".to_string()
    } else {
        payload.target
    };

    // Capture the pane content with history
    let result = Command::new("tmux")
        .args(["capture-pane", "-p", "-t", &target, "-S", "-100"])
        .output();

    match result {
        Ok(output) => {
            if output.status.success() {
                let content = String::from_utf8_lossy(&output.stdout).to_string();
                (StatusCode::OK, Json(CaptureResponse { content }))
            } else {
                (StatusCode::OK, Json(CaptureResponse { content: String::new() }))
            }
        }
        Err(_) => (StatusCode::OK, Json(CaptureResponse { content: String::new() })),
    }
}

async fn list_windows() -> impl IntoResponse {
    let result = Command::new("tmux")
        .args(["list-windows", "-a", "-F", "#{session_name}:#{window_index}\t#{window_name}"])
        .output();

    match result {
        Ok(output) => {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let windows: Vec<TmuxWindow> = stdout
                    .lines()
                    .filter_map(|line| {
                        let parts: Vec<&str> = line.split('\t').collect();
                        if parts.len() == 2 {
                            Some(TmuxWindow {
                                target: parts[0].to_string(),
                                name: parts[1].to_string(),
                            })
                        } else {
                            None
                        }
                    })
                    .collect();
                (StatusCode::OK, Json(windows))
            } else {
                (StatusCode::OK, Json(vec![]))
            }
        }
        Err(_) => (StatusCode::OK, Json(vec![])),
    }
}

async fn send_to_tmux(Json(payload): Json<SendCommand>) -> impl IntoResponse {
    let session = if payload.session.is_empty() {
        "0".to_string()
    } else {
        payload.session
    };

    let command = payload.command;

    // Send the command text literally
    let _ = Command::new("tmux")
        .args(["send-keys", "-t", &session, "-l", &command])
        .output();

    // Send Enter key
    let result = Command::new("tmux")
        .args(["send-keys", "-t", &session, "Enter"])
        .output();

    match result {
        Ok(output) => {
            if output.status.success() {
                (
                    StatusCode::OK,
                    Json(ApiResponse {
                        success: true,
                        error: None,
                    }),
                )
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                (
                    StatusCode::BAD_REQUEST,
                    Json(ApiResponse {
                        success: false,
                        error: Some(stderr.to_string()),
                    }),
                )
            }
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                success: false,
                error: Some(format!("Failed to execute tmux: {}", e)),
            }),
        ),
    }
}

async fn health() -> &'static str {
    "OK"
}

#[tokio::main]
async fn main() {
    let port = std::env::var("PORT").unwrap_or_else(|_| "5533".to_string());
    let addr = format!("0.0.0.0:{}", port);

    // Serve static files from the static directory with no-cache headers
    let static_service = ServeDir::new("static").append_index_html_on_directories(true);

    let app = Router::new()
        .route("/api/send", post(send_to_tmux))
        .route("/api/windows", get(list_windows))
        .route("/api/capture", post(capture_pane))
        .route("/health", get(health))
        .fallback_service(static_service)
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            header::HeaderValue::from_static("no-cache, no-store, must-revalidate"),
        ));

    println!("TMUX Terminal running on http://{}", addr);
    println!("Make sure tmux is running with an active session!");

    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
