use std::{
    fs,
    io::Cursor,
    path::{Path, PathBuf},
};

use image::{ColorType, ImageEncoder as _, codecs::png::PngEncoder as ImagePngEncoder};

use crate::{
    backend::ImageEncoder,
    error::{CaptureError, CaptureResult},
    model::{
        CapturedImage, CaptureRequest, ImageFormat, PixelFormat, SaveConflictStrategy, SaveOptions,
    },
};

/// Describes a persisted capture artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputArtifact {
    /// The path where the artifact was written.
    pub path: PathBuf,
    /// The encoded image format.
    pub format: ImageFormat,
    /// The number of bytes written to the output target.
    pub bytes_written: usize,
}

/// Writes encoded image bytes to a destination.
pub trait OutputTarget {
    /// Persists `bytes` according to the provided save options.
    fn write(&self, bytes: &[u8], options: &SaveOptions) -> CaptureResult<OutputArtifact>;
}

/// Encodes captured images into PNG bytes.
#[derive(Debug, Default, Clone, Copy)]
pub struct PngEncoder;

impl ImageEncoder for PngEncoder {
    fn format(&self) -> ImageFormat {
        ImageFormat::Png
    }

    fn encode(&self, image: &CapturedImage) -> CaptureResult<Vec<u8>> {
        if image.resolution.width == 0 || image.resolution.height == 0 || image.pixels.is_empty() {
            return Err(CaptureError::EmptyImage);
        }

        let color_type = match image.pixel_format {
            PixelFormat::Rgba8 => ColorType::Rgba8,
        };

        let mut encoded = Vec::new();
        let encoder = ImagePngEncoder::new(Cursor::new(&mut encoded));
        encoder
            .write_image(
                &image.pixels,
                image.resolution.width,
                image.resolution.height,
                color_type.into(),
            )
            .map_err(|error| CaptureError::Encoding {
                message: error.to_string(),
            })?;

        Ok(encoded)
    }
}

/// Writes encoded image bytes to a filesystem path.
#[derive(Debug, Default, Clone, Copy)]
pub struct FileOutputTarget;

impl OutputTarget for FileOutputTarget {
    fn write(&self, bytes: &[u8], options: &SaveOptions) -> CaptureResult<OutputArtifact> {
        let resolved_path = resolve_output_path(options)?;
        let parent = parent_dir(&resolved_path).ok_or_else(|| CaptureError::InvalidOutputPath {
            path: resolved_path.clone(),
        })?;

        fs::create_dir_all(parent).map_err(|error| CaptureError::Output {
            message: format!("failed to create output directory {parent:?}: {error}"),
        })?;

        match options.conflict_strategy {
            SaveConflictStrategy::Overwrite | SaveConflictStrategy::Rename => {
                fs::write(&resolved_path, bytes).map_err(|error| map_write_error(&resolved_path, error))?;
            }
            SaveConflictStrategy::Error => {
                write_new_file(&resolved_path, bytes)?;
            }
        }

        Ok(OutputArtifact {
            path: resolved_path,
            format: options.format,
            bytes_written: bytes.len(),
        })
    }
}

/// Executes the standard capture -> encode -> persist flow.
pub fn capture_and_save(
    backend: &dyn crate::backend::ScreenCaptureBackend,
    request: &CaptureRequest,
    encoder: &dyn ImageEncoder,
    target: &dyn OutputTarget,
    save_options: &SaveOptions,
) -> CaptureResult<OutputArtifact> {
    let image = backend.capture(request)?;
    let encoded = encoder.encode(&image)?;
    target.write(&encoded, save_options)
}

/// Executes the standard capture -> encode flow and returns encoded bytes.
pub fn capture_and_encode(
    backend: &dyn crate::backend::ScreenCaptureBackend,
    request: &CaptureRequest,
    encoder: &dyn ImageEncoder,
) -> CaptureResult<Vec<u8>> {
    let image = backend.capture(request)?;
    encoder.encode(&image)
}

fn resolve_output_path(options: &SaveOptions) -> CaptureResult<PathBuf> {
    match options.conflict_strategy {
        SaveConflictStrategy::Overwrite | SaveConflictStrategy::Error => Ok(options.output_path.clone()),
        SaveConflictStrategy::Rename => next_available_path(&options.output_path),
    }
}

fn parent_dir(path: &Path) -> Option<&Path> {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => Some(parent),
        Some(_) => Some(Path::new(".")),
        None => Some(Path::new(".")),
    }
}

fn next_available_path(path: &Path) -> CaptureResult<PathBuf> {
    if !path.exists() {
        return Ok(path.to_path_buf());
    }

    let parent = parent_dir(path)
        .ok_or_else(|| CaptureError::InvalidOutputPath {
            path: path.to_path_buf(),
        })?
        .to_path_buf();
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| CaptureError::InvalidOutputPath {
            path: path.to_path_buf(),
        })?;
    let extension = path.extension().and_then(|ext| ext.to_str());

    for index in 1..10_000 {
        let candidate_name = match extension {
            Some(extension) => format!("{stem}-{index}.{extension}"),
            None => format!("{stem}-{index}"),
        };
        let candidate = parent.join(candidate_name);
        if !candidate.exists() {
            return Ok(candidate);
        }
    }

    Err(CaptureError::Output {
        message: format!("failed to find an available filename near {:?}", path),
    })
}

