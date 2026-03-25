use std::env;

use capture_core::CaptureError;
use zbus::blocking::{Connection, fdo::DBusProxy};

use crate::{
    clipboard::detect_clipboard_backend,
    detect::{BackendSelection, detect_backend},
};

const PORTAL_BUS_NAME: &str = "org.freedesktop.portal.Desktop";

/// Represents a diagnostic check result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiagnosticStatus {
    /// The capability is available.
    Ok,
    /// The capability is partially available or has known limitations.
    Warn,
    /// The capability is unavailable.
    Error,
    /// The capability is not implemented yet.
    Unimplemented,
}

/// Represents one doctor check row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticItem {
    /// Human-readable check name.
    pub label: &'static str,
    /// Check status.
    pub status: DiagnosticStatus,
    /// Human-readable detail.
    pub detail: String,
}

/// Summarizes Linux desktop diagnostic signals for the current environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinuxDiagnostics {
    /// Detected session type such as `x11` or `wayland`.
    pub session_type: DiagnosticItem,
    /// Backend that `auto` selection currently resolves to.
    pub detected_backend: DiagnosticItem,
    /// Whether xdg-desktop-portal appears reachable on D-Bus.
    pub portal_availability: DiagnosticItem,
    /// Whether clipboard integration exists in this MVP.
    pub clipboard_support: DiagnosticItem,
    /// Placeholder for future multi-display diagnostics.
    pub multi_display: DiagnosticItem,
}

/// Collects Linux-specific doctor diagnostics.
pub fn collect_diagnostics() -> LinuxDiagnostics {
    LinuxDiagnostics {
        session_type: detect_session_type_item(),
        detected_backend: detect_backend_item(),
        portal_availability: detect_portal_item(),
        clipboard_support: detect_clipboard_item(),
        multi_display: DiagnosticItem {
            label: "Multi-display info",
            status: DiagnosticStatus::Unimplemented,
            detail: "planned for a later iteration".to_string(),
        },
    }
}

fn detect_session_type_item() -> DiagnosticItem {
    match detect_session_type() {
        Some(session_type) => DiagnosticItem {
            label: "Session type",
            status: DiagnosticStatus::Ok,
            detail: session_type,
        },
        None => DiagnosticItem {
            label: "Session type",
            status: DiagnosticStatus::Error,
            detail: "could not infer X11 or Wayland from the current environment".to_string(),
        },
    }
}

fn detect_backend_item() -> DiagnosticItem {
    match detect_backend(BackendSelection::Auto) {
        Ok(backend) => DiagnosticItem {
            label: "Detected backend",
            status: DiagnosticStatus::Ok,
            detail: backend.name().to_string(),
        },
        Err(error) => DiagnosticItem {
            label: "Detected backend",
            status: DiagnosticStatus::Error,
            detail: render_error(error),
        },
    }
}

fn detect_portal_item() -> DiagnosticItem {
    match Connection::session() {
        Ok(connection) => match DBusProxy::new(&connection) {
            Ok(proxy) => match proxy.name_has_owner(PORTAL_BUS_NAME.try_into().expect("static bus name")) {
                Ok(true) => DiagnosticItem {
                    label: "Portal availability",
                    status: DiagnosticStatus::Ok,
                    detail: format!("{PORTAL_BUS_NAME} is reachable on the session bus"),
                },
                Ok(false) => DiagnosticItem {
                    label: "Portal availability",
                    status: DiagnosticStatus::Warn,
                    detail: format!("{PORTAL_BUS_NAME} is not registered on the session bus"),
                },
                Err(error) => DiagnosticItem {
                    label: "Portal availability",
                    status: DiagnosticStatus::Warn,
                    detail: format!("failed to query portal service ownership: {error}"),
                },
            },
            Err(error) => DiagnosticItem {
                label: "Portal availability",
                status: DiagnosticStatus::Warn,
                detail: format!("failed to create D-Bus proxy: {error}"),
            },
        },
        Err(error) => DiagnosticItem {
            label: "Portal availability",
            status: DiagnosticStatus::Warn,
            detail: format!("failed to connect to session D-Bus: {error}"),
        },
    }
}

fn detect_clipboard_item() -> DiagnosticItem {
    match detect_clipboard_backend(BackendSelection::Auto) {
        Ok(backend) => DiagnosticItem {
            label: "Clipboard support",
            status: DiagnosticStatus::Ok,
            detail: format!("available via {}", backend.name()),
        },
        Err(error) => DiagnosticItem {
            label: "Clipboard support",
            status: DiagnosticStatus::Warn,
            detail: render_error(error),
        },
    }
}

fn detect_session_type() -> Option<String> {
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

fn render_error(error: CaptureError) -> String {
    match error {
        CaptureError::Unsupported { message }
        | CaptureError::BackendUnavailable { message }
        | CaptureError::Backend { message } => message,
        other => other.to_string(),
    }
}
