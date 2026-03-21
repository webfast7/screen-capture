//! Core abstractions and shared models for screen capture.

pub mod backend;
pub mod error;
pub mod model;
pub mod output;

pub use backend::{ImageEncoder, ScreenCaptureBackend};
pub use error::{CaptureError, CaptureResult};
pub use model::{
    CaptureArea, CaptureRegion, CaptureRequest, CapturedImage, ImageFormat, PixelFormat, Resolution,
    SaveConflictStrategy, SaveOptions,
};
pub use output::{
    FileOutputTarget, OutputArtifact, OutputTarget, PngEncoder, capture_and_encode,
    capture_and_save,
};