fn write_new_file(path: &Path, bytes: &[u8]) -> CaptureResult<()> {
    use std::io::Write as _;

    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| map_write_error(path, error))?;
    file.write_all(bytes)
        .map_err(|error| map_write_error(path, error))
}

fn map_write_error(path: &Path, error: std::io::Error) -> CaptureError {
    if error.kind() == std::io::ErrorKind::PermissionDenied {
        return CaptureError::Output {
            message: format!("permission denied while writing {:?}", path),
        };
    }

    if error.kind() == std::io::ErrorKind::AlreadyExists {
        return CaptureError::Output {
            message: format!("output file already exists: {:?}", path),
        };
    }

    CaptureError::Output {
        message: format!("failed to write output file {:?}: {error}", path),
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        backend::{ImageEncoder, ScreenCaptureBackend},
        model::{
            CaptureRequest, CapturedImage, ImageFormat, PixelFormat, Resolution,
            SaveConflictStrategy, SaveOptions,
        },
        output::{capture_and_encode, capture_and_save, FileOutputTarget, OutputTarget, PngEncoder},
    };
    use std::{
        cell::RefCell,
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[derive(Debug)]
    struct StubBackend;

    impl ScreenCaptureBackend for StubBackend {
        fn name(&self) -> &'static str {
            "stub"
        }

        fn capture(&self, request: &CaptureRequest) -> crate::CaptureResult<CapturedImage> {
            assert_eq!(request.area, crate::CaptureArea::Fullscreen);
            CapturedImage::new(
                Resolution {
                    width: 1,
                    height: 1,
                },
                PixelFormat::Rgba8,
                vec![255, 0, 0, 255],
            )
        }
    }

    #[derive(Debug, Default)]
    struct StubOutputTarget {
        writes: RefCell<Vec<PathBuf>>,
    }

    impl OutputTarget for StubOutputTarget {
        fn write(
            &self,
            bytes: &[u8],
            options: &SaveOptions,
        ) -> crate::CaptureResult<crate::OutputArtifact> {
            self.writes.borrow_mut().push(options.output_path.clone());
            Ok(crate::OutputArtifact {
                path: options.output_path.clone(),
                format: options.format,
                bytes_written: bytes.len(),
            })
        }
    }

    #[test]
    fn png_encoder_writes_png_signature() {
        let image = CapturedImage::new(
            Resolution {
                width: 1,
                height: 1,
            },
            PixelFormat::Rgba8,
            vec![255, 0, 0, 255],
        )
        .expect("valid image");

        let bytes = PngEncoder.encode(&image).expect("png encoding");
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    }

    #[test]
    fn capture_pipeline_uses_unified_request_and_save_options() {
        let target = StubOutputTarget::default();
        let artifact = capture_and_save(
            &StubBackend,
            &CaptureRequest::fullscreen(),
            &PngEncoder,
            &target,
            &SaveOptions::new(ImageFormat::Png, "out.png"),
        )
        .expect("capture pipeline");

        assert_eq!(artifact.path, PathBuf::from("out.png"));
        assert_eq!(artifact.format, ImageFormat::Png);
        assert_eq!(target.writes.borrow().as_slice(), &[PathBuf::from("out.png")]);
    }

    #[test]
    fn capture_and_encode_returns_png_bytes() {
        let encoded = capture_and_encode(&StubBackend, &CaptureRequest::fullscreen(), &PngEncoder)
            .expect("encoded");
        assert_eq!(&encoded[..8], b"\x89PNG\r\n\x1a\n");
    }

    #[test]
    fn png_encoder_rejects_empty_image() {
        let image = CapturedImage::new(
            Resolution {
                width: 0,
                height: 0,
            },
            PixelFormat::Rgba8,
            Vec::new(),
        )
        .expect("shape-valid image");

        let error = PngEncoder.encode(&image).expect_err("empty image should fail");
        assert!(matches!(error, crate::CaptureError::EmptyImage));
    }

    #[test]
    fn rename_strategy_picks_next_available_filename() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(&temp_dir).expect("temp dir");
        let base_path = temp_dir.join("shot.png");
        fs::write(&base_path, b"existing").expect("seed file");

        let artifact = FileOutputTarget
            .write(
                b"new",
                &SaveOptions::with_conflict_strategy(
                    ImageFormat::Png,
                    &base_path,
                    SaveConflictStrategy::Rename,
                ),
            )
            .expect("renamed write");

        assert_eq!(artifact.path, temp_dir.join("shot-1.png"));
    }

    #[test]
    fn error_strategy_rejects_existing_path() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(&temp_dir).expect("temp dir");
        let base_path = temp_dir.join("shot.png");
        fs::write(&base_path, b"existing").expect("seed file");

        let error = FileOutputTarget
            .write(
                b"new",
                &SaveOptions::with_conflict_strategy(
                    ImageFormat::Png,
                    &base_path,
                    SaveConflictStrategy::Error,
                ),
            )
            .expect_err("existing path should fail");

        assert!(error.to_string().contains("already exists"));
    }

    fn unique_temp_dir() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        std::env::temp_dir().join(format!("screen-capture-test-{nonce}"))
    }
}
