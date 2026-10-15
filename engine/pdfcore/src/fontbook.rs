//! Fonts the app hands to the engine, and the list of fonts a document would like to have.
//!
//! A document that does not embed its fonts (or embeds only the letters it used) can only be
//! edited faithfully if the real font is at hand. The engine never fetches fonts and cannot
//! see the device's: [`Document::required_fonts`] says which ones are missing and where a free
//! lookalike can be had, and [`Document::register_font`] accepts one the person has the right to
//! use (the device's own Verdana, a downloaded Open Sans...). A registered font is used, at
//! similarity 100, whenever a letter of a font of that family has to be drawn with another font.
//!
//! Registered fonts live as long as the [`Document`]; nothing is written to the file until a
//! letter is drawn with one.

use ttf_parser::{name_id, Face, Permissions};

use crate::error::{Error, Result};
use crate::fallback::Choice;
use crate::font::FontClass;
use crate::text::fonts_of;
use crate::Document;

/// The most fonts that can be registered in one document, and the biggest one.
const MAX_FONTS: usize = 16;
const MAX_BYTES: usize = 20 * 1024 * 1024;

pub(crate) struct Registered {
    /// Family name as the font states it, lower case, letters and digits only.
    family: String,
    pub bold: bool,
    pub italic: bool,
    /// Name the font is embedded under (`Verdana-BoldItalic`).
    pub key: &'static str,
    pub data: &'static [u8],
}

/// What [`Document::register_font`] understood about a font.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredFont {
    pub family: String,
    pub bold: bool,
    pub italic: bool,
}

/// Where a free lookalike of a font can be had.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontSource {
    pub name: &'static str,
    pub license: &'static str,
    pub url: &'static str,
}

/// A font the document uses that is worth having to edit it faithfully.
#[derive(Debug, Clone, PartialEq)]
pub struct RequiredFont {
    pub base_font: String,
    pub class: FontClass,
    /// Pages that show text in it, and how many strings.
    pub pages: Vec<usize>,
    pub runs: usize,
    /// Editing needs the real font to stay exact: it is not embedded, or only its used letters are.
    pub needed: bool,
    /// A font of that family has been registered.
    pub registered: bool,
    /// The bundled font that stands in for it, and how close it is (0-100).
    pub bundled: String,
    pub similarity: u8,
    /// Where to get a free font that looks like it, when known.
    pub source: Option<FontSource>,
}

const SOURCES: &[(&[&str], FontSource)] = &[
    (&["calibri", "carlito"], FontSource { name: "Carlito (bundled)", license: "OFL-1.1", url: "https://github.com/googlefonts/carlito" }),
    (&["cambria", "caladea"], FontSource { name: "Caladea (bundled)", license: "OFL-1.1", url: "https://github.com/huertatipografica/Caladea" }),
    (&["arial", "helvetica", "arimo"], FontSource { name: "Liberation Sans (bundled)", license: "OFL-1.1", url: "https://github.com/liberationfonts/liberation-fonts" }),
    (&["times"], FontSource { name: "Liberation Serif (bundled)", license: "OFL-1.1", url: "https://github.com/liberationfonts/liberation-fonts" }),
    (&["courier", "consolas"], FontSource { name: "Liberation Mono (bundled)", license: "OFL-1.1", url: "https://github.com/liberationfonts/liberation-fonts" }),
    (&["georgia"], FontSource { name: "Gelasio", license: "OFL-1.1", url: "https://fonts.google.com/specimen/Gelasio" }),
    (&["verdana", "tahoma", "segoe", "trebuchet"], FontSource { name: "Open Sans", license: "OFL-1.1", url: "https://fonts.google.com/specimen/Open+Sans" }),
    (&["garamond", "palatino", "minion", "book", "century"], FontSource { name: "EB Garamond", license: "OFL-1.1", url: "https://fonts.google.com/specimen/EB+Garamond" }),
    (&["roboto"], FontSource { name: "Roboto", license: "Apache-2.0", url: "https://fonts.google.com/specimen/Roboto" }),
    (&["lato"], FontSource { name: "Lato", license: "OFL-1.1", url: "https://fonts.google.com/specimen/Lato" }),
];

fn normalized(name: &str) -> String {
    name.rsplit('+').next().unwrap_or(name).chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase()
}

fn name_of(face: &Face, ids: &[u16]) -> Option<String> {
    ids.iter().find_map(|id| face.names().into_iter().find(|n| n.name_id == *id && n.is_unicode()).and_then(|n| n.to_string()))
}

