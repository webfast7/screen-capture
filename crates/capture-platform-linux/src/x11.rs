use capture_core::{
    CaptureArea, CaptureError, CaptureRegion, CaptureRequest, CaptureResult, CapturedImage,
    PixelFormat, Resolution, ScreenCaptureBackend,
};
use tracing::debug;
use x11rb::{
    connection::Connection,
    protocol::{
        Event,
        xproto::{
            ConnectionExt, CreateGCAux, CursorEnum, EventMask, GetImageReply, GrabMode,
            GrabStatus, Gcontext, GX, ImageFormat, Rectangle, SubwindowMode, Visualtype, Window,
        },
    },
    rust_connection::RustConnection,
};

/// X11-based screen capture backend for full-screen screenshots.
#[derive(Debug, Default)]
pub struct X11Backend;

impl X11Backend {
    /// Creates an X11 capture backend by validating connectivity to the X server.
    pub fn new() -> CaptureResult<Self> {
        let _ = RustConnection::connect(None).map_err(|error| CaptureError::Backend {
            message: format!("failed to connect to X server: {error}"),
        })?;

        Ok(Self)
    }
}

/// Resolves the top-level X11 window currently under the mouse pointer into a capture region.
pub fn window_region_under_pointer() -> CaptureResult<CaptureRegion> {
    let (connection, screen_num) =
        RustConnection::connect(None).map_err(|error| CaptureError::Backend {
            message: format!("failed to connect to X server: {error}"),
        })?;

    window_region_under_pointer_with_connection(&connection, screen_num)
}

impl ScreenCaptureBackend for X11Backend {
    fn name(&self) -> &'static str {
        "x11"
    }

    fn capture(&self, request: &CaptureRequest) -> CaptureResult<CapturedImage> {
        let (connection, screen_num) =
            RustConnection::connect(None).map_err(|error| CaptureError::Backend {
                message: format!("failed to connect to X server: {error}"),
            })?;

        let setup = connection.setup();
        let screen = &setup.roots[screen_num];
        let reply = connection
            .get_image(
                ImageFormat::Z_PIXMAP,
                screen.root,
                0,
                0,
                screen.width_in_pixels,
                screen.height_in_pixels,
                u32::MAX,
            )
            .map_err(|error| CaptureError::Backend {
                message: format!("failed to request X11 image: {error}"),
            })?
            .reply()
            .map_err(|error| CaptureError::Backend {
                message: format!("failed to receive X11 image: {error}"),
            })?;

        let visual = find_root_visual(setup, screen.root_visual).ok_or_else(|| {
            CaptureError::Backend {
                message: "failed to resolve X11 root visual".to_string(),
            }
        })?;

        let rgba = decode_x11_image(&connection, screen.root_depth, visual, &reply)?;
        debug!(
            width = screen.width_in_pixels,
            height = screen.height_in_pixels,
            "captured screen via X11"
        );

        let image = CapturedImage::new(
            Resolution {
                width: screen.width_in_pixels.into(),
                height: screen.height_in_pixels.into(),
            },
            PixelFormat::Rgba8,
            rgba,
        )?;

        match request.area {
            CaptureArea::Fullscreen => Ok(image),
            CaptureArea::Region(region) => image.crop(region),
            CaptureArea::InteractiveRegion => {
                let region = select_region(&connection, screen_num)?;
                image.crop(region)
            }
        }
    }
}

