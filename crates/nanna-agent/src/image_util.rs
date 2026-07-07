//! Image preprocessing utilities for LLM API compliance.
//!
//! Resizes and compresses images to fit within provider-specific size limits
//! before sending them as base64-encoded content blocks.

use base64::Engine;
use image::ImageFormat;
use std::io::Cursor;
use tracing::{debug, warn};

/// Maximum image size (in decoded bytes) per provider.
/// Returns the limit for the given provider prefix, defaulting to the
/// most restrictive (Anthropic's 5 MB) when unknown.
fn max_image_bytes(provider: &str) -> usize {
    match provider {
        "anthropic" => 5 * 1024 * 1024,
        "openai" => 20 * 1024 * 1024,
        "google" | "gemini" => 20 * 1024 * 1024,
        "mistral" => 10 * 1024 * 1024,
        _ => 5 * 1024 * 1024, // safe default
    }
}

/// Extract the provider prefix from a model spec like `"anthropic/claude-sonnet-4-20250514"`.
/// Returns `""` if there is no slash (which will map to the safe default).
pub fn provider_from_model(model: &str) -> &str {
    model.split_once('/').map_or("", |(p, _)| p)
}

/// Ensure the base64-encoded image fits within the provider's size limit.
///
/// If the decoded image is already small enough, returns the original
/// `(data, media_type)` unchanged.  Otherwise it progressively reduces
/// JPEG quality and resolution until the image fits.
///
/// # Arguments
/// * `base64_data` – the original base64-encoded image
/// * `media_type`  – MIME type, e.g. `"image/png"`, `"image/jpeg"`
/// * `model`       – full model spec (used to determine provider limit)
///
/// # Returns
/// `(base64_data, media_type)` — possibly re-encoded as `"image/jpeg"`.
pub fn fit_image_to_limit(
    base64_data: &str,
    media_type: &str,
    model: &str,
) -> (String, String) {
    let provider = provider_from_model(model);
    let limit = max_image_bytes(provider);

    // Fast path: check raw decoded size first
    let decoded_len = base64_data.len() * 3 / 4; // approximate
    if decoded_len <= limit {
        return (base64_data.to_owned(), media_type.to_owned());
    }

    // Decode the base64 data
    let raw = match base64::engine::general_purpose::STANDARD.decode(base64_data) {
        Ok(v) => v,
        Err(e) => {
            warn!("Failed to decode base64 image for resizing: {e}");
            return (base64_data.to_owned(), media_type.to_owned());
        }
    };

    // If decoded bytes are actually under limit, keep as-is
    if raw.len() <= limit {
        return (base64_data.to_owned(), media_type.to_owned());
    }

    debug!(
        original_bytes = raw.len(),
        limit,
        provider,
        "Image exceeds provider limit, resizing"
    );

    // Load with the `image` crate
    let img = match image::load_from_memory(&raw) {
        Ok(i) => i,
        Err(e) => {
            warn!("Failed to load image for resizing: {e}");
            return (base64_data.to_owned(), media_type.to_owned());
        }
    };

    // Strategy: try decreasing quality, then scale down
    let mut quality = 90u8;
    let mut scale: f32 = 1.0;
    let max_attempts = 8;

    for attempt in 0..max_attempts {
        let resized = if (scale - 1.0).abs() < f32::EPSILON {
            img.clone()
        } else {
            let new_w = ((img.width() as f32) * scale) as u32;
            let new_h = ((img.height() as f32) * scale) as u32;
            img.resize(
                new_w.max(1),
                new_h.max(1),
                image::imageops::FilterType::Lanczos3,
            )
        };

        let mut buf = Cursor::new(Vec::new());
        if resized
            .write_to(&mut buf, ImageFormat::Jpeg)
            .is_err()
        {
            // If JPEG encoding fails, try with lower quality via the encoder directly
            let mut buf2 = Vec::new();
            let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf2, quality);
            if resized.write_with_encoder(encoder).is_err() {
                warn!("JPEG encoding failed on attempt {attempt}");
                break;
            }
            if buf2.len() <= limit {
                let encoded = base64::engine::general_purpose::STANDARD.encode(&buf2);
                debug!(
                    final_bytes = buf2.len(),
                    quality,
                    scale,
                    attempts = attempt + 1,
                    "Image resized successfully"
                );
                return (encoded, "image/jpeg".to_owned());
            }
        } else {
            let bytes = buf.into_inner();
            if bytes.len() <= limit {
                let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
                debug!(
                    final_bytes = bytes.len(),
                    quality,
                    scale,
                    attempts = attempt + 1,
                    "Image resized successfully"
                );
                return (encoded, "image/jpeg".to_owned());
            }
        }

        // Reduce quality first, then start scaling down
        if quality > 60 {
            quality -= 10;
        } else {
            scale *= 0.75;
            quality = 85; // reset quality when we scale down
        }
    }

    warn!("Could not fit image under {limit} bytes after {max_attempts} attempts, sending as-is");
    (base64_data.to_owned(), media_type.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_from_model() {
        assert_eq!(provider_from_model("anthropic/claude-sonnet-4-20250514"), "anthropic");
        assert_eq!(provider_from_model("openai/gpt-4o"), "openai");
        assert_eq!(provider_from_model("claude-sonnet-4-20250514"), "");
    }

    #[test]
    fn test_small_image_passthrough() {
        // A tiny 1x1 white JPEG in base64
        let tiny = base64::engine::general_purpose::STANDARD.encode(&[
            0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46,
        ]);
        let (data, mt) = fit_image_to_limit(&tiny, "image/jpeg", "anthropic/claude-sonnet-4-20250514");
        assert_eq!(data, tiny);
        assert_eq!(mt, "image/jpeg");
    }
}
