//! Shared logging and path helpers for the screen capture workspace.

use std::{env, path::PathBuf};

use capture_core::model::ImageFormat;
use time::{OffsetDateTime, format_description::FormatItem, macros::format_description};
use tracing_subscriber::{EnvFilter, fmt};

const TIMESTAMP_FORMAT: &[FormatItem<'static>] =
    format_description!("[year][month][day]-[hour][minute][second]");

/// Initializes tracing with a sensible default log level for CLI usage.
pub fn init_logging() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = fmt().with_env_filter(filter).without_time().try_init();
}

/// Builds a default PNG output path in the preferred screenshot directory.
pub fn default_output_path() -> PathBuf {
    build_output_path(None, default_filename(ImageFormat::Png))
}

/// Returns the preferred screenshot output directory for the current user.
pub fn default_output_dir() -> PathBuf {
    let home = env::var_os("HOME").map(PathBuf::from);
    if let Some(home) = home {
        let pictures = home.join("Pictures");
        if pictures.is_dir() {
            return pictures;
        }

        return home;
    }

    env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

/// Builds an output path from an optional user-provided path or a generated filename.
pub fn build_output_path(output: Option<PathBuf>, fallback_name: String) -> PathBuf {
    output.unwrap_or_else(|| default_output_dir().join(fallback_name))
}

/// Returns a timestamped default filename for an encoded image format.
pub fn default_filename(format: ImageFormat) -> String {
    let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
    let timestamp = now
        .format(TIMESTAMP_FORMAT)
        .unwrap_or_else(|_| "unknown-time".to_string());

    format!("screencap-{timestamp}.{}", format.file_extension())
}

#[cfg(test)]
mod tests {
    use capture_core::model::ImageFormat;

    use super::{build_output_path, default_filename};

    #[test]
    fn generated_filename_uses_png_extension() {
        let filename = default_filename(ImageFormat::Png);
        assert!(filename.ends_with(".png"));
    }

    #[test]
    fn explicit_output_path_is_preserved() {
        let output = build_output_path(Some("shot.png".into()), "fallback.png".to_string());
        assert_eq!(output.to_string_lossy(), "shot.png");
    }

    #[test]
    fn generated_output_path_keeps_filename() {
        let output = build_output_path(None, "fallback.png".to_string());
        assert!(output.ends_with("fallback.png"));
    }
}
