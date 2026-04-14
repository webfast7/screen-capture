use std::{fs, path::{Path, PathBuf}, thread, time::{Duration, SystemTime, UNIX_EPOCH}};

use capture_core::{
    CaptureError, CaptureRegion, CaptureRequest, FileOutputTarget, ImageEncoder, PngEncoder,
    OutputTarget, SaveConflictStrategy, SaveOptions, capture_and_encode,
};
use capture_platform_linux::{
    BackendSelection, DiagnosticItem, DiagnosticStatus, collect_diagnostics, copy_png,
    detect_backend,
};
use capture_utils::{build_output_path, default_filename, default_output_dir, init_logging};
use clap::{Parser, Subcommand, ValueEnum};
use tracing::info;

/// Linux screen capture CLI.
#[derive(Debug, Parser)]
#[command(name = "capture-cli", version, about = "Minimal Linux screenshot tool")]
struct Cli {
    /// The command to execute.
    #[command(subcommand)]
    command: Commands,
}

/// Supported CLI commands.
#[derive(Debug, Subcommand)]
enum Commands {
    /// Capture the full screen and write a PNG file.
    Capture {
        /// Explicitly request a full-screen capture. The current MVP defaults to full screen.
        #[arg(long, default_value_t = false)]
        fullscreen: bool,
        /// Capture a fixed region in the form x,y,width,height.
        #[arg(long, value_parser = parse_region)]
        region: Option<CaptureRegion>,
        /// Ask the platform to let the user select a region interactively.
        #[arg(long, default_value_t = false)]
        select: bool,
        /// Optional output path. Defaults to a timestamped PNG in the preferred screenshot directory.
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Save the screenshot to a file.
        #[arg(long, default_value_t = false)]
        save: bool,
        /// Copy the screenshot to the system clipboard.
        #[arg(long, default_value_t = false)]
        copy: bool,
        /// How to handle an existing output file.
        #[arg(long, value_enum, default_value_t = CliConflictStrategy::Rename)]
        on_conflict: CliConflictStrategy,
        /// Select the Linux capture backend.
        #[arg(long, value_enum, default_value_t = CliBackend::Auto)]
        backend: CliBackend,
        /// Delay the capture by N seconds.
        #[arg(long, default_value_t = 0)]
        delay: u64,
    },
    /// Diagnose the current Linux screenshot environment.
    Doctor,
}

/// User-facing backend selector for the CLI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum CliBackend {
    /// Pick a backend from the current environment.
    Auto,
    /// Force the X11 backend.
    X11,
    /// Force the xdg-desktop-portal backend.
    Portal,
}

/// User-facing file conflict strategy for the CLI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum CliConflictStrategy {
    /// Pick a new filename if the target path already exists.
    Rename,
    /// Replace the existing file.
    Overwrite,
    /// Fail if the target path already exists.
    Error,
}

impl From<CliBackend> for BackendSelection {
    fn from(value: CliBackend) -> Self {
        match value {
            CliBackend::Auto => Self::Auto,
            CliBackend::X11 => Self::X11,
            CliBackend::Portal => Self::Portal,
        }
    }
}

impl From<CliConflictStrategy> for SaveConflictStrategy {
    fn from(value: CliConflictStrategy) -> Self {
        match value {
            CliConflictStrategy::Rename => Self::Rename,
            CliConflictStrategy::Overwrite => Self::Overwrite,
            CliConflictStrategy::Error => Self::Error,
        }
    }
}

fn main() {
    init_logging();

    if let Err(error) = run() {
        eprintln!("error: {}", friendly_error_message(&error));
        std::process::exit(1);
    }
}

fn run() -> Result<(), capture_core::CaptureError> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Capture {
            fullscreen,
            region,
            select,
            output,
            save,
            copy,
            on_conflict,
            backend,
            delay,
        } => capture_command(
            fullscreen,
            region,
            select,
            output,
            save,
            copy,
            on_conflict,
            backend,
            delay,
        ),
        Commands::Doctor => doctor_command(),
    }
}

fn capture_command(
    fullscreen: bool,
    region: Option<CaptureRegion>,
    select: bool,
    output: Option<PathBuf>,
    save: bool,
    copy: bool,
    on_conflict: CliConflictStrategy,
    backend: CliBackend,
    delay: u64,
) -> Result<(), capture_core::CaptureError> {
    if delay > 0 {
        info!(delay_seconds = delay, "delaying screen capture");
        thread::sleep(Duration::from_secs(delay));
    }

    let backend_selection: BackendSelection = backend.into();
    let backend = detect_backend(backend_selection)?;
    let encoder = PngEncoder;
    let actions = build_actions(output.is_some(), save, copy);
    let output_path = build_output_path(output, default_filename(encoder.format()));
    let request = build_request(fullscreen, region, select)?;
    let save_options =
        SaveOptions::with_conflict_strategy(encoder.format(), output_path, on_conflict.into());
    let target = FileOutputTarget;

    info!(
        backend = backend.name(),
        path = ?save_options.output_path,
        output_dir = ?default_output_dir(),
        request = ?request,
        save = actions.save,
        copy = actions.copy,
        "starting screen capture"
    );
    let encoded = capture_and_encode(backend.as_ref(), &request, &encoder)?;

    if actions.save {
        let artifact = target.write(&encoded, &save_options)?;
        info!(path = ?artifact.path, bytes = artifact.bytes_written, "capture saved");
        println!("{}", artifact.path.display());
    }

    if actions.copy {
        let clipboard_backend = copy_png(&encoded, backend_selection)?;
        info!(clipboard = clipboard_backend.name(), bytes = encoded.len(), "capture copied to clipboard");
        if !actions.save {
            println!("copied to clipboard via {}", clipboard_backend.name());
        }
    }

    Ok(())
}

