use std::{fs, path::PathBuf};

use ashpd::desktop::screenshot::Screenshot;
use capture_core::{
    CaptureArea, CaptureError, CaptureRequest, CaptureResult, CapturedImage, PixelFormat,
    Resolution, ScreenCaptureBackend,
};
use image::GenericImageView;
use tokio::runtime::Builder;
use tracing::debug;

/// Wayland-oriented backend that captures screenshots via `xdg-desktop-portal`.
#[derive(Debug, Default)]
pub struct PortalBackend;

impl PortalBackend {
    /// Creates a portal-backed capture backend.
    pub fn new() -> Self {
        Self
    }

    fn request_screenshot_uri(&self, interactive: bool) -> CaptureResult<String> {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| CaptureError::Backend {
                message: format!("failed to start Tokio runtime for portal capture: {error}"),
            })?;

        runtime
            .block_on(async {
                let response = Screenshot::request()
                    .interactive(interactive)
                    .modal(true)
                    .send()
                    .await
                    .map_err(|error| CaptureError::Backend {
                        message: format!("portal screenshot request failed: {error}"),
                    })?
                    .response()
                    .map_err(|error| CaptureError::Backend {
                        message: format!("portal screenshot was denied or failed: {error}"),
                    })?;

                Ok::<String, CaptureError>(response.uri().to_string())
            })
    }

    fn load_image_from_uri(&self, uri: &str) -> CaptureResult<CapturedImage> {
        let path = file_path_from_uri(uri)?;
        let bytes = fs::read(&path).map_err(|error| CaptureError::Backend {
            message: format!("failed to read portal screenshot at {path:?}: {error}"),
        })?;
        let image = image::load_from_memory(&bytes).map_err(|error| CaptureError::Backend {
            message: format!("failed to decode portal screenshot image: {error}"),
        })?;
        let rgba = image.to_rgba8();
        let (width, height) = image.dimensions();

        let captured = CapturedImage::new(
            Resolution { width, height },
            PixelFormat::Rgba8,
            rgba.into_raw(),
        )?;

        if let Err(error) = fs::remove_file(&path) {
            debug!(path = %path.display(), error = %error, "failed to remove portal screenshot file after import");
        } else {
            debug!(path = %path.display(), "removed portal screenshot file after import");
        }

        Ok(captured)
    }
}

impl ScreenCaptureBackend for PortalBackend {
    fn name(&self) -> &'static str {
        "xdg-desktop-portal"
    }

    fn capture(&self, request: &CaptureRequest) -> CaptureResult<CapturedImage> {
        let interactive = matches!(request.area, CaptureArea::InteractiveRegion);
        let uri = self.request_screenshot_uri(interactive)?;
        debug!(uri, "received screenshot URI from portal");
        let image = self.load_image_from_uri(&uri)?;

        match request.area {
            CaptureArea::Fullscreen | CaptureArea::InteractiveRegion => Ok(image),
            CaptureArea::Region(region) => image.crop(region),
        }
    }
}

fn file_path_from_uri(uri: &str) -> CaptureResult<PathBuf> {
    let file_uri = url::Url::parse(uri).map_err(|error| CaptureError::Backend {
        message: format!("portal returned an invalid screenshot URI {uri:?}: {error}"),
    })?;

    file_uri
        .to_file_path()
        .map_err(|_| CaptureError::Unsupported {
            message: format!(
                "portal returned a non-file screenshot URI {uri:?}; only file-backed screenshots are supported in this MVP"
            ),
        })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::file_path_from_uri;

    #[test]
    fn file_uri_is_converted_to_local_path() {
        let path = file_path_from_uri("file:///tmp/shot.png").expect("file uri");
        assert_eq!(path, PathBuf::from("/tmp/shot.png"));
    }

    #[test]
    fn non_file_uri_is_rejected() {
        let error = file_path_from_uri("https://example.com/shot.png").expect_err("unsupported");
        assert!(error.to_string().contains("non-file"));
    }
}
