//! Pure, allocation-free content-format sniffing by leading magic bytes.
//!
//! The content store is byte-opaque; nothing here changes that. This is a
//! standalone classifier used at the *boundary* where stored bytes meet a
//! modality-aware consumer — specifically the context assembler, which routes a
//! Mote's image-typed parents to the multi-modal inference path instead of
//! rendering their bytes as text. It lives in `kx-content` (not `kx-dataset`)
//! because byte inspection is a content concern and both the assembler and the
//! inference backend already depend on this crate — so no new dependency edge
//! is created, and the classifier has exactly one home.
//!
//! Deterministic and total: a fixed input always yields the same `Option`,
//! with no I/O and no allocation. It is intentionally conservative — it
//! recognizes only the handful of raster formats the vendored `stb_image`
//! decoder (compiled into `libmtmd`) can actually decode. Anything else
//! (text, JSON, audio, unknown) returns `None` and is treated as non-image by
//! the caller, which fails closed.

/// A raster image container recognized by [`sniff_image_format`].
///
/// Limited to the formats `stb_image` decodes; this is a *classification*, not
/// a decode (the heavy decode happens in `kx-llamacpp::mtmd`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ImageFormat {
    /// PNG — magic `89 50 4E 47 0D 0A 1A 0A`.
    Png,
    /// JPEG — magic `FF D8 FF`.
    Jpeg,
    /// GIF — magic `47 49 46 38` (`GIF8`).
    Gif,
    /// WebP — RIFF container with a `WEBP` form type at bytes 8..12.
    WebP,
    /// BMP — magic `42 4D` (`BM`).
    Bmp,
}

impl ImageFormat {
    /// A stable lowercase label (diagnostics / logging only).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            ImageFormat::Png => "png",
            ImageFormat::Jpeg => "jpeg",
            ImageFormat::Gif => "gif",
            ImageFormat::WebP => "webp",
            ImageFormat::Bmp => "bmp",
        }
    }
}

/// Classify `bytes` by leading magic bytes, or `None` if they are not a
/// recognized raster image.
///
/// Conservative by construction: requires the full signature to be present, so
/// a truncated header (fewer bytes than the magic) is `None`, and a RIFF
/// container that is not specifically `WEBP` (e.g. WAV/AVI) is `None`.
#[must_use]
pub fn sniff_image_format(bytes: &[u8]) -> Option<ImageFormat> {
    // PNG: 8-byte signature.
    const PNG: &[u8] = &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    if bytes.starts_with(PNG) {
        return Some(ImageFormat::Png);
    }
    // JPEG: starts with FF D8 FF (SOI + first marker).
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some(ImageFormat::Jpeg);
    }
    // GIF: "GIF8" (covers both 87a and 89a).
    if bytes.starts_with(b"GIF8") {
        return Some(ImageFormat::Gif);
    }
    // BMP: "BM".
    if bytes.starts_with(b"BM") {
        return Some(ImageFormat::Bmp);
    }
    // WebP: RIFF container — "RIFF" at 0..4 AND "WEBP" at 8..12. The middle 4
    // bytes are the little-endian chunk size (ignored). Guards against
    // false-positives on other RIFF formats (WAV/AVI).
    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return Some(ImageFormat::WebP);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_each_format() {
        assert_eq!(
            sniff_image_format(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00]),
            Some(ImageFormat::Png)
        );
        assert_eq!(
            sniff_image_format(&[0xFF, 0xD8, 0xFF, 0xE0, 0x00]),
            Some(ImageFormat::Jpeg)
        );
        assert_eq!(sniff_image_format(b"GIF89a..."), Some(ImageFormat::Gif));
        assert_eq!(sniff_image_format(b"GIF87a..."), Some(ImageFormat::Gif));
        assert_eq!(sniff_image_format(b"BM\x00\x00"), Some(ImageFormat::Bmp));
        // RIFF....WEBP
        let mut webp = Vec::from(*b"RIFF");
        webp.extend_from_slice(&[0x10, 0x00, 0x00, 0x00]); // chunk size
        webp.extend_from_slice(b"WEBPVP8 ");
        assert_eq!(sniff_image_format(&webp), Some(ImageFormat::WebP));
    }

    #[test]
    fn rejects_non_images() {
        assert_eq!(sniff_image_format(&[]), None);
        assert_eq!(sniff_image_format(b"hello, world"), None);
        assert_eq!(sniff_image_format(b"{\"json\": true}"), None);
        // Plausible UTF-8 model output must never be mistaken for an image.
        assert_eq!(sniff_image_format(b"The chart shows a rising trend."), None);
    }

    #[test]
    fn truncated_headers_are_none() {
        // 3 of PNG's 8 magic bytes.
        assert_eq!(sniff_image_format(&[0x89, 0x50, 0x4E]), None);
        // "FF D8" without the third JPEG byte.
        assert_eq!(sniff_image_format(&[0xFF, 0xD8]), None);
        // "GIF" without the version digit.
        assert_eq!(sniff_image_format(b"GIF"), None);
        // single 'B' of BMP.
        assert_eq!(sniff_image_format(b"B"), None);
    }

    #[test]
    fn riff_that_is_not_webp_is_rejected() {
        // WAV is RIFF....WAVE — must NOT be classified as an image.
        let mut wav = Vec::from(*b"RIFF");
        wav.extend_from_slice(&[0x24, 0x00, 0x00, 0x00]);
        wav.extend_from_slice(b"WAVEfmt ");
        assert_eq!(sniff_image_format(&wav), None);
        // RIFF header shorter than 12 bytes → None (no room for the form type).
        assert_eq!(sniff_image_format(b"RIFF\x00\x00\x00\x00"), None);
    }
}
