use std::env;

use capture_core::{CaptureError, ScreenCaptureBackend};
use tracing::debug;

use crate::{portal::PortalBackend, x11::X11Backend};

/// Controls how the Linux backend is selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BackendSelection {
    /// Pick a backend based on the current Linux session environment.
    #[default]
    Auto,
    /// Force the X11 backend.
    X11,
    /// Force the portal backend.
    Portal,
}

/// Detects the best available Linux capture backend for the current session.
pub fn detect_backend(selection: BackendSelection) -> Result<Box<dyn ScreenCaptureBackend>, CaptureError> {
    match selection {
        BackendSelection::Auto => detect_backend_for_current_session(),
        BackendSelection::X11 => {
            debug!("forcing X11 backend");
            Ok(Box::new(X11Backend::new().map_err(|error| match error {
                CaptureError::Backend { message } => CaptureError::BackendUnavailable { message },
                other => other,
            })?))
        }
        BackendSelection::Portal => {
            debug!("forcing portal backend");
            Ok(Box::new(PortalBackend::new()))
        }
    }
}

fn detect_backend_for_current_session() -> Result<Box<dyn ScreenCaptureBackend>, CaptureError> {
    let session_type = detect_session_type();

    match session_type.as_deref() {
        Some("x11") => {
            debug!("detected X11 session");
            Ok(Box::new(X11Backend::new()?))
        }
        Some("wayland") => {
            debug!("detected Wayland session");
            Ok(Box::new(PortalBackend::new()))
        }
        other => Err(CaptureError::Unsupported {
            message: format!(
                "unsupported Linux session type {other:?}; expected X11 or Wayland"
            ),
        }),
    }
}

/// Detects the active Linux session type from environment variables.
pub fn detect_session_type() -> Option<String> {
    env::var("XDG_SESSION_TYPE").ok().or_else(|| {
        if env::var_os("WAYLAND_DISPLAY").is_some() {
            Some("wayland".to_string())
        } else if env::var_os("DISPLAY").is_some() {
            Some("x11".to_string())
        } else {
            None
        }
    })
}
