//! Placing a picture on a page: a photo of an ID, a logo, a scanned stamp.
//!
//! JPEG goes into the file as it is (the PDF filter `DCTDecode` *is* JPEG), so a
//! phone photo costs no decoding and no memory; its EXIF orientation, which
//! readers of PDF ignore, is applied in the placement instead. PNG is decoded
//! to raw samples, with the alpha channel split off as a soft mask.

use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Object, ObjectId, Stream};

use crate::error::{Error, Result};
use crate::text::multiply;
use crate::Document;

/// Largest picture accepted, in pixels: a 100 MB bitmap on a phone is a crash.
const MAX_PIXELS: u64 = 64_000_000;

#[derive(Debug, Clone)]
pub struct NewImage<'a> {
    /// A JPEG or PNG file.
    pub data: &'a [u8],
    /// Top-left corner in the page as a reader shows it (origin top-left, y down).
    pub x: f32,
    pub y: f32,
    pub width: f32,
    /// `None` keeps the picture's proportions.
    pub height: Option<f32>,
}

impl<'a> NewImage<'a> {
    pub fn new(data: &'a [u8], x: f32, y: f32, width: f32) -> Self {
        NewImage { data, x, y, width, height: None }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AddedImage {
    /// Box the picture occupies, as a reader shows the page.
    pub display: [f32; 4],
    /// Pixel size as the picture is meant to be seen (orientation applied).
    pub pixels: (u32, u32),
}

/// What the file says about a picture, and the stream to embed.
struct Picture {
    /// Width and height of the stored pixels.
    stored: (u32, u32),
    /// EXIF orientation, 1..=8.
    orientation: u8,
    stream: Stream,
    mask: Option<Stream>,
}

impl Document {
    /// Draw a JPEG or PNG on a page, scaled to the box given.
    pub fn add_image(&mut self, page: usize, new: &NewImage) -> Result<AddedImage> {
        self.transaction(|d| d.put_image(page, new))
    }

    fn put_image(&mut self, page: usize, new: &NewImage) -> Result<AddedImage> {
        if !(new.width > 0.0) || new.height.is_some_and(|h| !(h > 0.0)) {
            return Err(Error::Rejected("el tamaño de la imagen debe ser mayor que cero".into()));
        }
        let id = self.page_id(page)?;
        let picture = decode(new.data)?;
        let (sw, sh) = picture.stored;
        let swapped = picture.orientation >= 5;
        let shown = if swapped { (sh, sw) } else { (sw, sh) };
        let height = new.height.unwrap_or(new.width * shown.1 as f32 / shown.0 as f32);

        // The unit square the image occupies, turned as its orientation says,
        // then stretched over the box, then taken from the page as shown back
        // to the page's own space.
        let (su, sv, s0, tu, tv, t0) = match picture.orientation {
            2 => (-1.0, 0.0, 1.0, 0.0, 1.0, 0.0),
            3 => (-1.0, 0.0, 1.0, 0.0, -1.0, 1.0),
            4 => (1.0, 0.0, 0.0, 0.0, -1.0, 1.0),
            5 => (0.0, -1.0, 1.0, -1.0, 0.0, 1.0),
            6 => (0.0, 1.0, 0.0, -1.0, 0.0, 1.0),
            7 => (0.0, 1.0, 0.0, 1.0, 0.0, 0.0),
            8 => (0.0, -1.0, 1.0, 1.0, 0.0, 0.0),
            _ => (1.0, 0.0, 0.0, 0.0, 1.0, 0.0),
        };
        let (w, h) = (new.width, height);
        let to_display = [w * su, -h * tu, w * sv, -h * tv, new.x + w * s0, new.y + h - h * t0];
        let inverse = invert(self.display_matrix(page)?)
            .ok_or_else(|| Error::Unsupported("la página tiene una transformación no invertible".into()))?;
        let cm = multiply(to_display, inverse);

        let mut image = picture.stream;
        if let Some(mask) = picture.mask {
            let mask_id = self.pdf.add_object(mask);
            image.dict.set("SMask", Object::Reference(mask_id));
        }
        let image_id = self.pdf.add_object(image);
        let name = self.register_xobject(id, image_id)?;
        let ops = vec![
            Operation::new("q", vec![]),
            Operation::new("cm", cm.iter().map(|v| Object::Real(*v)).collect()),
            Operation::new("Do", vec![Object::Name(name.into_bytes())]),
            Operation::new("Q", vec![]),
        ];
        self.append_to_page(id, &Content { operations: ops })?;
        Ok(AddedImage { display: [new.x, new.y, new.x + w, new.y + h], pixels: shown })
    }

    /// Make an XObject usable by name in one page's content; returns the name.
    fn register_xobject(&mut self, page: ObjectId, object: ObjectId) -> Result<String> {
        let mut resources = self.resources(page);
        let mut xobjects = resources
            .get(b"XObject")
            .ok()
            .map(|o| crate::font::resolve(&self.pdf, o))
            .and_then(|o| o.as_dict().ok().cloned())
            .unwrap_or_default();
        let name = (1..)
            .map(|n| format!("PdfCoreImg{n}"))
            .find(|n| !xobjects.has(n.as_bytes()))
            .expect("some number is free");
        xobjects.set(name.as_str(), Object::Reference(object));
        resources.set("XObject", Object::Dictionary(xobjects));
        self.pdf.get_dictionary_mut(page)?.set("Resources", Object::Dictionary(resources));
        Ok(name)
    }
}

fn invert(m: [f32; 6]) -> Option<[f32; 6]> {
    let det = m[0] * m[3] - m[1] * m[2];
    if det.abs() < 1e-9 {
        return None;
    }
    let (a, b, c, d) = (m[3] / det, -m[1] / det, -m[2] / det, m[0] / det);
    Some([a, b, c, d, -(m[4] * a + m[5] * c), -(m[4] * b + m[5] * d)])
}

fn decode(data: &[u8]) -> Result<Picture> {
    if data.starts_with(&[0xFF, 0xD8]) {
        jpeg(data)
    } else if data.starts_with(&[0x89, b'P', b'N', b'G']) {
        png(data)
    } else {
        Err(Error::Unsupported("formato de imagen no soportado: use JPEG o PNG".into()))
    }
}

fn too_big(w: u32, h: u32) -> Result<()> {
    if w == 0 || h == 0 {
        return Err(Error::Rejected("la imagen está vacía".into()));
    }
    if w as u64 * h as u64 > MAX_PIXELS {
        return Err(Error::Rejected(format!("la imagen es demasiado grande ({w}×{h} píxeles)")));
    }
    Ok(())
}

fn image_dict(w: u32, h: u32, space: &str) -> Dictionary {
    let mut dict = Dictionary::new();
    dict.set("Type", Object::Name(b"XObject".to_vec()));
    dict.set("Subtype", Object::Name(b"Image".to_vec()));
    dict.set("Width", Object::Integer(w as i64));
    dict.set("Height", Object::Integer(h as i64));
    dict.set("ColorSpace", Object::Name(space.as_bytes().to_vec()));
    dict.set("BitsPerComponent", Object::Integer(8));
    dict
}

/// Read a JPEG's headers: size, components, an Adobe marker, EXIF orientation.
fn jpeg(data: &[u8]) -> Result<Picture> {
    let (mut at, mut size, mut components, mut adobe, mut orientation) = (2usize, None, 0u8, false, 1u8);
    while at + 4 <= data.len() {
        if data[at] != 0xFF {
            at += 1;
            continue;
        }
        let marker = data[at + 1];
        if marker == 0xFF || marker == 0x00 || (0xD0..=0xD7).contains(&marker) || marker == 0x01 {
            at += if marker == 0xFF { 1 } else { 2 };
            continue;
        }
        let length = u16::from_be_bytes([data[at + 2], data[at + 3]]) as usize;
        let body = data.get(at + 4..(at + 2 + length).min(data.len())).unwrap_or(&[]);
        match marker {
            // Start of frame (baseline, extended, progressive).
            0xC0 | 0xC1 | 0xC2 if body.len() >= 6 => {
                let h = u16::from_be_bytes([body[1], body[2]]) as u32;
                let w = u16::from_be_bytes([body[3], body[4]]) as u32;
                size = Some((w, h));
                components = body[5];
                if body[0] != 8 {
                    return Err(Error::Unsupported("solo se admiten JPEG de 8 bits".into()));
                }
                break;
            }
            0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF => {
                return Err(Error::Unsupported("este tipo de JPEG (aritmético o sin pérdida) no está soportado".into()));
            }
            0xEE if body.starts_with(b"Adobe") => adobe = true,
            0xE1 if body.starts_with(b"Exif\0\0") => orientation = exif_orientation(&body[6..]).unwrap_or(1),
            _ => {}
        }
        at += 2 + length;
    }
    let (w, h) = size.ok_or_else(|| Error::Unsupported("no se pudo leer el tamaño del JPEG".into()))?;
    too_big(w, h)?;
    let space = match components {
        1 => "DeviceGray",
        3 => "DeviceRGB",
        4 => "DeviceCMYK",
        n => return Err(Error::Unsupported(format!("JPEG con {n} componentes no soportado"))),
    };
    let mut dict = image_dict(w, h, space);
    dict.set("Filter", Object::Name(b"DCTDecode".to_vec()));
    if components == 4 && adobe {
        // Adobe writes CMYK inverted.
        dict.set("Decode", Object::Array([1, 0, 1, 0, 1, 0, 1, 0].iter().map(|v| Object::Integer(*v)).collect()));
    }
    dict.set("Interpolate", Object::Boolean(true));
    let mut stream = Stream::new(dict, data.to_vec());
    stream.allows_compression = false;
    Ok(Picture { stored: (w, h), orientation: orientation.clamp(1, 8), stream, mask: None })
}

/// The orientation tag (0x0112) of a TIFF block as EXIF stores it.
fn exif_orientation(tiff: &[u8]) -> Option<u8> {
    let little = match tiff.get(..2)? {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let u16_at = |at: usize| -> Option<u16> {
        let b = tiff.get(at..at + 2)?;
        Some(if little { u16::from_le_bytes([b[0], b[1]]) } else { u16::from_be_bytes([b[0], b[1]]) })
    };
    let u32_at = |at: usize| -> Option<u32> {
        let b = tiff.get(at..at + 4)?;
        Some(if little { u32::from_le_bytes([b[0], b[1], b[2], b[3]]) } else { u32::from_be_bytes([b[0], b[1], b[2], b[3]]) })
    };
    let first = u32_at(4)? as usize;
    let count = u16_at(first)? as usize;
    for entry in 0..count.min(64) {
        let at = first + 2 + entry * 12;
        if u16_at(at)? == 0x0112 {
            return Some(u16_at(at + 8)? as u8);
        }
    }
    None
}

fn png(data: &[u8]) -> Result<Picture> {
    let mut decoder = png::Decoder::new(data);
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| Error::Unsupported(format!("PNG no válido: {e}")))?;
    let (w, h) = (reader.info().width, reader.info().height);
    too_big(w, h)?;
    let mut buffer = vec![0u8; reader.output_buffer_size()];
    let frame = reader.next_frame(&mut buffer).map_err(|e| Error::Unsupported(format!("PNG no válido: {e}")))?;
    buffer.truncate(frame.buffer_size());

    let (channels, color, alpha_at) = match frame.color_type {
        png::ColorType::Grayscale => (1, "DeviceGray", None),
        png::ColorType::GrayscaleAlpha => (2, "DeviceGray", Some(1)),
        png::ColorType::Rgb => (3, "DeviceRGB", None),
        png::ColorType::Rgba => (4, "DeviceRGB", Some(3)),
        png::ColorType::Indexed => return Err(Error::Unsupported("PNG indexado sin expandir".into())),
    };
    let pixels = (w as usize) * (h as usize);
    let (mut colour, mut alpha) = (Vec::with_capacity(pixels * (channels - alpha_at.is_some() as usize)), Vec::new());
    match alpha_at {
        None => colour = buffer,
        Some(a) => {
            alpha.reserve(pixels);
            for pixel in buffer.chunks_exact(channels) {
                colour.extend_from_slice(&pixel[..a]);
                alpha.push(pixel[a]);
            }
        }
    }
    let mut stream = Stream::new(image_dict(w, h, color), colour);
    let _ = stream.compress();
    let mask = (!alpha.is_empty() && alpha.iter().any(|a| *a != 255)).then(|| {
        let mut dict = image_dict(w, h, "DeviceGray");
        dict.set("Interpolate", Object::Boolean(true));
        let mut mask = Stream::new(dict, alpha);
        let _ = mask.compress();
        mask
    });
    Ok(Picture { stored: (w, h), orientation: 1, stream, mask })
}
