use crate::error::{CaptureError, CaptureResult};

/// Describes the dimensions of a captured image in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resolution {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
}

impl Resolution {
    /// Returns the number of pixels represented by this resolution.
    pub fn pixel_count(self) -> usize {
        self.width as usize * self.height as usize
    }
}

/// Describes the pixel memory layout of a captured frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    /// Packed 8-bit per channel red/green/blue/alpha layout.
    Rgba8,
}

impl PixelFormat {
    /// Returns the number of bytes required for one pixel of this format.
    pub fn bytes_per_pixel(self) -> usize {
        match self {
            Self::Rgba8 => 4,
        }
    }
}

/// Describes the encoded format for an output artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    /// Portable Network Graphics.
    Png,
}

impl ImageFormat {
    /// Returns the canonical file extension for this image format.
    pub fn file_extension(self) -> &'static str {
        match self {
            Self::Png => "png",
        }
    }
}

/// Defines how to handle an existing output file path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SaveConflictStrategy {
    /// Replace an existing file at the destination path.
    Overwrite,
    /// Pick a new filename when the destination path already exists.
    #[default]
    Rename,
    /// Return an error when the destination path already exists.
    Error,
}

/// Defines a rectangular region in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureRegion {
    /// Left coordinate in pixels.
    pub x: u32,
    /// Top coordinate in pixels.
    pub y: u32,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
}

impl CaptureRegion {
    /// Creates a validated capture region.
    pub fn new(x: u32, y: u32, width: u32, height: u32) -> CaptureResult<Self> {
        if width == 0 || height == 0 {
            return Err(CaptureError::InvalidRequest {
                message: "capture region width and height must be greater than zero".to_string(),
            });
        }

        Ok(Self {
            x,
            y,
            width,
            height,
        })
    }

    /// Returns `true` when the region is fully inside the provided resolution.
    pub fn fits_within(self, resolution: Resolution) -> bool {
        let x2 = self.x.saturating_add(self.width);
        let y2 = self.y.saturating_add(self.height);
        x2 <= resolution.width && y2 <= resolution.height
    }
}

/// Describes what kind of screen content should be captured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CaptureArea {
    /// Capture the full virtual screen.
    #[default]
    Fullscreen,
    /// Capture a specific rectangular region.
    Region(CaptureRegion),
    /// Ask the platform interaction layer to let the user select a region.
    InteractiveRegion,
}

/// Represents a backend-agnostic capture request.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CaptureRequest {
    /// The requested capture area.
    pub area: CaptureArea,
}

impl CaptureRequest {
    /// Creates a request for a full-screen capture.
    pub fn fullscreen() -> Self {
        Self {
            area: CaptureArea::Fullscreen,
        }
    }

    /// Creates a request for a specific rectangular region.
    pub fn region(region: CaptureRegion) -> Self {
        Self {
            area: CaptureArea::Region(region),
        }
    }

    /// Creates a request that asks the platform to let the user select a region.
    pub fn interactive_region() -> Self {
        Self {
            area: CaptureArea::InteractiveRegion,
        }
    }
}

/// Defines how a captured image should be persisted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveOptions {
    /// The destination image format.
    pub format: ImageFormat,
    /// The destination filesystem path.
    pub output_path: std::path::PathBuf,
    /// The strategy used when the destination file already exists.
    pub conflict_strategy: SaveConflictStrategy,
}

impl SaveOptions {
    /// Creates save options for a specific path and format.
    pub fn new(format: ImageFormat, output_path: impl Into<std::path::PathBuf>) -> Self {
        Self {
            format,
            output_path: output_path.into(),
            conflict_strategy: SaveConflictStrategy::default(),
        }
    }

    /// Creates save options for a specific path, format, and conflict strategy.
    pub fn with_conflict_strategy(
        format: ImageFormat,
        output_path: impl Into<std::path::PathBuf>,
        conflict_strategy: SaveConflictStrategy,
    ) -> Self {
        Self {
            format,
            output_path: output_path.into(),
            conflict_strategy,
        }
    }
}

