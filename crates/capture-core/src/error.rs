use std::path::PathBuf;

use thiserror::Error;

/// Standard result type used across the screen capture workspace.
pub type CaptureResult<T> = Result<T, CaptureError>;

/// Represents all recoverable errors exposed by the public API.
#[derive(Debug, Error)]
pub enum CaptureError {
    /// The current platform or session type is not supported by the active backend.
    #[error("unsupported operation: {message}")]
    Unsupported { message: String },
    /// No compatible backend could be initialized for the requested operation.
    #[error("backend unavailable: {message}")]
    BackendUnavailable { message: String },
    /// The requested capture parameters are invalid.
    #[error("invalid capture request: {message}")]
    InvalidRequest { message: String },
    /// The user cancelled the in-progress capture interaction.
    #[error("capture cancelled: {message}")]
    Cancelled { message: String },
    /// A requested output path is invalid.
    #[error("invalid output path: {path:?}")]
    InvalidOutputPath { path: PathBuf },
    /// The captured image is empty and cannot be encoded or saved.
    #[error("captured image is empty")]
    EmptyImage,
    /// The captured pixel buffer does not match the declared metadata.
    #[error("invalid image buffer: expected {expected} bytes, got {actual}")]
    InvalidImageBuffer { expected: usize, actual: usize },
    /// An error occurred while talking to a platform backend.
    #[error("backend error: {message}")]
    Backend { message: String },
    /// An error occurred while encoding an image.
    #[error("encoding error: {message}")]
    Encoding { message: String },
    /// An error occurred while writing output bytes.
    #[error("output error: {message}")]
    Output { message: String },
}
