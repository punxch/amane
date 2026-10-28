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
    Invalid { code: &'static str, message: String },
    Conflict { message: String },
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
            Self::Invalid { code, message } => (StatusCode::BAD_REQUEST, code, message.clone()),
            Self::Conflict { message } => (StatusCode::CONFLICT, "conflict", message.clone()),
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

impl From<amane_core::TaskRepositoryError> for ApiError {
    fn from(error: amane_core::TaskRepositoryError) -> Self {
        match error {
            amane_core::TaskRepositoryError::NotFound(id) => Self::NotFound {
                what: format!("任务 {id} 不存在"),
            },
            amane_core::TaskRepositoryError::NotRetryable(id) => Self::Conflict {
                message: format!("任务 {id} 尚未结束, 不能重试"),
            },
            other => {
                tracing::error!(error = %other, "任务仓储操作失败");
                Self::Internal("任务仓储操作失败".to_owned())
            }
        }
    }
}

impl From<amane_core::ResourceError> for ApiError {
    fn from(error: amane_core::ResourceError) -> Self {
        tracing::error!(error = %error, "资源仓储操作失败");
        Self::Internal("资源仓储操作失败".to_owned())
    }
}

impl From<amane_core::ScheduleError> for ApiError {
    fn from(error: amane_core::ScheduleError) -> Self {
        match error {
            amane_core::ScheduleError::NotFound(id) => Self::NotFound {
                what: format!("定时任务 {id} 不存在"),
            },
            amane_core::ScheduleError::InvalidCron(message) => Self::Invalid {
                code: "invalid_cron",
                message,
            },
            amane_core::ScheduleError::InvalidTimezone(message) => Self::Invalid {
                code: "invalid_timezone",
                message,
            },
            amane_core::ScheduleError::InvalidPayload(message) => Self::Invalid {
                code: "invalid_input",
                message,
            },
            other => {
                tracing::error!(%other, "定时任务操作失败");
                Self::Internal("定时任务操作失败".to_owned())
            }
        }
    }
}

impl From<amane_core::OrganizeError> for ApiError {
    fn from(error: amane_core::OrganizeError) -> Self {
        match error {
            amane_core::OrganizeError::PathTaken(path) => Self::Conflict {
                message: format!("路径 {path} 已被占用"),
            },
            other => {
                tracing::error!(%other, "整理登记操作失败");
                Self::Internal("整理登记操作失败".to_owned())
            }
        }
    }
}

impl From<NumberError> for ApiError {
    fn from(error: NumberError) -> Self {
        Self::InvalidNumber(error)
    }
}

impl From<RepositoryError> for ApiError {
    fn from(error: RepositoryError) -> Self {
        match error {
            RepositoryError::NotFound => Self::NotFound {
                what: "番号条目不存在".to_owned(),
            },
            RepositoryError::InvalidInput(message) => Self::Invalid {
                code: "invalid_input",
                message,
            },
            RepositoryError::Database(error) => {
                tracing::error!(%error, "仓储操作失败");
                Self::Internal("数据库操作失败".to_owned())
            }
        }
    }
}

impl From<amane_core::LibraryError> for ApiError {
    fn from(error: amane_core::LibraryError) -> Self {
        match error {
            amane_core::LibraryError::NotFound(id) => Self::NotFound {
                what: format!("媒体库 {id} 不存在"),
            },
            amane_core::LibraryError::AlreadyRegistered(path) => Self::Conflict {
                message: format!("路径 {path} 已登记"),
            },
            other => {
                tracing::error!(%other, "媒体库操作失败");
                Self::Internal("媒体库操作失败".to_owned())
            }
        }
    }
}
