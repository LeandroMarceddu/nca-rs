//! Deterministic grayscale heightmap export.
//!
//! The image has one pixel per heightmap cell. The stock top is white (255)
//! and the lowest final cut is black (0); intermediate heights are linear.
//! Row 0 of the heightmap is row 0 of the image. The PNG carries no timestamp
//! or other ambient data.

use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

use nc_stock::Heightmap;

/// Writes the heightmap as an 8-bit grayscale PNG.
pub fn write_heightmap_png(heightmap: &Heightmap, path: &Path) -> Result<(), String> {
    let cols = heightmap.cols();
    let rows = heightmap.rows();
    if cols == 0 || rows == 0 {
        return Err("heightmap has no cells".to_string());
    }
    let cols_u32 = u32::try_from(cols).map_err(|_| "heightmap is too wide for PNG".to_string())?;
    let rows_u32 = u32::try_from(rows).map_err(|_| "heightmap is too tall for PNG".to_string())?;

    let top_z_mm = heightmap.top_z_mm();
    let mut lowest_z_mm = top_z_mm;
    for cell in heightmap.cells() {
        let height = f64::from(*cell);
        if height < lowest_z_mm {
            lowest_z_mm = height;
        }
    }

    let span_mm = top_z_mm - lowest_z_mm;
    let pixels: Vec<u8> = heightmap
        .cells()
        .iter()
        .map(|cell| {
            if span_mm <= 0.0 {
                return 255;
            }
            let height = f64::from(*cell);
            let fraction = ((height - lowest_z_mm) / span_mm).clamp(0.0, 1.0);
            let value = nc_core::math::round(fraction * 255.0).clamp(0.0, 255.0);
            value as u8
        })
        .collect();

    let file = File::create(path).map_err(|error| format!("cannot create image: {error}"))?;
    let writer = BufWriter::new(file);
    let mut encoder = png::Encoder::new(writer, cols_u32, rows_u32);
    encoder.set_color(png::ColorType::Grayscale);
    encoder.set_depth(png::BitDepth::Eight);
    let mut png_writer = encoder
        .write_header()
        .map_err(|error| format!("cannot write PNG header: {error}"))?;
    png_writer
        .write_image_data(&pixels)
        .map_err(|error| format!("cannot write PNG pixels: {error}"))?;
    png_writer
        .finish()
        .map_err(|error| format!("cannot finish PNG: {error}"))?;
    Ok(())
}