/// Represents a full frame of captured pixels and associated metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedImage {
    /// The image dimensions.
    pub resolution: Resolution,
    /// The pixel layout of `pixels`.
    pub pixel_format: PixelFormat,
    /// Raw image pixels.
    pub pixels: Vec<u8>,
}

impl CapturedImage {
    /// Creates a captured image after validating the pixel buffer length.
    pub fn new(
        resolution: Resolution,
        pixel_format: PixelFormat,
        pixels: Vec<u8>,
    ) -> CaptureResult<Self> {
        let expected = resolution.pixel_count() * pixel_format.bytes_per_pixel();
        if pixels.len() != expected {
            return Err(CaptureError::InvalidImageBuffer {
                expected,
                actual: pixels.len(),
            });
        }

        Ok(Self {
            resolution,
            pixel_format,
            pixels,
        })
    }

    /// Returns a cropped image for the requested region.
    pub fn crop(&self, region: CaptureRegion) -> CaptureResult<Self> {
        if !region.fits_within(self.resolution) {
            return Err(CaptureError::InvalidRequest {
                message: format!(
                    "capture region ({}, {}, {}, {}) is outside image bounds {}x{}",
                    region.x,
                    region.y,
                    region.width,
                    region.height,
                    self.resolution.width,
                    self.resolution.height
                ),
            });
        }

        let bytes_per_pixel = self.pixel_format.bytes_per_pixel();
        let stride = self.resolution.width as usize * bytes_per_pixel;
        let mut pixels = Vec::with_capacity(region.width as usize * region.height as usize * bytes_per_pixel);

        for row in region.y as usize..(region.y + region.height) as usize {
            let start = row * stride + region.x as usize * bytes_per_pixel;
            let end = start + region.width as usize * bytes_per_pixel;
            pixels.extend_from_slice(&self.pixels[start..end]);
        }

        Self::new(
            Resolution {
                width: region.width,
                height: region.height,
            },
            self.pixel_format,
            pixels,
        )
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{
        CaptureArea, CaptureRegion, CaptureRequest, CapturedImage, ImageFormat, PixelFormat,
        Resolution, SaveConflictStrategy, SaveOptions,
    };

    #[test]
    fn captured_image_rejects_invalid_buffer_size() {
        let result = CapturedImage::new(
            Resolution {
                width: 2,
                height: 2,
            },
            PixelFormat::Rgba8,
            vec![0; 15],
        );

        assert!(result.is_err());
    }

    #[test]
    fn fullscreen_request_uses_fullscreen_area() {
        assert_eq!(CaptureRequest::fullscreen().area, CaptureArea::Fullscreen);
    }

    #[test]
    fn save_options_preserve_path_and_format() {
        let options = SaveOptions::new(ImageFormat::Png, "shot.png");
        assert_eq!(options.format, ImageFormat::Png);
        assert_eq!(options.output_path, PathBuf::from("shot.png"));
        assert_eq!(options.conflict_strategy, SaveConflictStrategy::Rename);
    }

    #[test]
    fn region_request_uses_region_area() {
        let region = CaptureRegion::new(10, 20, 30, 40).expect("region");
        assert_eq!(CaptureRequest::region(region).area, CaptureArea::Region(region));
    }

    #[test]
    fn captured_image_crop_returns_sub_region() {
        let image = CapturedImage::new(
            Resolution {
                width: 2,
                height: 2,
            },
            PixelFormat::Rgba8,
            vec![
                1, 0, 0, 255, 2, 0, 0, 255,
                3, 0, 0, 255, 4, 0, 0, 255,
            ],
        )
        .expect("image");

        let cropped = image
            .crop(CaptureRegion::new(1, 0, 1, 2).expect("region"))
            .expect("crop");

        assert_eq!(cropped.resolution.width, 1);
        assert_eq!(cropped.resolution.height, 2);
        assert_eq!(cropped.pixels, vec![2, 0, 0, 255, 4, 0, 0, 255]);
    }
}
