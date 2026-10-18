//! API errors: one enum, turned into an HTTP status and a JSON body by `IntoResponse`.
//!
//! Handlers return `Result<T, ApiError>`, and `?` converts other errors into `ApiError::Internal`
//! (through the `From<anyhow::Error>` below). In ASP.NET terms this is an exception filter that
//! maps exceptions to `ProblemDetails`, except that the mapping is an ordinary function and every
//! failure is visible in the handler's signature.

use axum::extract::rejection::JsonRejection;
use axum::extract::{FromRequest, Request};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use cfd_core::validate::{Issue, Severity};
use serde::Serialize;
use serde_json::json;

/// A validation issue as the API reports it: a stable code, the English message, the values in
/// it (`params`, for the UI's own wording) and the cell it is about, if any.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct IssueDto {
    pub code: &'static str,
    pub severity: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "serde_json::Map::is_empty")]
    pub params: serde_json::Map<String, serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<[u32; 2]>,
}

impl From<&Issue> for IssueDto {
    fn from(i: &Issue) -> Self {
        Self {
            code: i.code,
            severity: match i.severity {
                Severity::Error => "error",
                Severity::Warning => "warning",
            },
            message: i.message.clone(),
            params: i.params.clone(),
            at: i.at,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("{message}")]
    BadRequest { code: &'static str, message: String },
    #[error("{0}")]
    NotFound(String),
    /// The request does not fit the job's current state (e.g. pausing a finished job).
    #[error("{0}")]
    Conflict(String),
    /// The scene (or its options) cannot run; `issues` says why.
    #[error("{message}")]
    Unprocessable {
        code: &'static str,
        message: String,
        issues: Vec<IssueDto>,
    },
    #[error("{0}")]
    PayloadTooLarge(String),
    #[error("{0}")]
    TooManyRequests(String),
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl ApiError {
    fn status_and_code(&self) -> (StatusCode, &'static str) {
        match self {
            ApiError::BadRequest { code, .. } => (StatusCode::BAD_REQUEST, code),
            ApiError::NotFound(_) => (StatusCode::NOT_FOUND, "notFound"),
            ApiError::Conflict(_) => (StatusCode::CONFLICT, "conflict"),
            ApiError::Unprocessable { code, .. } => (StatusCode::UNPROCESSABLE_ENTITY, code),
            ApiError::PayloadTooLarge(_) => (StatusCode::PAYLOAD_TOO_LARGE, "request.tooLarge"),
            ApiError::TooManyRequests(_) => (StatusCode::TOO_MANY_REQUESTS, "queue.full"),
            ApiError::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "internal"),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code) = self.status_and_code();
        if let ApiError::Internal(e) = &self {
            tracing::error!("internal error: {e:#}");
        }
        let mut body = json!({ "code": code, "message": format!("{self:#}") });
        if let ApiError::Unprocessable { issues, .. } = &self {
            body["issues"] = json!(issues);
        }
        (status, Json(json!({ "error": body }))).into_response()
    }
}

/// `Json<T>` with the API's error format: a body that is not valid JSON is a `400` with the
/// parser's message, and one over the size limit a `413` that names the remedy.
///
/// An extractor is a type that builds itself from the request (`FromRequest`), like a model
/// binder in ASP.NET; this one wraps axum's `Json` and only changes its errors.
pub struct ApiJson<T>(pub T);

impl<S, T> FromRequest<S> for ApiJson<T>
where
    Json<T>: FromRequest<S, Rejection = JsonRejection>,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        match Json::<T>::from_request(req, state).await {
            Ok(Json(v)) => Ok(ApiJson(v)),
            Err(rejection) if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE => {
                Err(ApiError::PayloadTooLarge(
                    "the request body is larger than the server's limit; send the scene's layers \
                     with the \"zstd+base64\" encoding (usually more than 50× smaller than \
                     \"raw+base64\")"
                        .into(),
                ))
            }
            Err(rejection) => Err(ApiError::BadRequest {
                code: "request.json",
                message: rejection.body_text(),
            }),
        }
    }
}
