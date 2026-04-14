use std::{env, io, process::Command};

use tracing::info;

#[derive(Debug, Clone)]
pub(crate) enum EditorLaunch {
    Region,
}

pub(crate) fn spawn_editor_process(launch: EditorLaunch) -> io::Result<()> {
    let executable = env::current_exe()?;
    let mut command = Command::new(&executable);
    match launch {
        EditorLaunch::Region => {
            command.arg("--edit-region");
        }
    }

    let child = command.spawn()?;
    info!(
        pid = child.id(),
        executable = %executable.display(),
        "tray: launched editor process"
    );
    Ok(())
}
