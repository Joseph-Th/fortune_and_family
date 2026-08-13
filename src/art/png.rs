//! A dependency-free encoder for 8-bit indexed PNG images.
//!
//! The encoder emits uncompressed deflate blocks. Sprite frames are small, review sheets are
//! transient, and avoiding a compression dependency keeps the renderer auditable and
//! deterministic.

use super::canvas::Canvas;
use super::color::Palette;

const SIGNATURE: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];
const MAX_STORED_BLOCK: usize = 65_535;
const BASE64_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Encodes an indexed canvas and its palette as PNG bytes.
///
/// Palette index zero is written as fully transparent; every other entry is opaque.
///
/// # Panics
///
/// Panics when the canvas references a palette index that is not present in `palette`, or when
/// an encoded chunk is too large for PNG's `u32` chunk length.
#[must_use]
pub fn encode_indexed_png(canvas: &Canvas, palette: &Palette) -> Vec<u8> {
    assert!(
        canvas
            .pixels()
            .iter()
            .all(|index| usize::from(*index) < palette.len()),
        "canvas palette indexes must exist in the supplied palette"
    );
    let mut bytes = Vec::from(SIGNATURE);

    let mut header = Vec::with_capacity(13);
    header.extend_from_slice(&canvas.width().to_be_bytes());
    header.extend_from_slice(&canvas.height().to_be_bytes());
    header.extend_from_slice(&[8, 3, 0, 0, 0]);
    write_chunk(&mut bytes, *b"IHDR", &header);

    let mut plte = Vec::with_capacity(palette.len() * 3);
    for color in palette.colors() {
        plte.extend_from_slice(&[color.red, color.green, color.blue]);
    }
    write_chunk(&mut bytes, *b"PLTE", &plte);
    write_chunk(&mut bytes, *b"tRNS", &[0]);

    write_chunk(
        &mut bytes,
        *b"IDAT",
        &deflate_stored(&raw_scanlines(canvas)),
    );
    write_chunk(&mut bytes, *b"IEND", &[]);
    bytes
}

/// Encodes a canvas as a `data:` URI suitable for embedding in the review harness.
#[must_use]
pub fn encode_png_data_uri(canvas: &Canvas, palette: &Palette) -> String {
    format!(
        "data:image/png;base64,{}",
        encode_base64(&encode_indexed_png(canvas, palette))
    )
}

fn raw_scanlines(canvas: &Canvas) -> Vec<u8> {
    let width = usize::try_from(canvas.width()).expect("canvas width must fit usize");
    let height = usize::try_from(canvas.height()).expect("canvas height must fit usize");
    let stride = width
        .checked_add(1)
        .expect("scanline stride must fit usize");
    let capacity = height
        .checked_mul(stride)
        .expect("raw scanline length must fit usize");
    let mut raw = Vec::with_capacity(capacity);
    for row in 0..height {
        raw.push(0);
        let start = row
            .checked_mul(width)
            .expect("scanline offset must fit usize");
        let end = start
            .checked_add(width)
            .expect("scanline end must fit usize");
        raw.extend_from_slice(&canvas.pixels()[start..end]);
    }
    raw
}

fn deflate_stored(data: &[u8]) -> Vec<u8> {
    let mut stream = vec![0x78, 0x01];
    if data.is_empty() {
        stream.extend_from_slice(&[1, 0, 0, 0xFF, 0xFF]);
    }
    let block_count = data.len().div_ceil(MAX_STORED_BLOCK);
    for (index, block) in data.chunks(MAX_STORED_BLOCK).enumerate() {
        let is_final = index == block_count - 1;
        stream.push(u8::from(is_final));
        let length = u16::try_from(block.len()).expect("stored block length must fit u16");
        stream.extend_from_slice(&length.to_le_bytes());
        stream.extend_from_slice(&(!length).to_le_bytes());
        stream.extend_from_slice(block);
    }
    stream.extend_from_slice(&adler32(data).to_be_bytes());
    stream
}

fn write_chunk(bytes: &mut Vec<u8>, kind: [u8; 4], payload: &[u8]) {
    let length = u32::try_from(payload.len()).expect("chunk length must fit u32");
    bytes.extend_from_slice(&length.to_be_bytes());
    bytes.extend_from_slice(&kind);
    bytes.extend_from_slice(payload);
    let checked_capacity = payload
        .len()
        .checked_add(4)
        .expect("CRC input length must fit usize");
    let mut checked = Vec::with_capacity(checked_capacity);
    checked.extend_from_slice(&kind);
    checked.extend_from_slice(payload);
    bytes.extend_from_slice(&crc32(&checked).to_be_bytes());
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFF_u32;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = 0u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let mut low = 1u32;
    let mut high = 0u32;
    for byte in data {
        low = (low + u32::from(*byte)) % 65_521;
        high = (high + low) % 65_521;
    }
    (high << 16) | low
}

