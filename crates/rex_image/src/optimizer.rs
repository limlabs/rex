use image::imageops::FilterType;
use image::ImageReader;
use std::io::Cursor;
use tracing::debug;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    WebP,
    Jpeg,
    Png,
}

impl OutputFormat {
    pub fn content_type(self) -> &'static str {
        match self {
            Self::WebP => "image/webp",
            Self::Jpeg => "image/jpeg",
            Self::Png => "image/png",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::WebP => "webp",
            Self::Jpeg => "jpg",
            Self::Png => "png",
        }
    }
}

#[derive(Debug, Clone)]
pub struct OptimizeParams {
    pub width: u32,
    pub quality: u8,
    pub format: OutputFormat,
}

#[derive(Debug, thiserror::Error)]
pub enum OptimizeError {
    #[error("invalid width: {0} (must be 16–4096)")]
    InvalidWidth(u32),
    #[error("invalid quality: {0} (must be 1–100)")]
    InvalidQuality(u8),
    #[error("input too large: {0} bytes (max 10MB)")]
    InputTooLarge(usize),
    #[error("image decode error: {0}")]
    Decode(#[from] image::ImageError),
    #[error("encode error: {0}")]
    Encode(String),
}

const MAX_INPUT_SIZE: usize = 10 * 1024 * 1024; // 10MB
const MIN_WIDTH: u32 = 16;
const MAX_WIDTH: u32 = 4096;

/// Decode, resize, and re-encode an image.
pub fn optimize(src_bytes: &[u8], params: &OptimizeParams) -> Result<Vec<u8>, OptimizeError> {
    if params.width < MIN_WIDTH || params.width > MAX_WIDTH {
        return Err(OptimizeError::InvalidWidth(params.width));
    }
    if params.quality == 0 || params.quality > 100 {
        return Err(OptimizeError::InvalidQuality(params.quality));
    }
    if src_bytes.len() > MAX_INPUT_SIZE {
        return Err(OptimizeError::InputTooLarge(src_bytes.len()));
    }

    let reader = ImageReader::new(Cursor::new(src_bytes))
        .with_guessed_format()
        .map_err(|e| OptimizeError::Decode(image::ImageError::IoError(e)))?;
    let img = reader.decode()?;

    // Don't upscale
    let target_width = params.width.min(img.width());
    let resized = if target_width < img.width() {
        let scale = target_width as f64 / img.width() as f64;
        let target_height = (img.height() as f64 * scale).round() as u32;
        debug!(
            original_w = img.width(),
            original_h = img.height(),
            target_w = target_width,
            target_h = target_height,
            "resizing image"
        );
        img.resize_exact(target_width, target_height, FilterType::Lanczos3)
    } else {
        img
    };

    encode(&resized, params.format, params.quality)
}

/// Generate a tiny LQIP blur placeholder (8px wide).
pub fn generate_blur_placeholder(src_bytes: &[u8]) -> Result<(String, u32, u32), OptimizeError> {
    let reader = ImageReader::new(Cursor::new(src_bytes))
        .with_guessed_format()
        .map_err(|e| OptimizeError::Decode(image::ImageError::IoError(e)))?;
    let img = reader.decode()?;

    let blur_width: u32 = 8;
    let scale = blur_width as f64 / img.width() as f64;
    let blur_height = (img.height() as f64 * scale).round() as u32;
    let tiny = img.resize_exact(blur_width, blur_height.max(1), FilterType::Lanczos3);

    // Encode as JPEG for smallest data URL
    let encoded = encode(&tiny, OutputFormat::Jpeg, 50)?;
    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &encoded);
    let data_url = format!("data:image/jpeg;base64,{b64}");

    Ok((data_url, img.width(), img.height()))
}

/// Pick the best output format from an Accept header.
/// Always returns JPEG for auto-negotiation — it's the best lossy format
/// available until lossy WebP or AVIF support is added. Users can explicitly
/// request other formats via the `f=` query parameter.
pub fn negotiate_format(_accept: &str) -> OutputFormat {
    OutputFormat::Jpeg
}

