use std::{
    io::Write as _,
    thread,
    process::{Command, Stdio},
};

use capture_core::CaptureError;
use tracing::warn;

use crate::detect::{BackendSelection, detect_session_type};

/// Represents the clipboard mechanism available on Linux.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardBackend {
    /// Use `wl-copy` for Wayland clipboard integration.
    WlCopy,
    /// Use `xclip` for X11 clipboard integration.
    Xclip,
}

impl ClipboardBackend {
    /// Returns a stable backend identifier for diagnostics.
    pub fn name(self) -> &'static str {
        match self {
            Self::WlCopy => "wl-copy",
            Self::Xclip => "xclip",
        }
    }
}

/// Detects the best clipboard backend for the current Linux environment.
pub fn detect_clipboard_backend(selection: BackendSelection) -> Result<ClipboardBackend, CaptureError> {
    match selection {
        BackendSelection::Portal => detect_wayland_clipboard(),
        BackendSelection::X11 => detect_x11_clipboard(),
        BackendSelection::Auto => match detect_session_type().as_deref() {
            Some("wayland") => detect_wayland_clipboard().or_else(|_| detect_x11_clipboard()),
            Some("x11") => detect_x11_clipboard().or_else(|_| detect_wayland_clipboard()),
            _ => detect_wayland_clipboard().or_else(|_| detect_x11_clipboard()),
        },
    }
}

/// Copies PNG bytes to the Linux desktop clipboard.
pub fn copy_png(bytes: &[u8], selection: BackendSelection) -> Result<ClipboardBackend, CaptureError> {
    let backend = detect_clipboard_backend(selection)?;
    run_clipboard_command(backend, bytes)?;
    Ok(backend)
}

fn detect_wayland_clipboard() -> Result<ClipboardBackend, CaptureError> {
    if command_exists("wl-copy") {
        Ok(ClipboardBackend::WlCopy)
    } else {
        Err(CaptureError::BackendUnavailable {
            message: "wl-copy is not installed or not available in PATH".to_string(),
        })
    }
}

fn detect_x11_clipboard() -> Result<ClipboardBackend, CaptureError> {
    if command_exists("xclip") {
        Ok(ClipboardBackend::Xclip)
    } else {
        Err(CaptureError::BackendUnavailable {
            message: "xclip is not installed or not available in PATH".to_string(),
        })
    }
}

fn command_exists(command: &str) -> bool {
    Command::new("sh")
        .arg("-lc")
        .arg(format!("command -v {command} >/dev/null 2>&1"))
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn run_clipboard_command(backend: ClipboardBackend, bytes: &[u8]) -> Result<(), CaptureError> {
    let mut command = match backend {
        ClipboardBackend::WlCopy => {
            let mut command = Command::new("wl-copy");
            command.arg("--type").arg("image/png");
            command
        }
        ClipboardBackend::Xclip => {
            let mut command = Command::new("xclip");
            command
                .arg("-selection")
                .arg("clipboard")
                .arg("-t")
                .arg("image/png")
                .arg("-i");
            command
        }
    };

    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| CaptureError::Backend {
            message: format!("failed to launch clipboard command {}: {error}", backend.name()),
        })?;

    if let Some(stdin) = child.stdin.as_mut() {
        stdin.write_all(bytes).map_err(|error| CaptureError::Backend {
            message: format!("failed to write PNG bytes to {}: {error}", backend.name()),
        })?;
    }

    // Close stdin so the clipboard tool can finish ingesting the PNG bytes.
    let _ = child.stdin.take();

    // Do not block the caller waiting for clipboard ownership processes like `xclip` / `wl-copy`
    // to exit. They may intentionally stay alive while owning the clipboard selection.
    thread::spawn(move || match child.wait_with_output() {
        Ok(output) if !output.status.success() => {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            warn!(
                backend = backend.name(),
                error = %stderr,
                "clipboard command exited with a failure status"
            );
        }
        Ok(_) => {}
        Err(error) => {
            warn!(
                backend = backend.name(),
                error = %error,
                "failed waiting for clipboard command to exit"
            );
        }
    });

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::ClipboardBackend;

    #[test]
    fn clipboard_backend_names_are_stable() {
        assert_eq!(ClipboardBackend::WlCopy.name(), "wl-copy");
        assert_eq!(ClipboardBackend::Xclip.name(), "xclip");
    }
}
