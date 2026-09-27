//! The image handed to a viewer, as PNG bytes, 8-bit RGB pixels, or both.
//!
//! The kitty and iTerm2 protocols and the browser page send the PNG; sixel and the window need
//! pixels. Whichever form is missing is produced on first use and kept.

use std::fmt;
use std::io::Cursor;
use std::sync::OnceLock;

/// A rendered chart.
pub struct Image {
    width: u32,
    height: u32,
    png: OnceLock<Vec<u8>>,
    rgb: OnceLock<Vec<u8>>,
}

impl fmt::Debug for Image {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Image")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("png_bytes", &self.png.get().map(Vec::len))
            .field("has_rgb", &self.rgb.get().is_some())
            .finish()
    }
}

/// Why an [`Image`] could not be built, encoded or decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageError {
    /// Width or height is zero.
    Empty,
    /// The pixel buffer does not hold `width * height * 3` bytes.
    BufferSize { expected: usize, actual: usize },
    /// The PNG could not be decoded.
    Decode(String),
    /// The pixels could not be encoded as PNG.
    Encode(String),
}

impl fmt::Display for ImageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ImageError::Empty => f.write_str("image has zero width or height"),
            ImageError::BufferSize { expected, actual } => write!(
                f,
                "RGB buffer holds {actual} bytes, expected {expected} (width * height * 3)"
            ),
            ImageError::Decode(e) => write!(f, "cannot decode PNG: {e}"),
            ImageError::Encode(e) => write!(f, "cannot encode PNG: {e}"),
        }
    }
}

impl std::error::Error for ImageError {}

impl Image {
    /// An image from PNG bytes. Only the header is read now; pixels are decoded when a viewer needs them.
    pub fn from_png(png: Vec<u8>) -> Result<Image, ImageError> {
        let decoder = png::Decoder::new(Cursor::new(&png[..]));
        let reader = decoder
            .read_info()
            .map_err(|e| ImageError::Decode(e.to_string()))?;
        let info = reader.info();
        let (width, height) = (info.width, info.height);
        if width == 0 || height == 0 {
            return Err(ImageError::Empty);
        }
        Ok(Image {
            width,
            height,
            png: OnceLock::from(png),
            rgb: OnceLock::new(),
        })
    }

    /// An image from 8-bit RGB pixels, row-major without padding, as plotters' `BitMapBackend` draws them.
    /// The PNG is encoded when a viewer needs it.
    pub fn from_rgb(width: u32, height: u32, rgb: Vec<u8>) -> Result<Image, ImageError> {
        check_rgb(width, height, &rgb)?;
        Ok(Image {
            width,
            height,
            png: OnceLock::new(),
            rgb: OnceLock::from(rgb),
        })
    }