impl Document {
    /// Make a font the person may use available for the letters the document's own fonts lack.
    /// Only TrueType outlines can be embedded; a font whose licence forbids embedding
    /// (`fsType`) is refused with `rejected`.
    pub fn register_font(&mut self, bytes: &[u8]) -> Result<RegisteredFont> {
        if bytes.len() > MAX_BYTES {
            return Err(Error::Rejected("esa fuente es demasiado grande".into()));
        }
        if self.registered.len() >= MAX_FONTS {
            return Err(Error::Rejected(format!("ya hay {MAX_FONTS} fuentes registradas")));
        }
        let face = Face::parse(bytes, 0).map_err(|_| Error::Unsupported("no se pudo leer esa fuente (¿es TrueType u OpenType?)".into()))?;
        if face.tables().glyf.is_none() {
            return Err(Error::Unsupported("solo se pueden usar fuentes con contornos TrueType (no CFF/PostScript)".into()));
        }
        match face.permissions() {
            Some(Permissions::Restricted) | Some(Permissions::PreviewAndPrint) => {
                return Err(Error::Rejected("la licencia de esa fuente no permite incrustarla en documentos".into()));
            }
            _ => {}
        }
        let family = name_of(&face, &[name_id::TYPOGRAPHIC_FAMILY, name_id::FAMILY]).ok_or_else(|| Error::Unsupported("la fuente no dice su nombre".into()))?;
        let style = name_of(&face, &[name_id::TYPOGRAPHIC_SUBFAMILY, name_id::SUBFAMILY]).unwrap_or_default().to_ascii_lowercase();
        let bold = face.is_bold() || style.contains("bold");
        let italic = face.is_italic() || face.is_oblique() || style.contains("italic") || style.contains("oblique");
        let label = match (bold, italic) {
            (true, true) => "BoldItalic",
            (true, false) => "Bold",
            (false, true) => "Italic",
            _ => "Regular",
        };
        let key = format!("{}-{label}", family.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>());
        let norm = normalized(&family);
        self.registered.retain(|r| !(r.family == norm && r.bold == bold && r.italic == italic));
        // The font lives as long as the process: the fallback machinery hands out `'static` data.
        let data: &'static [u8] = Box::leak(bytes.to_vec().into_boxed_slice());
        let key: &'static str = Box::leak(key.into_boxed_str());
        self.registered.push(Registered { family: norm, bold, italic, key, data });
        Ok(RegisteredFont { family, bold, italic })
    }

    /// The font to draw letters of `base_font` with when its own cannot: a registered font of
    /// the same family if there is one (similarity 100, or 90 in another style), else the
    /// closest bundled one.
    pub(crate) fn pick_font(&self, base_font: &str, flags: Option<u32>) -> Choice {
        let normal = normalized(base_font);
        let bold = ["bold", "black", "heavy", "demi"].iter().any(|w| normal.contains(w)) || flags.is_some_and(|f| f & (1 << 18) != 0);
        let italic = ["italic", "oblique"].iter().any(|w| normal.contains(w)) || flags.is_some_and(|f| f & (1 << 6) != 0);
        let of_family = || self.registered.iter().filter(|r| normal.starts_with(&r.family) || normal.contains(&r.family));
        if let Some(r) = of_family().find(|r| r.bold == bold && r.italic == italic) {
            return Choice { key: r.key, data: r.data, similarity: 100 };
        }
        if let Some(r) = of_family().next() {
            return Choice { key: r.key, data: r.data, similarity: 90 };
        }
        crate::fallback::pick(base_font, flags)
    }

    /// The font that would draw letters `run`'s font lacks, and how close it is (0-100): the
    /// registered or bundled font. Show it before editing: under 90 it is a different face.
    pub fn substitute_for(&self, run: &crate::text::TextRun) -> (String, u8) {
        let c = self.pick_font(&run.base_font, run.font_flags);
        (c.key.to_string(), c.similarity)
    }

    /// The fonts of the document, with what editing them needs: see [`RequiredFont`]. Fonts that
    /// no text uses are left out.
    pub fn required_fonts(&self) -> Result<Vec<RequiredFont>> {
        let mut out: Vec<RequiredFont> = Vec::new();
        for page in 0..self.page_count() {
            let id = self.page_id(page)?;
            let fonts = fonts_of(&self.pdf, &self.resources(id));
            for run in self.text_runs(page)? {
                let Some(font) = fonts.get(&run.font) else { continue };
                let at = match out.iter().position(|f| f.base_font == run.base_font) {
                    Some(i) => i,
                    None => {
                        let choice = self.pick_font(&run.base_font, font.flags());
                        let name = normalized(&run.base_font);
                        out.push(RequiredFont {
                            base_font: run.base_font.clone(),
                            class: font.class,
                            pages: Vec::new(),
                            runs: 0,
                            needed: matches!(font.class, FontClass::NotEmbedded | FontClass::EmbeddedSubset),
                            registered: self.registered.iter().any(|r| name.contains(&r.family)),
                            bundled: choice.key.to_string(),
                            similarity: choice.similarity,
                            source: SOURCES.iter().find(|(words, _)| words.iter().any(|w| name.contains(w))).map(|(_, s)| s.clone()),
                        });
                        out.len() - 1
                    }
                };
                out[at].runs += 1;
                if !out[at].pages.contains(&page) {
                    out[at].pages.push(page);
                }
            }
        }
        Ok(out)
    }
}