fn select_region(connection: &RustConnection, screen_num: usize) -> CaptureResult<CaptureRegion> {
    let setup = connection.setup();
    let screen = &setup.roots[screen_num];
    let root = screen.root;
    let gc = connection.generate_id().map_err(map_id_error)?;
    let keymap = load_keymap(connection)?;

    connection
        .create_gc(
            gc,
            root,
            &CreateGCAux::new()
                .function(GX::XOR)
                .foreground(screen.white_pixel ^ screen.black_pixel)
                .line_width(2)
                .subwindow_mode(SubwindowMode::INCLUDE_INFERIORS),
        )
        .map_err(map_connection_error)?
        .check()
        .map_err(map_reply_error)?;

    let grab_result = (|| -> CaptureResult<CaptureRegion> {
        let pointer_grab = connection
            .grab_pointer(
                false,
                root,
                EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE | EventMask::BUTTON_MOTION,
                GrabMode::ASYNC,
                GrabMode::ASYNC,
                x11rb::NONE,
                CursorEnum::NONE,
                x11rb::CURRENT_TIME,
            )
            .map_err(map_connection_error)?
            .reply()
            .map_err(map_reply_error)?;
        if pointer_grab.status != GrabStatus::SUCCESS {
            return Err(CaptureError::Backend {
                message: format!("failed to grab X11 pointer: {:?}", pointer_grab.status),
            });
        }

        let keyboard_grab = connection
            .grab_keyboard(false, root, x11rb::CURRENT_TIME, GrabMode::ASYNC, GrabMode::ASYNC)
            .map_err(map_connection_error)?
            .reply()
            .map_err(map_reply_error)?;
        if keyboard_grab.status != GrabStatus::SUCCESS {
            return Err(CaptureError::Backend {
                message: format!("failed to grab X11 keyboard: {:?}", keyboard_grab.status),
            });
        }

        let mut state = SelectionState::default();

        loop {
            match connection.wait_for_event().map_err(map_connection_error)? {
                Event::ButtonPress(event) => {
                    if event.detail == 1 {
                        state.begin(event.root_x, event.root_y);
                    }
                }
                Event::MotionNotify(event) => {
                    if state.is_dragging {
                        redraw_selection(connection, root, gc, state.current_rectangle())?;
                        state.update(event.root_x, event.root_y);
                        redraw_selection(connection, root, gc, state.current_rectangle())?;
                    }
                }
                Event::ButtonRelease(event) => {
                    if event.detail == 1 && state.is_dragging {
                        redraw_selection(connection, root, gc, state.current_rectangle())?;
                        state.finish(event.root_x, event.root_y);
                        redraw_selection(connection, root, gc, state.current_rectangle())?;
                    }
                }
                Event::KeyPress(event) => match lookup_keysym(&keymap, event.detail) {
                    Some(XK_ESCAPE) => {
                        redraw_selection(connection, root, gc, state.current_rectangle())?;
                        return Err(CaptureError::Cancelled {
                            message: "user pressed Escape while selecting a region".to_string(),
                        });
                    }
                    Some(XK_RETURN) => {
                        redraw_selection(connection, root, gc, state.current_rectangle())?;
                        let region = state.final_region.ok_or_else(|| CaptureError::InvalidRequest {
                            message: "drag to create a region before pressing Enter".to_string(),
                        })?;
                        return Ok(region);
                    }
                    _ => {}
                },
                _ => {}
            }
        }
    })();

    let _ = connection.ungrab_pointer(x11rb::CURRENT_TIME);
    let _ = connection.ungrab_keyboard(x11rb::CURRENT_TIME);
    let _ = connection.free_gc(gc);
    let _ = connection.flush();

    grab_result
}

fn window_region_under_pointer_with_connection(
    connection: &RustConnection,
    screen_num: usize,
) -> CaptureResult<CaptureRegion> {
    let setup = connection.setup();
    let screen = &setup.roots[screen_num];
    let deepest = find_deepest_window_under_pointer(connection, screen.root)?.ok_or_else(|| {
        CaptureError::Backend {
            message: "no X11 window found under the pointer".to_string(),
        }
    })?;
    let top_level = top_level_window_for(connection, screen.root, deepest)?;
    let geometry = connection
        .get_geometry(top_level)
        .map_err(map_connection_error)?
        .reply()
        .map_err(map_reply_error)?;
    let translated = connection
        .translate_coordinates(top_level, screen.root, 0, 0)
        .map_err(map_connection_error)?
        .reply()
        .map_err(map_reply_error)?;

    let mut x = i32::from(translated.dst_x);
    let mut y = i32::from(translated.dst_y);
    let mut width = i32::from(geometry.width);
    let mut height = i32::from(geometry.height);

    if x < 0 {
        width += x;
        x = 0;
    }
    if y < 0 {
        height += y;
        y = 0;
    }

    let max_width = i32::from(screen.width_in_pixels).saturating_sub(x);
    let max_height = i32::from(screen.height_in_pixels).saturating_sub(y);
    width = width.min(max_width);
    height = height.min(max_height);

    if width <= 0 || height <= 0 {
        return Err(CaptureError::InvalidRequest {
            message: "window under pointer does not have a visible on-screen area".to_string(),
        });
    }

    CaptureRegion::new(
        u32::try_from(x).map_err(|_| CaptureError::InvalidRequest {
            message: "window X coordinate is outside supported range".to_string(),
        })?,
        u32::try_from(y).map_err(|_| CaptureError::InvalidRequest {
            message: "window Y coordinate is outside supported range".to_string(),
        })?,
        u32::try_from(width).map_err(|_| CaptureError::InvalidRequest {
            message: "window width is outside supported range".to_string(),
        })?,
        u32::try_from(height).map_err(|_| CaptureError::InvalidRequest {
            message: "window height is outside supported range".to_string(),
        })?,
    )
}