    /// An image whose PNG and pixels are both at hand. The two are trusted to show the same picture.
    pub fn from_png_and_rgb(
        png: Vec<u8>,
        width: u32,
        height: u32,
        rgb: Vec<u8>,
    ) -> Result<Image, ImageError> {
        check_rgb(width, height, &rgb)?;
        Ok(Image {
            width,
            height,
            png: OnceLock::from(png),
            rgb: OnceLock::from(rgb),
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// The PNG bytes, encoded from the pixels on first use.
    pub fn png(&self) -> Result<&[u8], ImageError> {
        if let Some(png) = self.png.get() {
            return Ok(png);
        }
        let rgb = self.rgb.get().expect("an image holds PNG or pixels");
        let png = encode_png(self.width, self.height, rgb)?;
        Ok(self.png.get_or_init(|| png))
    }

    /// The pixels as 8-bit RGB, decoded from the PNG on first use.
    /// Transparent pixels are composited over white, the chart background.
    pub fn rgb(&self) -> Result<&[u8], ImageError> {
        if let Some(rgb) = self.rgb.get() {
            return Ok(rgb);
        }
        let png = self.png.get().expect("an image holds PNG or pixels");
        let rgb = decode_png(png)?;
        check_rgb(self.width, self.height, &rgb)?;
        Ok(self.rgb.get_or_init(|| rgb))
    }
}

fn check_rgb(width: u32, height: u32, rgb: &[u8]) -> Result<(), ImageError> {
    if width == 0 || height == 0 {
        return Err(ImageError::Empty);
    }
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(3))
        .unwrap_or(usize::MAX);
    if rgb.len() != expected {
        return Err(ImageError::BufferSize {
            expected,
            actual: rgb.len(),
        });
    }
    Ok(())
}

fn encode_png(width: u32, height: u32, rgb: &[u8]) -> Result<Vec<u8>, ImageError> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let enc = |e: png::EncodingError| ImageError::Encode(e.to_string());
    let mut writer = encoder.write_header().map_err(enc)?;
    writer.write_image_data(rgb).map_err(enc)?;
    writer.finish().map_err(enc)?;
    Ok(out)
}

fn decode_png(png: &[u8]) -> Result<Vec<u8>, ImageError> {
    let dec = |e: png::DecodingError| ImageError::Decode(e.to_string());
    let mut decoder = png::Decoder::new(Cursor::new(png));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(dec)?;
    let size = reader
        .output_buffer_size()
        .ok_or_else(|| ImageError::Decode("image too large".into()))?;
    let mut buf = vec![0; size];
    let info = reader.next_frame(&mut buf).map_err(dec)?;
    buf.truncate(info.buffer_size());
    let over_white =
        |c: u8, a: u8| ((c as u16 * a as u16 + 255 * (255 - a as u16) + 127) / 255) as u8;
    let rgb = match info.color_type {
        png::ColorType::Rgb => buf,
        png::ColorType::Rgba => buf
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|p| {
                [
                    over_white(p[0], p[3]),
                    over_white(p[1], p[3]),
                    over_white(p[2], p[3]),
                ]
            })
            .collect(),
        png::ColorType::Grayscale => buf.iter().flat_map(|&g| [g, g, g]).collect(),
        png::ColorType::GrayscaleAlpha => buf
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|p| {
                let g = over_white(p[0], p[1]);
                [g, g, g]
            })
            .collect(),
        png::ColorType::Indexed => {
            return Err(ImageError::Decode("indexed colour was not expanded".into()));
        }
    };
    Ok(rgb)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient(w: u32, h: u32) -> Vec<u8> {
        (0..h)
            .flat_map(|y| (0..w).flat_map(move |x| [(x * 255 / w) as u8, (y * 255 / h) as u8, 128]))
            .collect()
    }

    #[test]
    fn rgb_round_trips_through_png() {
        let rgb = gradient(7, 5);
        let img = Image::from_rgb(7, 5, rgb.clone()).unwrap();
        let png = img.png().unwrap().to_vec();
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        let back = Image::from_png(png).unwrap();
        assert_eq!((back.width(), back.height()), (7, 5));
        assert_eq!(back.rgb().unwrap(), &rgb[..]);
    }

    #[test]
    fn rgba_is_composited_over_white() {
        let mut out = Vec::new();
        let mut enc = png::Encoder::new(&mut out, 2, 1);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().unwrap();
        w.write_image_data(&[255, 0, 0, 255, 0, 0, 0, 0]).unwrap();
        w.finish().unwrap();
        let img = Image::from_png(out).unwrap();
        assert_eq!(img.rgb().unwrap(), &[255, 0, 0, 255, 255, 255]);
    }

    #[test]
    fn wrong_buffer_size_is_rejected() {
        assert_eq!(
            Image::from_rgb(2, 2, vec![0; 11]).unwrap_err(),
            ImageError::BufferSize {
                expected: 12,
                actual: 11
            }
        );
        assert_eq!(
            Image::from_rgb(0, 2, vec![]).unwrap_err(),
            ImageError::Empty
        );
    }

    #[test]
    fn garbage_png_is_a_decode_error() {
        assert!(matches!(
            Image::from_png(b"not a png".to_vec()),
            Err(ImageError::Decode(_))
        ));
    }
}
