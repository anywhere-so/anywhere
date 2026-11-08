//! Only decoded pixels cross from an app's icon into the platform's origin.
use std::io::Cursor;

pub const MAX_BYTES: usize = 1024 * 1024;
pub const MAX_DIMENSION: u32 = 1024;

#[derive(Debug, thiserror::Error)]
#[error(
    "invalid app icon: {0}; use a non-animated PNG at most 1 MiB and 1024×1024 pixels (512×512 recommended)"
)]
pub struct Error(pub String);

impl Error {
    pub fn code(&self) -> &'static str {
        "app_icon_invalid"
    }
}

/// Decode with bounded memory, then write only pixels, without metadata or trailing bytes.
pub fn sanitize(bytes: &[u8]) -> Result<Vec<u8>, Error> {
    if bytes.len() > MAX_BYTES {
        return Err(Error("the file exceeds 1 MiB".to_owned()));
    }
    let mut decoder = png::Decoder::new_with_limits(
        Cursor::new(bytes),
        png::Limits {
            bytes: 16 * 1024 * 1024,
        },
    );
    decoder.set_ignore_text_chunk(true);
    decoder.set_ignore_iccp_chunk(true);
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| Error(e.to_string()))?;
    let info = reader.info();
    if info.width == 0
        || info.height == 0
        || info.width > MAX_DIMENSION
        || info.height > MAX_DIMENSION
        || info.animation_control.is_some()
    {
        return Err(Error(
            "dimensions are too large, empty, or the image is animated".to_owned(),
        ));
    }
    let len = reader
        .output_buffer_size()
        .ok_or_else(|| Error("image dimensions overflow".to_owned()))?;
    let mut pixels = vec![0; len];
    let frame = reader
        .next_frame(&mut pixels)
        .map_err(|e| Error(e.to_string()))?;
    reader.finish().map_err(|e| Error(e.to_string()))?;
    let mut output = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut output, frame.width, frame.height);
        encoder.set_color(frame.color_type);
        encoder.set_depth(frame.bit_depth);
        let mut writer = encoder.write_header().map_err(|e| Error(e.to_string()))?;
        writer
            .write_image_data(&pixels[..frame.buffer_size()])
            .map_err(|e| Error(e.to_string()))?;
        writer.finish().map_err(|e| Error(e.to_string()))?;
    }
    if output.len() > MAX_BYTES {
        return Err(Error(
            "the re-encoded PNG exceeds 1 MiB; use smaller dimensions".to_owned(),
        ));
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_transparent_pixels_but_strips_metadata_and_trailing_markup() {
        let clean = sanitize(include_bytes!("../tests/fixtures/icon.png")).unwrap();
        assert!(!clean.windows(6).any(|part| part == b"script"));
        assert!(!clean.windows(4).any(|part| part == b"html"));
        let mut reader = png::Decoder::new(Cursor::new(&clean)).read_info().unwrap();
        assert_eq!((reader.info().width, reader.info().height), (2, 2));
        assert!(reader.info().uncompressed_latin1_text.is_empty());
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        reader.next_frame(&mut pixels).unwrap();
        assert_eq!(pixels, [0x12, 0x34, 0x56, 0x80].repeat(4));
    }

    #[test]
    fn refuses_bad_truncated_oversized_and_over_dimension_images() {
        let good = include_bytes!("../tests/fixtures/icon.png");
        for bad in [
            b"<svg/>".as_slice(),
            &good[..50],
            include_bytes!("../tests/fixtures/too-wide.png"),
            &vec![0; MAX_BYTES + 1],
        ] {
            assert_eq!(sanitize(bad).unwrap_err().code(), "app_icon_invalid");
        }
    }
}