fn find_deepest_window_under_pointer(
    connection: &RustConnection,
    root: Window,
) -> CaptureResult<Option<Window>> {
    let mut current = root;
    let mut deepest = None;

    loop {
        let reply = connection
            .query_pointer(current)
            .map_err(map_connection_error)?
            .reply()
            .map_err(map_reply_error)?;
        if reply.child == x11rb::NONE {
            return Ok(deepest);
        }

        deepest = Some(reply.child);
        current = reply.child;
    }
}

fn top_level_window_for(
    connection: &RustConnection,
    root: Window,
    mut window: Window,
) -> CaptureResult<Window> {
    loop {
        let tree = connection
            .query_tree(window)
            .map_err(map_connection_error)?
            .reply()
            .map_err(map_reply_error)?;

        if tree.parent == root || tree.parent == x11rb::NONE {
            return Ok(window);
        }

        window = tree.parent;
    }
}

fn redraw_selection(
    connection: &RustConnection,
    root: u32,
    gc: Gcontext,
    rectangle: Option<Rectangle>,
) -> CaptureResult<()> {
    if let Some(rectangle) = rectangle {
        connection
            .poly_rectangle(root, gc, &[rectangle])
            .map_err(map_connection_error)?
            .check()
            .map_err(map_reply_error)?;
        connection.flush().map_err(map_connection_error)?;
    }

    Ok(())
}

fn load_keymap(connection: &RustConnection) -> CaptureResult<Keymap> {
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

fn lookup_keysym(keymap: &Keymap, keycode: u8) -> Option<u32> {
    if keycode < keymap.min_keycode || keymap.keysyms_per_keycode == 0 {
        return None;
    }

    let offset = usize::from(keycode - keymap.min_keycode) * usize::from(keymap.keysyms_per_keycode);
    let slice = keymap
        .keysyms
        .get(offset..offset + usize::from(keymap.keysyms_per_keycode))?;

    slice.iter().copied().find(|keysym| *keysym != 0)
}

fn map_connection_error(error: x11rb::errors::ConnectionError) -> CaptureError {
    CaptureError::Backend {
        message: format!("X11 connection error: {error}"),
    }
}

fn map_reply_error(error: x11rb::errors::ReplyError) -> CaptureError {
    CaptureError::Backend {
        message: format!("X11 reply error: {error}"),
    }
}

fn map_id_error(error: x11rb::errors::ReplyOrIdError) -> CaptureError {
    CaptureError::Backend {
        message: format!("X11 resource allocation failed: {error}"),
    }
}

const XK_ESCAPE: u32 = 0xff1b;
const XK_RETURN: u32 = 0xff0d;

#[derive(Debug)]
struct Keymap {
    min_keycode: u8,
    keysyms_per_keycode: u8,
    keysyms: Vec<u32>,
}

#[derive(Debug, Default, Clone, Copy)]
struct SelectionState {
    start: Option<(i16, i16)>,
    current: Option<(i16, i16)>,
    final_region: Option<CaptureRegion>,
    is_dragging: bool,
}

impl SelectionState {
    fn begin(&mut self, x: i16, y: i16) {
        self.start = Some((x, y));
        self.current = Some((x, y));
        self.final_region = None;
        self.is_dragging = true;
    }

    fn update(&mut self, x: i16, y: i16) {
        self.current = Some((x, y));
    }

    fn finish(&mut self, x: i16, y: i16) {
        self.current = Some((x, y));
        self.is_dragging = false;
        self.final_region = self
            .region_from_points()
            .and_then(|(x, y, width, height)| CaptureRegion::new(x, y, width, height).ok());
    }

    fn current_rectangle(&self) -> Option<Rectangle> {
        self.region_from_points().map(|(x, y, width, height)| Rectangle {
            x: i16::try_from(x).unwrap_or(i16::MAX),
            y: i16::try_from(y).unwrap_or(i16::MAX),
            width: u16::try_from(width.saturating_sub(1)).unwrap_or(u16::MAX),
            height: u16::try_from(height.saturating_sub(1)).unwrap_or(u16::MAX),
        })
    }

    fn region_from_points(&self) -> Option<(u32, u32, u32, u32)> {
        let (start_x, start_y) = self.start?;
        let (current_x, current_y) = self.current?;
        let x1 = i32::from(start_x.min(current_x));
        let y1 = i32::from(start_y.min(current_y));
        let x2 = i32::from(start_x.max(current_x));
        let y2 = i32::from(start_y.max(current_y));

        Some((
            u32::try_from(x1).ok()?,
            u32::try_from(y1).ok()?,
            u32::try_from((x2 - x1).max(1)).ok()?,
            u32::try_from((y2 - y1).max(1)).ok()?,
        ))
    }
}

fn find_root_visual<'a>(
    setup: &'a x11rb::protocol::xproto::Setup,
    visual_id: u32,
) -> Option<&'a Visualtype> {
    setup
        .roots
        .iter()
        .flat_map(|screen| screen.allowed_depths.iter())
        .flat_map(|depth| depth.visuals.iter())
        .find(|visual| visual.visual_id == visual_id)
}

