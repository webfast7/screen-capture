pub mod app;

use std::{
    fs,
    path::{Path, PathBuf},
};

use app::{AnnotationEditorApp, EditorBootstrap};
use capture_core::{CaptureError, CaptureRequest, CapturedImage, PixelFormat};
use capture_platform_linux::{BackendSelection, detect_backend};
use eframe::{NativeOptions, egui::ViewportBuilder};
use image::ImageReader;
use tracing::info;

/// Launch the annotation editor after capturing an interactive screenshot region.
pub fn run_editor_with_capture(font: Option<PathBuf>) -> Result<(), Box<dyn std::error::Error>> {
    let image = capture_interactive_region()?;
    run_editor_with_image(image, font)
}

/// Launch the annotation editor for an existing image file.
pub fn run_editor_with_input(
    input: &Path,
    font: Option<PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    let image = load_image_from_path(input)?;
    run_editor_with_image(image, font)
}

/// Resolve the default annotation font path, preferring the vendored Chinese font.
pub fn default_font_path() -> Option<PathBuf> {
    let bundled = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/fonts/DroidSansFallbackFull.ttf");
    let candidates = [
        bundled.as_path(),
        Path::new("/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf"),
        Path::new("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"),
        Path::new("/home/ryk/.local/share/fonts/MapleMono-NF-CN-Regular.ttf"),
    ];

    candidates
        .iter()
        .map(|path| path.to_path_buf())
        .find(|path| fs::metadata(path).is_ok())
}

fn run_editor_with_image(
    image: CapturedImage,
    font: Option<PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    let font_path = font.or_else(default_font_path);
    if let Some(path) = font_path.as_ref() {
        info!(path = %path.display(), "editor: using annotation font");
    }

    let bootstrap = EditorBootstrap::new(image, font_path);
    let options = NativeOptions {
        viewport: ViewportBuilder::default()
            .with_title("截图标注编辑器")
            .with_inner_size([1280.0, 860.0])
            .with_min_inner_size([900.0, 640.0]),
        ..Default::default()
    };

    eframe::run_native(
        "截图标注编辑器",
        options,
        Box::new(move |cc| Box::new(AnnotationEditorApp::new(cc, bootstrap))),
    )?;

    Ok(())
}

fn capture_interactive_region() -> Result<CapturedImage, CaptureError> {
    let backend = detect_backend(BackendSelection::Auto)?;
    backend.capture(&CaptureRequest::interactive_region())
}

fn load_image_from_path(path: &Path) -> Result<CapturedImage, Box<dyn std::error::Error>> {
    let image = ImageReader::open(path)?.decode()?.to_rgba8();
    let (width, height) = image.dimensions();
    Ok(CapturedImage::new(
        capture_core::Resolution { width, height },
        PixelFormat::Rgba8,
        image.into_raw(),
    )?)
}
