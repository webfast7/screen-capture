use crate::{
    error::CaptureResult,
    model::{CaptureRequest, CapturedImage, ImageFormat},
};

/// Captures pixels from a screen source.
pub trait ScreenCaptureBackend {
    /// Returns a stable backend identifier for logs and diagnostics.
    fn name(&self) -> &'static str;

    /// Captures image data for the provided backend-agnostic request.
    fn capture(&self, request: &CaptureRequest) -> CaptureResult<CapturedImage>;
}

/// Encodes a raw image into a serialized image format.
pub trait ImageEncoder {
    /// Returns the encoded image format produced by this encoder.
    fn format(&self) -> ImageFormat;

    /// Encodes a captured image and returns the encoded bytes.
    fn encode(&self, image: &CapturedImage) -> CaptureResult<Vec<u8>>;
}