/// Encodes bytes as standard padded base64.
///
/// # Panics
///
/// Panics when the encoded output length cannot fit `usize`.
#[must_use]
pub fn encode_base64(data: &[u8]) -> String {
    let capacity = data
        .len()
        .div_ceil(3)
        .checked_mul(4)
        .expect("base64 output length must fit usize");
    let mut encoded = String::with_capacity(capacity);
    for chunk in data.chunks(3) {
        let mut buffer = [0u8; 3];
        buffer[..chunk.len()].copy_from_slice(chunk);
        let packed =
            (u32::from(buffer[0]) << 16) | (u32::from(buffer[1]) << 8) | u32::from(buffer[2]);
        let symbols = [
            BASE64_ALPHABET[usize::try_from((packed >> 18) & 63).expect("base64 index must fit")],
            BASE64_ALPHABET[usize::try_from((packed >> 12) & 63).expect("base64 index must fit")],
            BASE64_ALPHABET[usize::try_from((packed >> 6) & 63).expect("base64 index must fit")],
            BASE64_ALPHABET[usize::try_from(packed & 63).expect("base64 index must fit")],
        ];
        for (index, symbol) in symbols.into_iter().enumerate() {
            if index <= chunk.len() {
                encoded.push(char::from(symbol));
            } else {
                encoded.push('=');
            }
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::super::color::{Ramp, Rgb8, ShadeProfile};
    use super::*;

    fn sample() -> (Canvas, Palette) {
        let mut palette = Palette::new();
        let ramp = palette.insert_ramp(&Ramp::build(
            Rgb8::new(150, 96, 72),
            ShadeProfile::material(),
        ));
        let mut canvas = Canvas::new(6, 4);
        for step in 0..4 {
            canvas.set(step, step, ramp.index(step));
        }
        (canvas, palette)
    }

    #[test]
    fn encoded_images_begin_with_the_png_signature() {
        let (canvas, palette) = sample();

        let bytes = encode_indexed_png(&canvas, &palette);

        assert_eq!(&bytes[..8], &SIGNATURE);
    }

    #[test]
    fn encoded_images_contain_every_required_chunk_in_order() {
        let (canvas, palette) = sample();
        let bytes = encode_indexed_png(&canvas, &palette);

        let position = |kind: &[u8]| {
            bytes
                .windows(4)
                .position(|window| window == kind)
                .expect("chunk must be present")
        };

        assert!(position(b"IHDR") < position(b"PLTE"));
        assert!(position(b"PLTE") < position(b"tRNS"));
        assert!(position(b"tRNS") < position(b"IDAT"));
        assert!(position(b"IDAT") < position(b"IEND"));
    }

    #[test]
    fn encoding_is_deterministic() {
        let (canvas, palette) = sample();

        assert_eq!(
            encode_indexed_png(&canvas, &palette),
            encode_indexed_png(&canvas, &palette)
        );
    }

    #[test]
    #[should_panic(expected = "canvas palette indexes must exist")]
    fn encoding_rejects_indexes_missing_from_the_palette() {
        let mut canvas = Canvas::new(2, 2);
        canvas.set(0, 0, 7);
        let palette = Palette::new();

        let _ = encode_indexed_png(&canvas, &palette);
    }

    #[test]
    fn adler_and_crc_match_known_values() {
        assert_eq!(adler32(b"abc"), 0x024D_0127);
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn base64_matches_reference_vectors() {
        assert_eq!(encode_base64(b""), "");
        assert_eq!(encode_base64(b"f"), "Zg==");
        assert_eq!(encode_base64(b"fo"), "Zm8=");
        assert_eq!(encode_base64(b"foo"), "Zm9v");
        assert_eq!(encode_base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn large_images_split_into_multiple_stored_blocks() {
        let mut palette = Palette::new();
        palette.insert_color(Rgb8::WHITE);
        let canvas = Canvas::new(400, 400);

        let bytes = encode_indexed_png(&canvas, &palette);

        assert!(bytes.len() > MAX_STORED_BLOCK);
    }

    #[test]
    fn data_uris_carry_a_png_media_type() {
        let (canvas, palette) = sample();

        assert!(encode_png_data_uri(&canvas, &palette).starts_with("data:image/png;base64,"));
    }
}
