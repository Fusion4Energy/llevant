//! PNG files of 8-bit RGB images (rows from the top).

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

/// Write a `w × h` RGB image as a PNG file.
pub fn write_png(path: &Path, w: usize, h: usize, rgb: &[u8]) -> Result<(), String> {
    let f = File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    encode_png(BufWriter::new(f), w, h, rgb)
}

/// A `w × h` RGB image encoded as PNG.
pub fn png_bytes(w: usize, h: usize, rgb: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    encode_png(&mut out, w, h, rgb)?;
    Ok(out)
}

fn encode_png<W: Write>(out: W, w: usize, h: usize, rgb: &[u8]) -> Result<(), String> {
    let mut enc = ::png::Encoder::new(out, w as u32, h as u32);
    enc.set_color(::png::ColorType::Rgb);
    enc.set_depth(::png::BitDepth::Eight);
    let mut wr = enc.write_header().map_err(|e| e.to_string())?;
    wr.write_image_data(rgb).map_err(|e| e.to_string())
}
