//! Linux platform backends for screen capture.

pub mod clipboard;
pub mod detect;
pub mod diagnostics;
pub mod portal;
pub mod x11;

pub use clipboard::{ClipboardBackend, copy_png, detect_clipboard_backend};
pub use detect::{BackendSelection, detect_backend, detect_session_type};
pub use diagnostics::{DiagnosticItem, DiagnosticStatus, LinuxDiagnostics, collect_diagnostics};
pub use x11::window_region_under_pointer;