fn friendly_error_message(error: &CaptureError) -> String {
    match error {
        CaptureError::InvalidRequest { message } => format!("invalid capture request: {message}"),
        CaptureError::Cancelled { message } => format!("capture cancelled: {message}"),
        CaptureError::Unsupported { message } => {
            format!("current session does not support this operation: {message}")
        }
        CaptureError::BackendUnavailable { message } => {
            format!("requested backend is unavailable: {message}")
        }
        CaptureError::InvalidOutputPath { path } => {
            format!("invalid output path: {}", path.display())
        }
        CaptureError::EmptyImage => "capture backend returned empty image data".to_string(),
        CaptureError::InvalidImageBuffer { .. } => {
            "capture backend returned malformed image data".to_string()
        }
        CaptureError::Backend { message } => format!("capture backend failed: {message}"),
        CaptureError::Encoding { message } => format!("failed to encode PNG image: {message}"),
        CaptureError::Output { message } => format!("failed to save screenshot: {message}"),
    }
}

fn build_request(
    fullscreen: bool,
    region: Option<CaptureRegion>,
    select: bool,
) -> Result<CaptureRequest, CaptureError> {
    let explicit_modes = usize::from(fullscreen) + usize::from(region.is_some()) + usize::from(select);
    if explicit_modes > 1 {
        return Err(CaptureError::InvalidRequest {
            message: "use only one of --fullscreen, --region, or --select".to_string(),
        });
    }

    if let Some(region) = region {
        return Ok(CaptureRequest::region(region));
    }

    if select {
        return Ok(CaptureRequest::interactive_region());
    }

    Ok(CaptureRequest::fullscreen())
}

fn parse_region(value: &str) -> Result<CaptureRegion, String> {
    let parts: Vec<&str> = value.split(',').collect();
    if parts.len() != 4 {
        return Err("expected region format x,y,width,height".to_string());
    }

    let parse_part = |index: usize, label: &str| -> Result<u32, String> {
        parts[index]
            .trim()
            .parse::<u32>()
            .map_err(|_| format!("invalid {label} value in region {value:?}"))
    };

    let x = parse_part(0, "x")?;
    let y = parse_part(1, "y")?;
    let width = parse_part(2, "width")?;
    let height = parse_part(3, "height")?;

    CaptureRegion::new(x, y, width, height).map_err(|error| error.to_string())
}

fn build_actions(has_output: bool, save: bool, copy: bool) -> CaptureActions {
    if copy && !save && !has_output {
        return CaptureActions {
            save: false,
            copy: true,
        };
    }

    CaptureActions {
        save: save || has_output || !copy,
        copy,
    }
}

fn doctor_command() -> Result<(), CaptureError> {
    let diagnostics = collect_diagnostics();
    let output_dir = default_output_dir();
    let output_dir_item = check_output_dir_writable(&output_dir);

    print_diagnostic(&diagnostics.session_type);
    print_diagnostic(&diagnostics.detected_backend);
    print_diagnostic(&diagnostics.portal_availability);
    print_diagnostic(&output_dir_item);
    print_diagnostic(&diagnostics.clipboard_support);
    print_diagnostic(&diagnostics.multi_display);

    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CaptureActions {
    save: bool,
    copy: bool,
}

#[cfg(test)]
mod tests {
    use super::{CaptureActions, build_actions};

    #[test]
    fn defaults_to_save_when_no_explicit_action_is_requested() {
        assert_eq!(
            build_actions(false, false, false),
            CaptureActions {
                save: true,
                copy: false,
            }
        );
    }

    #[test]
    fn copy_only_disables_save_when_no_output_path_is_present() {
        assert_eq!(
            build_actions(false, false, true),
            CaptureActions {
                save: false,
                copy: true,
            }
        );
    }

    #[test]
    fn output_path_implies_save_even_when_copy_is_requested() {
        assert_eq!(
            build_actions(true, false, true),
            CaptureActions {
                save: true,
                copy: true,
            }
        );
    }
}

fn print_diagnostic(item: &DiagnosticItem) {
    println!(
        "{:<20} {:<13} {}",
        item.label,
        format!("[{}]", status_label(&item.status)),
        item.detail
    );
}

fn status_label(status: &DiagnosticStatus) -> &'static str {
    match status {
        DiagnosticStatus::Ok => "ok",
        DiagnosticStatus::Warn => "warn",
        DiagnosticStatus::Error => "error",
        DiagnosticStatus::Unimplemented => "todo",
    }
}

fn check_output_dir_writable(path: &Path) -> DiagnosticItem {
    if let Err(error) = fs::create_dir_all(path) {
        return DiagnosticItem {
            label: "Output directory",
            status: DiagnosticStatus::Error,
            detail: format!("{} is not usable: {error}", path.display()),
        };
    }

    let probe = path.join(format!(
        ".screen-capture-doctor-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));

    match fs::write(&probe, b"doctor") {
        Ok(()) => {
            let _ = fs::remove_file(&probe);
            DiagnosticItem {
                label: "Output directory",
                status: DiagnosticStatus::Ok,
                detail: format!("{} is writable", path.display()),
            }
        }
        Err(error) => DiagnosticItem {
            label: "Output directory",
            status: DiagnosticStatus::Error,
            detail: format!("{} is not writable: {error}", path.display()),
        },
    }
}
