use std::thread;

use capture_core::CaptureError;
use tracing::{error, info};
use x11rb::{
    connection::Connection,
    protocol::{
        Event,
        xproto::{ConnectionExt, GrabMode, KeyButMask, ModMask},
    },
    rust_connection::RustConnection,
};

use crate::launch::{EditorLaunch, spawn_editor_process};

const XK_A: u32 = 0x0061;
const XK_NUM_LOCK: u32 = 0xff7f;

pub fn spawn_hotkey_listener() {
    thread::spawn(|| {
        if let Err(error) = run_hotkey_loop() {
            error!(error = %error, "tray hotkey: listener stopped");
        }
    });
}

fn run_hotkey_loop() -> Result<(), CaptureError> {
    let (connection, screen_num) =
        RustConnection::connect(None).map_err(|error| CaptureError::Backend {
            message: format!("failed to connect to X server for hotkey listener: {error}"),
        })?;
    let setup = connection.setup();
    let screen = &setup.roots[screen_num];
    let keymap = load_keymap(&connection)?;
    let trigger_keycode =
        find_keycode_for_keysym(&keymap, XK_A).ok_or_else(|| CaptureError::Unsupported {
            message: "failed to resolve X11 keycode for shortcut key A".to_string(),
        })?;
    let numlock_mask = detect_numlock_mask(&connection, &keymap)?;
    let modifiers = hotkey_modifier_variants(numlock_mask);

    for modifier in modifiers {
        connection
            .grab_key(
                false,
                screen.root,
                modifier,
                trigger_keycode,
                GrabMode::ASYNC,
                GrabMode::ASYNC,
            )
            .map_err(map_connection_error)?
            .check()
            .map_err(map_reply_error)?;
    }
    connection.flush().map_err(map_connection_error)?;

    info!(shortcut = "Ctrl+Alt+A", "tray hotkey: listener armed");
    let mut hotkey_pressed = false;

    loop {
        match connection.wait_for_event().map_err(map_connection_error)? {
            Event::KeyPress(event)
                if event.detail == trigger_keycode
                    && has_trigger_modifiers(event.state, numlock_mask) =>
            {
                if !hotkey_pressed {
                    hotkey_pressed = true;
                    match spawn_editor_process(EditorLaunch::Region) {
                        Ok(()) => info!("tray hotkey: launched region editor"),
                        Err(error) => error!(error = %error, "tray hotkey: failed to launch region editor"),
                    }
                }
            }
            Event::KeyRelease(event) if event.detail == trigger_keycode => {
                hotkey_pressed = false;
            }
            _ => {}
        }
    }
}

fn has_trigger_modifiers(state: KeyButMask, numlock_mask: ModMask) -> bool {
    let normalized = ModMask::from(
        u16::from(state)
            & !(u16::from(ModMask::LOCK) | u16::from(numlock_mask))
            & 0x00ff,
    );
    normalized == (ModMask::CONTROL | ModMask::M1)
}

fn hotkey_modifier_variants(numlock_mask: ModMask) -> [ModMask; 4] {
    [
        ModMask::CONTROL | ModMask::M1,
        ModMask::CONTROL | ModMask::M1 | ModMask::LOCK,
        ModMask::CONTROL | ModMask::M1 | numlock_mask,
        ModMask::CONTROL | ModMask::M1 | ModMask::LOCK | numlock_mask,
    ]
}

#[derive(Debug)]
struct Keymap {
    min_keycode: u8,
    keysyms_per_keycode: u8,
    keysyms: Vec<u32>,
}

fn load_keymap(connection: &RustConnection) -> Result<Keymap, CaptureError> {
    let setup = connection.setup();
    let count = setup.max_keycode - setup.min_keycode + 1;
    let reply = connection
        .get_keyboard_mapping(setup.min_keycode, count)
        .map_err(map_connection_error)?
        .reply()
        .map_err(map_reply_error)?;

    Ok(Keymap {
        min_keycode: setup.min_keycode,
        keysyms_per_keycode: reply.keysyms_per_keycode,
        keysyms: reply.keysyms,
    })
}

fn find_keycode_for_keysym(keymap: &Keymap, needle: u32) -> Option<u8> {
    if keymap.keysyms_per_keycode == 0 {
        return None;
    }

    let width = usize::from(keymap.keysyms_per_keycode);
    for (index, chunk) in keymap.keysyms.chunks_exact(width).enumerate() {
        if chunk.iter().any(|keysym| *keysym == needle) {
            return Some(keymap.min_keycode + u8::try_from(index).ok()?);
        }
    }

    None
}

fn detect_numlock_mask(
    connection: &RustConnection,
    keymap: &Keymap,
) -> Result<ModMask, CaptureError> {
    let reply = connection
        .get_modifier_mapping()
        .map_err(map_connection_error)?
        .reply()
        .map_err(map_reply_error)?;
    let per_modifier = usize::from(reply.keycodes_per_modifier());

    for modifier_index in 0..8usize {
        let start = modifier_index * per_modifier;
        let end = start + per_modifier;
        let keycodes = &reply.keycodes[start..end];

        if keycodes.iter().copied().filter(|code| *code != 0).any(|keycode| {
            keycode_keysyms(keymap, keycode).iter().any(|keysym| *keysym == XK_NUM_LOCK)
        }) {
            return Ok(match modifier_index {
                0 => ModMask::SHIFT,
                1 => ModMask::LOCK,
                2 => ModMask::CONTROL,
                3 => ModMask::M1,
                4 => ModMask::M2,
                5 => ModMask::M3,
                6 => ModMask::M4,
                7 => ModMask::M5,
                _ => ModMask::M2,
            });
        }
    }

    Ok(ModMask::M2)
}

fn keycode_keysyms<'a>(keymap: &'a Keymap, keycode: u8) -> &'a [u32] {
    if keycode < keymap.min_keycode || keymap.keysyms_per_keycode == 0 {
        return &[];
    }

    let width = usize::from(keymap.keysyms_per_keycode);
    let offset = usize::from(keycode - keymap.min_keycode) * width;
    keymap.keysyms.get(offset..offset + width).unwrap_or(&[])
}

fn map_connection_error(error: x11rb::errors::ConnectionError) -> CaptureError {
    CaptureError::Backend {
        message: format!("X11 hotkey connection error: {error}"),
    }
}

fn map_reply_error(error: x11rb::errors::ReplyError) -> CaptureError {
    CaptureError::Backend {
        message: format!("X11 hotkey reply error: {error}"),
    }
}
