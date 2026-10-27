use amane_core::NumberError;
use amane_core::RepositoryError;
use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use utoipa::ToSchema;

/// 统一的错误响应体.
#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorBody {
    /// 机器可读的错误码.
    pub code: String,
    /// 面向用户的说明.
    pub message: String,
}

#[derive(Debug)]
pub enum ApiError {
    InvalidNumber(NumberError),
    NotFound { what: String },
    Unauthorized,
    Internal(String),
}

impl ApiError {
    fn parts(&self) -> (StatusCode, &'static str, String) {
        match self {
            Self::InvalidNumber(error) => {
                (StatusCode::BAD_REQUEST, "invalid_number", error.to_string())
            }
            Self::NotFound { what } => (StatusCode::NOT_FOUND, "not_found", what.clone()),
            Self::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "缺少或错误的访问令牌".to_owned(),
            ),
            Self::Internal(message) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                message.clone(),
            ),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code, message) = self.parts();
        let body = ErrorBody {
            code: code.to_owned(),
            message,
        };
        (status, Json(body)).into_response()
    }
}

impl From<NumberError> for ApiError {
    fn from(error: NumberError) -> Self {
        Self::InvalidNumber(error)
    }
}

impl From<RepositoryError> for ApiError {
    fn from(error: RepositoryError) -> Self {
        tracing::error!(error = %error, "仓储操作失败");
        Self::Internal("数据库操作失败".to_owned())
    }
}