fn encode(
    img: &image::DynamicImage,
    format: OutputFormat,
    quality: u8,
) -> Result<Vec<u8>, OptimizeError> {
    let mut buf = Vec::new();
    let mut cursor = Cursor::new(&mut buf);

    match format {
        OutputFormat::Jpeg => {
            let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut cursor, quality);
            img.write_with_encoder(encoder)?;
        }
        OutputFormat::Png => {
            let encoder = image::codecs::png::PngEncoder::new(&mut cursor);
            img.write_with_encoder(encoder)?;
        }
        OutputFormat::WebP => {
            // image 0.25 WebP encoder only supports lossless
            let encoder = image::codecs::webp::WebPEncoder::new_lossless(&mut cursor);
            img.write_with_encoder(encoder)?;
        }
    }

    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_jpeg(w: u32, h: u32) -> Vec<u8> {
        let img = image::DynamicImage::new_rgb8(w, h);
        let mut buf = Vec::new();
        let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(Cursor::new(&mut buf), 90);
        img.write_with_encoder(encoder).expect("encode test jpeg");
        buf
    }

    #[test]
    fn resize_jpeg() {
        let src = test_jpeg(800, 600);
        let result = optimize(
            &src,
            &OptimizeParams {
                width: 400,
                quality: 75,
                format: OutputFormat::Jpeg,
            },
        )
        .expect("optimize");
        // Should produce valid JPEG that's smaller than the 800px version at same quality
        assert!(!result.is_empty());
        let decoded = ImageReader::new(Cursor::new(&result))
            .with_guessed_format()
            .unwrap()
            .decode()
            .unwrap();
        assert_eq!(decoded.width(), 400);
        assert_eq!(decoded.height(), 300);
    }

    #[test]
    fn webp_encode() {
        let src = test_jpeg(200, 100);
        let result = optimize(
            &src,
            &OptimizeParams {
                width: 200,
                quality: 75,
                format: OutputFormat::WebP,
            },
        )
        .expect("optimize to webp");
        assert!(!result.is_empty());
        // WebP files start with RIFF header
        assert_eq!(&result[..4], b"RIFF");
    }

    #[test]
    fn quality_affects_size() {
        let src = test_jpeg(400, 300);
        let low_q = optimize(
            &src,
            &OptimizeParams {
                width: 400,
                quality: 10,
                format: OutputFormat::Jpeg,
            },
        )
        .expect("low quality");
        let high_q = optimize(
            &src,
            &OptimizeParams {
                width: 400,
                quality: 95,
                format: OutputFormat::Jpeg,
            },
        )
        .expect("high quality");
        assert!(
            low_q.len() < high_q.len(),
            "low quality ({}) should be smaller than high quality ({})",
            low_q.len(),
            high_q.len()
        );
    }

    #[test]
    fn invalid_width_errors() {
        let src = test_jpeg(100, 100);
        let err = optimize(
            &src,
            &OptimizeParams {
                width: 5,
                quality: 75,
                format: OutputFormat::Jpeg,
            },
        );
        assert!(matches!(err, Err(OptimizeError::InvalidWidth(5))));

        let err = optimize(
            &src,
            &OptimizeParams {
                width: 5000,
                quality: 75,
                format: OutputFormat::Jpeg,
            },
        );
        assert!(matches!(err, Err(OptimizeError::InvalidWidth(5000))));
    }

    #[test]
    fn too_large_errors() {
        let big = vec![0u8; MAX_INPUT_SIZE + 1];
        let err = optimize(
            &big,
            &OptimizeParams {
                width: 100,
                quality: 75,
                format: OutputFormat::Jpeg,
            },
        );
        assert!(matches!(err, Err(OptimizeError::InputTooLarge(_))));
    }

    #[test]
    fn no_upscale() {
        let src = test_jpeg(200, 100);
        let result = optimize(
            &src,
            &OptimizeParams {
                width: 800,
                quality: 75,
                format: OutputFormat::Jpeg,
            },
        )
        .expect("no upscale");
        let decoded = ImageReader::new(Cursor::new(&result))
            .with_guessed_format()
            .unwrap()
            .decode()
            .unwrap();
        // Should stay at original 200px, not upscale to 800
        assert_eq!(decoded.width(), 200);
    }

    #[test]
    fn blur_placeholder_generation() {
        let src = test_jpeg(800, 600);
        let (data_url, orig_w, orig_h) = generate_blur_placeholder(&src).expect("blur placeholder");
        assert!(data_url.starts_with("data:image/jpeg;base64,"));
        assert_eq!(orig_w, 800);
        assert_eq!(orig_h, 600);
        // Data URL should be small (< 1KB for an 8px image)
        assert!(
            data_url.len() < 1024,
            "data URL too large: {}",
            data_url.len()
        );
    }

    #[test]
    fn format_negotiation() {
        // Always JPEG for auto-negotiation (no lossy WebP/AVIF in image 0.25)
        assert_eq!(
            negotiate_format("text/html,image/webp,image/png,*/*"),
            OutputFormat::Jpeg
        );
        assert_eq!(
            negotiate_format("text/html,image/png,*/*"),
            OutputFormat::Jpeg
        );
        assert_eq!(negotiate_format("text/html,*/*"), OutputFormat::Jpeg);
    }
}
