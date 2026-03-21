mod hotkey;
mod launch;

use std::{env, path::PathBuf, process, thread};
use std::time::Duration;

use capture_editor::{run_editor_with_capture, run_editor_with_input};
use capture_platform_linux::detect_session_type;
use clap::Parser;
use ksni::blocking::TrayMethods;
use ksni::menu::{MenuItem, StandardItem};
use tracing::{error, info, warn};

use crate::launch::{EditorLaunch, spawn_editor_process};

#[derive(Debug, Parser)]
#[command(name = "capture-tray", about = "Linux tray companion for screen capture")]
struct Args {
    /// Launch the region capture flow and open the built-in editor.
    #[arg(long)]
    edit_region: bool,

    /// Open the built-in editor for an existing image file.
    #[arg(long)]
    edit_input: Option<PathBuf>,
}

struct CaptureTray;

impl ksni::Tray for CaptureTray {
    fn activate(&mut self, _x: i32, _y: i32) {
        thread::spawn(|| {
            match spawn_editor_process(EditorLaunch::Region) {
                Ok(()) => info!("tray: launched region editor from tray activation"),
                Err(error) => error!(error = %error, "tray: failed to launch region editor"),
            }
        });
    }

    fn icon_name(&self) -> String {
        "camera-photo".to_owned()
    }

    fn id(&self) -> String {
        "screen-capture".to_owned()
    }

    fn title(&self) -> String {
        "Screen Capture".to_owned()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: "Screen Capture".to_owned(),
            description: "Background tray service".to_owned(),
            ..Default::default()
        }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        vec![
            StandardItem {
                label: "Quit".to_owned(),
                activate: Box::new(|_| std::process::exit(0)),
                ..Default::default()
            }
            .into(),
        ]
    }
}

fn main() {
    capture_utils::init_logging();
    let args = Args::parse_from(env::args_os());

    if args.edit_region {
        match run_editor_with_capture(None) {
            Ok(()) => return,
            Err(error) => {
                error!(error = %error, "tray: one-shot editor capture failed");
                process::exit(1);
            }
        }
    }

    if let Some(path) = args.edit_input.as_deref() {
        match run_editor_with_input(path, None) {
            Ok(()) => return,
            Err(error) => {
                error!(error = %error, path = %path.display(), "tray: failed to open editor input");
                process::exit(1);
            }
        }
    }

    let _handle = CaptureTray
        .assume_sni_available(true)
        .spawn()
        .expect("failed to start tray service");

    info!("tray: service started");
    match detect_session_type().as_deref() {
        Some("x11") => hotkey::spawn_hotkey_listener(),
        Some("wayland") => warn!(
            "tray hotkey Ctrl+Alt+A is not enabled on Wayland; use the same binary with --edit-region from GNOME custom shortcuts"
        ),
        other => warn!(session = ?other, "tray hotkey: unsupported session type"),
    }

    loop {
        thread::sleep(Duration::from_secs(60));
    }
}