fn decode_x11_image(
    connection: &RustConnection,
    depth: u8,
    visual: &Visualtype,
    reply: &GetImageReply,
) -> CaptureResult<Vec<u8>> {
    let format = connection
        .setup()
        .pixmap_formats
        .iter()
        .find(|candidate| candidate.depth == depth)
        .ok_or_else(|| CaptureError::Backend {
            message: format!("missing pixmap format for X11 depth {depth}"),
        })?;
    let bytes_per_pixel = usize::from(format.bits_per_pixel.div_ceil(8));
    if bytes_per_pixel == 0 {
        return Err(CaptureError::Backend {
            message: "invalid X11 bytes_per_pixel value".to_string(),
        });
    }

    let pixel_count = reply.data.len() / bytes_per_pixel;
    let mut rgba = Vec::with_capacity(pixel_count * 4);
    let little_endian = matches!(
        connection.setup().image_byte_order,
        x11rb::protocol::xproto::ImageOrder::LSB_FIRST
    );

    for chunk in reply.data.chunks_exact(bytes_per_pixel) {
        let pixel = match bytes_per_pixel {
            1 => u32::from(chunk[0]),
            2 => {
                let bytes = [chunk[0], chunk[1]];
                if little_endian {
                    u16::from_le_bytes(bytes).into()
                } else {
                    u16::from_be_bytes(bytes).into()
                }
            }
            3 => {
                if little_endian {
                    u32::from(chunk[0]) | (u32::from(chunk[1]) << 8) | (u32::from(chunk[2]) << 16)
                } else {
                    u32::from(chunk[2]) | (u32::from(chunk[1]) << 8) | (u32::from(chunk[0]) << 16)
                }
            }
            4 => {
                let bytes = [chunk[0], chunk[1], chunk[2], chunk[3]];
                if little_endian {
                    u32::from_le_bytes(bytes)
                } else {
                    u32::from_be_bytes(bytes)
                }
            }
            _ => {
                return Err(CaptureError::Unsupported {
                    message: format!("unsupported X11 pixel width: {bytes_per_pixel} bytes"),
                });
            }
        };

        rgba.push(scale_channel(pixel, visual.red_mask));
        rgba.push(scale_channel(pixel, visual.green_mask));
        rgba.push(scale_channel(pixel, visual.blue_mask));
        rgba.push(255);
    }

    Ok(rgba)
}

fn scale_channel(pixel: u32, mask: u32) -> u8 {
    if mask == 0 {
        return 0;
    }

    let shift = mask.trailing_zeros();
    let max_value = mask >> shift;
    let raw = (pixel & mask) >> shift;

    if max_value == 0 {
        0
    } else {
        ((raw * 255) / max_value) as u8
    }
}

#[cfg(test)]
mod tests {
    use super::scale_channel;

    #[test]
    fn scale_channel_expands_to_8bit() {
        let pixel = 0b10101u32 << 11;
        let mask = 0b11111u32 << 11;

        assert_eq!(scale_channel(pixel, mask), 172);
    }
}
