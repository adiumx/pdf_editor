//! Interactive forms (AcroForm): reading fields and filling them in.
//!
//! A field is not a rectangle on a page but a node in a tree. Its name is the
//! dotted path of its ancestors' `/T`, its type, flags and value are inherited
//! from them, and the rectangles are its *widgets*, which are annotations on
//! pages: one, several (a checkbox repeated on every sheet), or the field
//! itself when the two are merged in one dictionary.
//!
//! Filling a field sets its value and, for every widget, an appearance
//! stream. The second half matters as much as the first: many forms (the ones
//! this editor was written for among them) have text widgets with no
//! appearance at all, and a viewer that does not build its own then shows an
//! empty box over a field that holds a value.

use std::collections::HashMap;

use lopdf::{Dictionary, Object, ObjectId, Stream, StringFormat};

use crate::error::{Error, Result};
use crate::fallback;
use crate::font::{num, resolve, Font};
use crate::Document;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    Text,
    Checkbox,
    Radio,
    PushButton,
    Combo,
    List,
    Signature,
}

#[derive(Debug, Clone)]
pub struct Widget {
    /// Page it sits on, when the page's `/Annots` lists it.
    pub page: Option<usize>,
    /// `[x0, y0, x1, y1]` in default user space.
    pub rect: [f32; 4],
    /// The same box as a reader shows it (top-left origin, y down).
    pub display: [f32; 4],
    /// For checkboxes and radio buttons: the name of this widget's "on" state.
    pub on_state: Option<String>,
    /// It carries an appearance stream, which is what a reader draws. A widget
    /// without one shows as an empty box in readers that do not build their own.
    pub has_appearance: bool,
    id: ObjectId,
}

impl Widget {
    pub(crate) fn id(&self) -> ObjectId {
        self.id
    }
}

#[derive(Debug, Clone)]
pub struct Field {
    /// Fully qualified name.
    pub name: String,
    pub kind: FieldKind,
    /// Text of a text or choice field; for buttons, the selected state
    /// (`Off` when nothing is).
    pub value: String,
    pub checked: bool,
    pub read_only: bool,
    pub required: bool,
    pub multiline: bool,
    pub password: bool,
    pub comb: bool,
    pub editable: bool,
    /// A list box that allows more than one row to be selected.
    pub multi_select: bool,
    pub max_len: Option<usize>,
    /// `(export value, label)` of a choice field's options.
    pub options: Vec<(String, String)>,
    /// Where this field comes in the order a person fills the form: by page,
    /// then top to bottom, then left to right. Use it for a "next field" button.
    pub order: usize,
    pub widgets: Vec<Widget>,
    /// A signature field that holds a completed signature. Editing the document
    /// invalidates it.
    pub signed: bool,
    /// Who signed, when the signature says (`/Name`).
    pub signer: Option<String>,
    id: ObjectId,
    /// Default appearance string and alignment (0 left, 1 centre, 2 right).
    da: Vec<u8>,
    q: i64,
}

impl Field {
    pub(crate) fn id(&self) -> ObjectId {
        self.id
    }

    /// How the field's text is aligned inside its box.
    pub fn text_align(&self) -> crate::add::Align {
        match self.q {
            1 => crate::add::Align::Center,
            2 => crate::add::Align::Right,
            _ => crate::add::Align::Left,
        }
    }
}

#[derive(Default, Clone)]
struct Inherited {
    ft: Option<String>,
    ff: u32,
    v: Option<Object>,
    da: Option<Vec<u8>>,
    q: i64,
    max_len: Option<usize>,
    opt: Option<Object>,
}

impl Document {
    /// The form's dictionary, whether the catalog holds it directly or points to it.
    pub(crate) fn acroform_dict(&self) -> Option<&Dictionary> {
        let root = self.pdf.trailer.get(b"Root").ok()?.as_reference().ok()?;
        let catalog = self.pdf.get_dictionary(root).ok()?;
        resolve(&self.pdf, catalog.get(b"AcroForm").ok()?).as_dict().ok()
    }

    /// Every terminal field in the document's form, in tree order.
    pub fn fields(&self) -> Vec<Field> {
        let mut out = Vec::new();
        let Some(dict) = self.acroform_dict() else { return out };
        let defaults = Inherited {
            da: dict.get(b"DA").ok().and_then(|o| resolve(&self.pdf, o).as_str().ok()).map(<[u8]>::to_vec),
            ..Inherited::default()
        };
        let pages = self.annotation_pages();
        let roots: Vec<ObjectId> = dict
            .get(b"Fields")
            .ok()
            .and_then(|o| resolve(&self.pdf, o).as_array().ok())
            .map(|a| a.iter().filter_map(|o| o.as_reference().ok()).collect())
            .unwrap_or_default();
        for id in roots {
            self.collect_field(id, "", &defaults, &pages, &mut out, 0);
        }
        // Reading order: page, then rows top to bottom (tops within a few points
        // share a row), then left to right. Rows are found by scanning tops in
        // order, not by comparing pairs, which would not be a consistent order.
        let place = |f: &Field| {
            let w = f.widgets.first();
            (w.and_then(|w| w.page).unwrap_or(usize::MAX), w.map_or(0.0, |w| w.display[1]), w.map_or(0.0, |w| w.display[0]))
        };
        let mut by_top: Vec<usize> = (0..out.len()).collect();
        by_top.sort_by(|&a, &b| {
            let ((pa, ya, _), (pb, yb, _)) = (place(&out[a]), place(&out[b]));
            pa.cmp(&pb).then(ya.total_cmp(&yb))
        });
        let mut row_of = vec![0usize; out.len()];
        let (mut row, mut row_page, mut row_top) = (0usize, usize::MAX, f32::MIN);
        for &i in &by_top {
            let (page, top, _) = place(&out[i]);
            if page != row_page || top - row_top > 6.0 {
                row += 1;
                (row_page, row_top) = (page, top);
            }
            row_of[i] = row;
        }
        let mut ranked: Vec<usize> = (0..out.len()).collect();
        ranked.sort_by(|&a, &b| row_of[a].cmp(&row_of[b]).then(place(&out[a]).2.total_cmp(&place(&out[b]).2)).then(a.cmp(&b)));
        for (rank, index) in ranked.into_iter().enumerate() {
            out[index].order = rank;
        }
        out
    }

    pub fn field(&self, name: &str) -> Option<Field> {
        self.fields().into_iter().find(|f| f.name == name)
    }

    fn annotation_pages(&self) -> HashMap<ObjectId, usize> {
        let mut map = HashMap::new();
        for (index, page) in self.pages.iter().enumerate() {
            let Ok(dict) = self.pdf.get_dictionary(*page) else { continue };
            let Some(Object::Array(list)) = dict.get(b"Annots").ok().map(|o| resolve(&self.pdf, o)) else { continue };
            for item in list {
                if let Ok(id) = item.as_reference() {
                    map.insert(id, index);
                }
            }
        }
        map
    }

    fn collect_field(
        &self,
        id: ObjectId,
        prefix: &str,
        parent: &Inherited,
        pages: &HashMap<ObjectId, usize>,
        out: &mut Vec<Field>,
        depth: usize,
    ) {
        let Ok(dict) = self.pdf.get_dictionary(id) else { return };
        if depth > 16 {
            return;
        }
        let mut here = parent.clone();
        let name = dict.get(b"T").ok().and_then(|o| resolve(&self.pdf, o).as_str().ok()).map(text_string);
        let full = match (&name, prefix.is_empty()) {
            (Some(n), true) => n.clone(),
            (Some(n), false) => format!("{prefix}.{n}"),
            (None, _) => prefix.to_string(),
        };
        if let Some(ft) = dict.get(b"FT").ok().and_then(|o| o.as_name().ok()) {
            here.ft = Some(String::from_utf8_lossy(ft).into_owned());
        }
        if let Some(ff) = dict.get(b"Ff").ok().and_then(|o| num(&self.pdf, o)) {
            here.ff = ff as u32;
        }
        if let Ok(v) = dict.get(b"V") {
            here.v = Some(resolve(&self.pdf, v).clone());
        }
        if let Some(da) = dict.get(b"DA").ok().and_then(|o| resolve(&self.pdf, o).as_str().ok()) {
            here.da = Some(da.to_vec());
        }
        if let Some(q) = dict.get(b"Q").ok().and_then(|o| num(&self.pdf, o)) {
            here.q = q as i64;
        }
        if let Some(m) = dict.get(b"MaxLen").ok().and_then(|o| num(&self.pdf, o)) {
            here.max_len = Some(m as usize);
        }
        if let Ok(opt) = dict.get(b"Opt") {
            here.opt = Some(resolve(&self.pdf, opt).clone());
        }

        let kids: Vec<ObjectId> = dict
            .get(b"Kids")
            .ok()
            .and_then(|o| resolve(&self.pdf, o).as_array().ok())
            .map(|a| a.iter().filter_map(|o| o.as_reference().ok()).collect())
            .unwrap_or_default();
        let is_widget = |kid: &ObjectId| {
            self.pdf.get_dictionary(*kid).map(|d| !d.has(b"T") && d.has(b"Rect")).unwrap_or(false)
        };
        let has_field_kids = kids.iter().any(|k| !is_widget(k));
        if has_field_kids {
            for kid in kids {
                self.collect_field(kid, &full, &here, pages, out, depth + 1);
            }
            return;
        }

        let widget_ids = if kids.is_empty() { vec![id] } else { kids };
        let Some(kind) = field_kind(&here) else { return };
        let to_display = |page: Option<usize>, r: [f32; 4]| match page.and_then(|p| self.display_matrix(p).ok()) {
            Some(m) => crate::text::transform_box(m, r),
            None => r,
        };
        let widgets: Vec<Widget> = widget_ids
            .into_iter()
            .filter_map(|wid| {
                let w = self.pdf.get_dictionary(wid).ok()?;
                let rect = rect_of(&self.pdf, w)?;
                let page = pages.get(&wid).copied();
                Some(Widget {
                    page,
                    rect,
                    display: to_display(page, rect),
                    on_state: on_state(&self.pdf, w),
                    has_appearance: has_appearance(&self.pdf, w),
                    id: wid,
                })
            })
            .collect();

        let value = match &here.v {
            Some(Object::String(b, _)) => text_string(b),
            Some(Object::Name(n)) => String::from_utf8_lossy(n).into_owned(),
            Some(Object::Array(a)) => a
                .iter()
                .filter_map(|o| resolve(&self.pdf, o).as_str().ok())
                .map(text_string)
                .collect::<Vec<_>>()
                .join("\n"),
            _ => String::new(),
        };
        // A signature's value is a dictionary; one with contents or a byte range is
        // a completed signature, an empty field is not.
        let (signed, signer) = match (&here.v, kind) {
            (Some(Object::Dictionary(v)), FieldKind::Signature) if v.has(b"Contents") || v.has(b"ByteRange") => (
                true,
                v.get(b"Name").ok().and_then(|o| resolve(&self.pdf, o).as_str().ok()).map(text_string),
            ),
            _ => (false, None),
        };
        let checked = matches!(kind, FieldKind::Checkbox | FieldKind::Radio) && !value.is_empty() && value != "Off";
        out.push(Field {
            name: full,
            kind,
            value,
            checked,
            read_only: here.ff & 1 != 0,
            required: here.ff & 2 != 0,
            multiline: kind == FieldKind::Text && here.ff & (1 << 12) != 0,
            password: kind == FieldKind::Text && here.ff & (1 << 13) != 0,
            comb: kind == FieldKind::Text && here.ff & (1 << 24) != 0,
            editable: kind == FieldKind::Combo && here.ff & (1 << 18) != 0,
            multi_select: kind == FieldKind::List && here.ff & (1 << 21) != 0,
            max_len: here.max_len,
            options: options_of(&self.pdf, here.opt.as_ref()),
            order: 0,
            widgets,
            signed,
            signer,
            id,
            da: here.da.clone().unwrap_or_default(),
            q: here.q,
        });
    }
}

fn field_kind(i: &Inherited) -> Option<FieldKind> {
    Some(match i.ft.as_deref()? {
        "Tx" => FieldKind::Text,
        "Btn" if i.ff & (1 << 16) != 0 => FieldKind::PushButton,
        "Btn" if i.ff & (1 << 15) != 0 => FieldKind::Radio,
        "Btn" => FieldKind::Checkbox,
        "Ch" if i.ff & (1 << 17) != 0 => FieldKind::Combo,
        "Ch" => FieldKind::List,
        "Sig" => FieldKind::Signature,
        _ => return None,
    })
}

fn rect_of(pdf: &lopdf::Document, widget: &Dictionary) -> Option<[f32; 4]> {
    let a = resolve(pdf, widget.get(b"Rect").ok()?).as_array().ok()?;
    let v: Vec<f32> = a.iter().map(|o| num(pdf, o).unwrap_or(0.0)).collect();
    (v.len() == 4).then(|| [v[0].min(v[2]), v[1].min(v[3]), v[0].max(v[2]), v[1].max(v[3])])
}

/// The name of a checkbox or radio widget's "on" appearance: whichever state
/// of `/AP /N` is not `Off`.
fn on_state(pdf: &lopdf::Document, widget: &Dictionary) -> Option<String> {
    let ap = resolve(pdf, widget.get(b"AP").ok()?).as_dict().ok()?;
    let normal = resolve(pdf, ap.get(b"N").ok()?).as_dict().ok()?;
    normal
        .iter()
        .map(|(k, _)| String::from_utf8_lossy(k).into_owned())
        .find(|k| k != "Off")
}

fn has_appearance(pdf: &lopdf::Document, widget: &Dictionary) -> bool {
    widget
        .get(b"AP")
        .ok()
        .and_then(|o| resolve(pdf, o).as_dict().ok())
        .is_some_and(|ap| ap.has(b"N"))
}

fn options_of(pdf: &lopdf::Document, opt: Option<&Object>) -> Vec<(String, String)> {
    let Some(Object::Array(list)) = opt else { return Vec::new() };
    list.iter()
        .filter_map(|item| match resolve(pdf, item) {
            Object::String(b, _) => Some((text_string(b), text_string(b))),
            Object::Array(pair) if pair.len() >= 2 => {
                let (a, b) = (resolve(pdf, &pair[0]).as_str().ok()?, resolve(pdf, &pair[1]).as_str().ok()?);
                Some((text_string(a), text_string(b)))
            }
            _ => None,
        })
        .collect()
}

/// A PDF text string as text: UTF-16 (with its byte-order mark), UTF-8 with a
/// mark, or PDFDocEncoding, which agrees with Latin-1 where it matters here.
pub(crate) fn text_string(bytes: &[u8]) -> String {
    match bytes {
        [0xFE, 0xFF, rest @ ..] => String::from_utf16_lossy(&rest.chunks(2).map(|c| u16::from_be_bytes([c[0], *c.get(1).unwrap_or(&0)])).collect::<Vec<_>>()),
        [0xFF, 0xFE, rest @ ..] => String::from_utf16_lossy(&rest.chunks(2).map(|c| u16::from_le_bytes([c[0], *c.get(1).unwrap_or(&0)])).collect::<Vec<_>>()),
        [0xEF, 0xBB, 0xBF, rest @ ..] => String::from_utf8_lossy(rest).into_owned(),
        _ => bytes.iter().map(|b| *b as char).collect(),
    }
}

/// Text as a PDF text string: plain bytes when every character is in the
/// range PDFDocEncoding and Latin-1 share, UTF-16BE with a mark otherwise.
pub(crate) fn encode_text_string(text: &str) -> Object {
    let plain = text.chars().all(|c| (c as u32) <= 0xFF && !(0x80..=0xA0).contains(&(c as u32)));
    if plain {
        Object::String(text.chars().map(|c| c as u8).collect(), StringFormat::Literal)
    } else {
        let mut bytes = vec![0xFE, 0xFF];
        for unit in text.encode_utf16() {
            bytes.extend(unit.to_be_bytes());
        }
        Object::String(bytes, StringFormat::Hexadecimal)
    }
}

// -- filling ----------------------------------------------------------------

impl Document {
    /// Put `value` into the field called `name`.
    ///
    /// Text and choice fields take the text; checkboxes take `true`/`yes`/`on`
    /// (or their own state name) and `false`/`off`; radio buttons take the name
    /// of the state to select, or `off`. Everything is checked before anything
    /// is changed, and a refusal says why.
    pub fn set_field_value(&mut self, name: &str, value: &str) -> Result<()> {
        self.set_field_value_with(name, value, false)
    }

    /// As [`set_field_value`](Self::set_field_value), optionally filling a
    /// field the form marks read-only. Some forms mark fields meant to be
    /// filled that way by mistake (every checkbox of one real insurance form
    /// does), and every standards-following reader then refuses them. That is
    /// the person's call to make, not the engine's, so the caller asks first.
    pub fn set_field_value_with(&mut self, name: &str, value: &str, ignore_read_only: bool) -> Result<()> {
        self.transaction(|d| d.fill_field(name, value, ignore_read_only))
    }

    fn fill_field(&mut self, name: &str, value: &str, ignore_read_only: bool) -> Result<()> {
        let field = self.field(name).ok_or_else(|| Error::NoSuchField(name.to_string()))?;
        if field.read_only && !ignore_read_only {
            return Err(Error::Rejected(format!("«{name}» es de solo lectura")));
        }
        match field.kind {
            FieldKind::Text => {
                if let Some(max) = field.max_len {
                    if value.chars().count() > max {
                        return Err(Error::Rejected(format!("«{name}» admite como máximo {max} caracteres")));
                    }
                }
                if !field.multiline && value.contains('\n') {
                    return Err(Error::Rejected(format!("«{name}» es de una sola línea")));
                }
                self.fill_text(&field, value)
            }
            FieldKind::Combo => {
                let chosen = field.options.iter().find(|(e, l)| e == value || l == value).map(|(e, _)| e.clone());
                let stored = match (chosen, field.editable) {
                    (Some(export), _) => export,
                    (None, true) => value.to_string(),
                    (None, false) => {
                        return Err(Error::Rejected(format!("«{value}» no es una opción de «{name}»")));
                    }
                };
                self.fill_text(&field, &stored)
            }
            FieldKind::List => {
                // One row, or with several allowed, one per line of `value`.
                let wanted: Vec<&str> = if value.is_empty() {
                    Vec::new()
                } else if field.multi_select {
                    value.split('\n').collect()
                } else {
                    vec![value]
                };
                let mut chosen = Vec::new();
                for item in wanted {
                    let found = field.options.iter().position(|(e, l)| e == item || l == item);
                    chosen.push(found.ok_or_else(|| Error::Rejected(format!("«{item}» no es una opción de «{name}»")))?);
                }
                self.fill_list(&field, &chosen)
            }
            FieldKind::Checkbox | FieldKind::Radio => {
                self.ensure_button_appearances(&field)?;
                let field = self.field(name).ok_or_else(|| Error::NoSuchField(name.to_string()))?;
                self.fill_button(&field, value)
            }
            FieldKind::PushButton => Err(Error::Unsupported(format!("«{name}» es un botón, no un campo que se rellene"))),
            FieldKind::Signature => Err(Error::Unsupported(format!("«{name}» es una firma: no se rellena como un texto"))),
        }
    }

    fn fill_button(&mut self, field: &Field, value: &str) -> Result<()> {
        let lower = value.trim().to_lowercase();
        let on_states: Vec<&String> = field.widgets.iter().filter_map(|w| w.on_state.as_ref()).collect();
        // A state's own name comes first: a radio group's "No" is a state to
        // select, not the word for "unticked".
        let target = if let Some(exact) = on_states.iter().find(|s| ***s == value) {
            (*exact).clone()
        } else if matches!(lower.as_str(), "" | "off" | "false" | "no" | "0") {
            "Off".to_string()
        } else if field.kind == FieldKind::Checkbox && matches!(lower.as_str(), "true" | "yes" | "on" | "1" | "si" | "sí") {
            on_states
                .first()
                .map(|s| (*s).clone())
                .ok_or_else(|| Error::Unsupported(format!("«{}» no trae apariencia de casilla marcada", field.name)))?
        } else {
            return Err(Error::Rejected(format!("«{value}» no es un estado de «{}»", field.name)));
        };
        self.pdf.get_dictionary_mut(field.id)?.set("V", Object::Name(target.as_bytes().to_vec()));
        for widget in &field.widgets {
            let state = if widget.on_state.as_deref() == Some(target.as_str()) { target.as_str() } else { "Off" };
            self.pdf.get_dictionary_mut(widget.id)?.set("AS", Object::Name(state.as_bytes().to_vec()));
        }
        Ok(())
    }

    pub(crate) fn fill_text(&mut self, field: &Field, value: &str) -> Result<()> {
        // Build every appearance first: a failure there leaves the field as it was.
        let mut appearances = Vec::new();
        for widget in &field.widgets {
            appearances.push((widget.id, self.text_appearance(field, widget, value, &[])?));
        }
        self.pdf.get_dictionary_mut(field.id)?.set("V", encode_text_string(value));
        for (widget, stream) in appearances {
            self.set_normal_appearance(widget, stream)?;
        }
        Ok(())
    }

    fn set_normal_appearance(&mut self, widget: ObjectId, stream: ObjectId) -> Result<()> {
        let mut ap = Dictionary::new();
        ap.set("N", Object::Reference(stream));
        self.pdf.get_dictionary_mut(widget)?.set("AP", Object::Dictionary(ap));
        Ok(())
    }

    /// Select the rows at `chosen` (indexes into the options) of a list box.
    pub(crate) fn fill_list(&mut self, field: &Field, chosen: &[usize]) -> Result<()> {
        let mut sorted = chosen.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        let exports: Vec<&String> = sorted.iter().map(|&i| &field.options[i].0).collect();
        let mut appearances = Vec::new();
        for widget in &field.widgets {
            appearances.push((widget.id, self.text_appearance(field, widget, "", &sorted)?));
        }
        let (value, indexes) = match exports.as_slice() {
            [] => (Object::Null, Object::Null),
            [one] if !field.multi_select => (encode_text_string(one), Object::Array(vec![Object::Integer(sorted[0] as i64)])),
            many => (
                Object::Array(many.iter().map(|e| encode_text_string(e)).collect()),
                Object::Array(sorted.iter().map(|&i| Object::Integer(i as i64)).collect()),
            ),
        };
        let dict = self.pdf.get_dictionary_mut(field.id)?;
        dict.set("V", value);
        dict.set("I", indexes);
        for (widget, stream) in appearances {
            self.set_normal_appearance(widget, stream)?;
        }
        Ok(())
    }

    /// The raw font entry `/Helv` or the like names, looked up where the spec
    /// says: the widget's own resources, the field's, then the form's.
    fn default_resource_font(&self, field: &Field, widget: &Widget, name: &str) -> Option<(Object, Dictionary)> {
        let holders = [
            self.pdf.get_dictionary(widget.id).ok(),
            self.pdf.get_dictionary(field.id).ok(),
            self.acroform_dict(),
        ];
        for dict in holders.into_iter().flatten() {
            let Some(Object::Dictionary(resources)) = dict.get(b"DR").ok().map(|o| resolve(&self.pdf, o)) else { continue };
            let Some(Object::Dictionary(fonts)) = resources.get(b"Font").ok().map(|o| resolve(&self.pdf, o)) else { continue };
            if let Ok(entry) = fonts.get(name.as_bytes()) {
                if let Ok(font) = resolve(&self.pdf, entry).as_dict() {
                    return Some((entry.clone(), font.clone()));
                }
            }
        }
        None
    }

    fn text_appearance(&mut self, field: &Field, widget: &Widget, value: &str, selected: &[usize]) -> Result<ObjectId> {
        let [x0, y0, x1, y1] = widget.rect;
        // The text reads upright on the page as shown: the widget's own rotation
        // (`/MK /R`) if it has one, else the page's. On a turned page the box is
        // drawn in a frame turned the other way, so it must be laid out sideways.
        let turn = self.widget_turn(widget);
        let (w, h) = if turn % 180 == 0 { (x1 - x0, y1 - y0) } else { (y1 - y0, x1 - x0) };
        let da = parse_da(&field.da);
        let is_list = field.kind == FieldKind::List;
        let shown: String = if is_list {
            field.options.iter().map(|(_, label)| label.as_str()).collect::<Vec<_>>().join("\n")
        } else if field.password {
            "*".repeat(value.chars().count())
        } else {
            value.to_string()
        };

        // The form's own font when it can draw every character; ours otherwise.
        let own = self
            .default_resource_font(field, widget, &da.font)
            .map(|(entry, dict)| (entry, Font::from_dict(&self.pdf, &da.font, &dict)))
            .filter(|(_, font)| font.encode(&shown).is_ok());
        let (key, data) = fallback::choose(&da.font);
        let (ascent, descent) = own.as_ref().map(|(_, f)| (f.ascent, f.descent)).unwrap_or((905.0, -212.0));
        let measure = |text: &str, size: f32| match &own {
            Some((_, font)) => font.advance_of(&font.encode(text).unwrap_or_default(), size),
            None => fallback::advance(data, text, size),
        };

        let pad = 2.0f32;
        let (inner_w, inner_h) = ((w - 2.0 * pad).max(1.0), (h - 2.0 * pad).max(1.0));
        let line_box = (ascent - descent) / 1000.0;
        let wrap = |size: f32| -> Vec<String> {
            let mut lines: Vec<String> = Vec::new();
            for paragraph in shown.split('\n').map(|l| l.trim_end_matches('\r')) {
                if !field.multiline {
                    lines.push(paragraph.to_string());
                    continue;
                }
                let mut current = String::new();
                for word in paragraph.split(' ') {
                    let candidate = if current.is_empty() { word.to_string() } else { format!("{current} {word}") };
                    if !current.is_empty() && measure(&candidate, size) > inner_w {
                        lines.push(std::mem::take(&mut current));
                        current = word.to_string();
                    } else {
                        current = candidate;
                    }
                }
                lines.push(current);
            }
            lines
        };
        let mut size = da.size;
        if size <= 0.0 {
            if field.multiline {
                // Largest size up to 12 at which the wrapped text still fits the box.
                size = 12.0;
                while size > 6.0 && wrap(size).len() as f32 * size * 1.15 > inner_h {
                    size *= 0.92;
                }
            } else if is_list {
                size = 12.0;
            } else {
                size = (inner_h / line_box).min(12.0);
                while size > 4.0 && measure(&shown, size) > inner_w {
                    size *= 0.92;
                }
            }
        }
        let mut lines = wrap(size);
        // A list box scrolls: start at the first row that keeps the first selected one in view.
        let lead_for_rows = size * 1.15;
        let first_row = if is_list {
            let visible = ((inner_h / lead_for_rows).floor() as usize).max(1);
            selected.first().map_or(0, |&s| (s + 1).saturating_sub(visible))
        } else {
            0
        };
        let selected_in_view: Vec<usize> = selected.iter().filter_map(|s| s.checked_sub(first_row)).collect();
        if first_row > 0 {
            lines.drain(..first_row.min(lines.len()));
        }

        // Encode through whichever font was chosen.
        let encode = |this: &mut Document, text: &str| -> Result<Vec<u8>> {
            match &own {
                Some((_, font)) => Ok(font.encode(text).unwrap_or_default()),
                None => {
                    this.ensure_fallback(key, data)?;
                    let embedded = this.fallbacks.get_mut(key).expect("just ensured");
                    Ok(embedded.write(&mut this.pdf, text, size)?.0)
                }
            }
        };
        let hex = |b: &[u8]| b.iter().map(|x| format!("{x:02X}")).collect::<String>();

        let mut body = String::new();
        let lead = size * 1.15;
        if field.comb && field.max_len.unwrap_or(0) > 0 && !field.multiline {
            let cells = field.max_len.unwrap_or(1) as f32;
            let cell = w / cells;
            let y = (h - size * line_box) / 2.0 - descent / 1000.0 * size;
            for (i, ch) in lines[0].chars().enumerate() {
                let text = ch.to_string();
                let bytes = encode(self, &text)?;
                let x = cell * i as f32 + (cell - measure(&text, size)) / 2.0;
                body.push_str(&format!("1 0 0 1 {x:.2} {y:.2} Tm <{}> Tj\n", hex(&bytes)));
            }
        } else {
            for (i, line) in lines.iter().enumerate() {
                let bytes = encode(self, line)?;
                let width = measure(line, size);
                let x = match field.q {
                    1 => (w - width) / 2.0,
                    2 => w - pad - width,
                    _ => pad,
                };
                let y = if field.multiline || is_list {
                    h - pad - ascent / 1000.0 * size - lead * i as f32
                } else {
                    (h - size * line_box) / 2.0 - descent / 1000.0 * size
                };
                body.push_str(&format!("1 0 0 1 {x:.2} {y:.2} Tm <{}> Tj\n", hex(&bytes)));
            }
        }

        let (font_entry, resource) = match &own {
            Some((entry, _)) => (entry.clone(), da.font.clone()),
            None => {
                // An empty field never encoded anything, so the font may not be in yet.
                self.ensure_fallback(key, data)?;
                let embedded = &self.fallbacks[key];
                (Object::Reference(embedded.font_id), embedded.resource.clone())
            }
        };
        let font_entry = match font_entry {
            Object::Reference(id) => Object::Reference(id),
            direct => Object::Reference(self.pdf.add_object(direct)),
        };

        let mut content = String::from("/Tx BMC\nq\n");
        let widget_dict = self.pdf.get_dictionary(widget.id)?.clone();
        if let Some(mk) = widget_dict.get(b"MK").ok().and_then(|o| resolve(&self.pdf, o).as_dict().ok()) {
            if let Some(bg) = color_ops(&self.pdf, mk.get(b"BG").ok(), false) {
                content.push_str(&format!("{bg}\n0 0 {w:.2} {h:.2} re f\n"));
            }
            if let Some(bc) = color_ops(&self.pdf, mk.get(b"BC").ok(), true) {
                let bw = widget_dict
                    .get(b"BS")
                    .ok()
                    .and_then(|o| resolve(&self.pdf, o).as_dict().ok())
                    .and_then(|d| d.get(b"W").ok())
                    .and_then(|o| num(&self.pdf, o))
                    .unwrap_or(1.0);
                if bw > 0.0 {
                    content.push_str(&format!("{bc}\n{bw:.2} w\n{:.2} {:.2} {:.2} {:.2} re S\n", bw / 2.0, bw / 2.0, w - bw, h - bw));
                }
            }
        }
        content.push_str(&format!("{pad:.2} {pad:.2} {inner_w:.2} {inner_h:.2} re W n\n"));
        for row in selected_in_view {
            let top = h - pad - lead * row as f32;
            content.push_str(&format!("0.6 0.75 0.85 rg\n{pad:.2} {:.2} {inner_w:.2} {lead:.2} re f\n", top - lead));
        }
        content.push_str(&format!("BT\n{}\n/{resource} {size:.2} Tf\n", da.color));
        content.push_str(&body);
        content.push_str("ET\nQ\nEMC\n");

        let mut fonts = Dictionary::new();
        fonts.set(resource.as_str(), font_entry);
        let mut resources = Dictionary::new();
        resources.set("Font", Object::Dictionary(fonts));
        let mut dict = Dictionary::new();
        dict.set("Type", Object::Name(b"XObject".to_vec()));
        dict.set("Subtype", Object::Name(b"Form".to_vec()));
        dict.set("BBox", vec![Object::Real(0.0), Object::Real(0.0), Object::Real(w), Object::Real(h)]);
        if let Some(matrix) = turn_matrix(turn, w, h) {
            dict.set("Matrix", Object::Array(matrix.iter().map(|v| Object::Real(*v)).collect()));
        }
        dict.set("Resources", Object::Dictionary(resources));
        let mut stream = Stream::new(dict, content.into_bytes());
        let _ = stream.compress();
        Ok(self.pdf.add_object(stream))
    }
}

struct DefaultAppearance {
    font: String,
    size: f32,
    /// Colour operators to repeat in the appearance (`0 g`, `0 0 1 rg`...).
    color: String,
}

fn parse_da(da: &[u8]) -> DefaultAppearance {
    let text = String::from_utf8_lossy(da);
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let (mut font, mut size, mut color) = ("Helv".to_string(), 0.0, "0 g".to_string());
    for (i, token) in tokens.iter().enumerate() {
        match *token {
            "Tf" if i >= 2 => {
                font = tokens[i - 2].trim_start_matches('/').to_string();
                size = tokens[i - 1].parse().unwrap_or(0.0);
            }
            "g" if i >= 1 => color = tokens[i - 1..=i].join(" "),
            "rg" if i >= 3 => color = tokens[i - 3..=i].join(" "),
            "k" if i >= 4 => color = tokens[i - 4..=i].join(" "),
            _ => {}
        }
    }
    DefaultAppearance { font, size, color }
}

/// `MK` colours: one number is grey, three RGB, four CMYK; none is transparent.
fn color_ops(pdf: &lopdf::Document, array: Option<&Object>, stroke: bool) -> Option<String> {
    let list = resolve(pdf, array?).as_array().ok()?;
    let v: Vec<String> = list.iter().filter_map(|o| num(pdf, o)).map(|n| format!("{n:.3}")).collect();
    let op = match (v.len(), stroke) {
        (1, false) => "g",
        (1, true) => "G",
        (3, false) => "rg",
        (3, true) => "RG",
        (4, false) => "k",
        (4, true) => "K",
        _ => return None,
    };
    Some(format!("{} {op}", v.join(" ")))
}

/// A completed digital signature in the document.
#[derive(Debug, Clone, PartialEq)]
pub struct Signature {
    /// The signature field's name.
    pub field: String,
    pub signer: Option<String>,
}

impl Document {
    /// The signatures the document carries. Any edit invalidates them, so a
    /// caller should say so before the first one.
    pub fn signatures(&self) -> Vec<Signature> {
        self.fields()
            .into_iter()
            .filter(|f| f.signed)
            .map(|f| Signature { field: f.name, signer: f.signer })
            .collect()
    }
}

// -- buttons without an appearance --------------------------------------------

impl Document {
    /// Give every checkbox or radio widget that has no "on" appearance one, so it
    /// can be ticked: forms made by some producers carry the fields but no
    /// drawings for their states, and a reader then shows nothing when ticked.
    ///
    /// A checkbox's on state is called `Yes`; a radio button's is its `/Opt`
    /// entry when there is one, or its position in the group.
    fn ensure_button_appearances(&mut self, field: &Field) -> Result<()> {
        for (index, widget) in field.widgets.iter().enumerate() {
            if widget.on_state.is_some() {
                continue;
            }
            let state = match field.kind {
                FieldKind::Radio => field
                    .options
                    .get(index)
                    .map(|(export, _)| export.clone())
                    .unwrap_or_else(|| index.to_string()),
                _ => "Yes".to_string(),
            };
            let [x0, y0, x1, y1] = widget.rect;
            let (w, h) = (x1 - x0, y1 - y0);
            let dict = self.pdf.get_dictionary(widget.id)?.clone();
            let mk = dict.get(b"MK").ok().and_then(|o| resolve(&self.pdf, o).as_dict().ok()).cloned();
            let mut frame = String::new();
            if let Some(mk) = &mk {
                if let Some(bg) = color_ops(&self.pdf, mk.get(b"BG").ok(), false) {
                    frame.push_str(&format!("{bg}\n0 0 {w:.2} {h:.2} re f\n"));
                }
                if let Some(bc) = color_ops(&self.pdf, mk.get(b"BC").ok(), true) {
                    frame.push_str(&format!("{bc}\n1 w\n0.5 0.5 {:.2} {:.2} re S\n", w - 1.0, h - 1.0));
                }
            }
            let style = mk
                .as_ref()
                .and_then(|m| m.get(b"CA").ok())
                .and_then(|o| resolve(&self.pdf, o).as_str().ok())
                .and_then(|b| b.first().copied())
                .map(|c| c as char)
                .unwrap_or(if field.kind == FieldKind::Radio { 'l' } else { '4' });
            let color = parse_da(&field.da).color;
            let mark = mark_ops(style, w, h, &color);
            let on = self.add_button_state(w, h, &format!("q\n{frame}Q\nq\n{mark}Q\n"));
            let off = self.add_button_state(w, h, &format!("q\n{frame}Q\n"));
            let mut states = Dictionary::new();
            states.set(state.as_str(), Object::Reference(on));
            states.set("Off", Object::Reference(off));
            let mut ap = Dictionary::new();
            ap.set("N", Object::Dictionary(states));
            let target = self.pdf.get_dictionary_mut(widget.id)?;
            target.set("AP", Object::Dictionary(ap));
            target.set("AS", Object::Name(b"Off".to_vec()));
        }
        Ok(())
    }

    fn add_button_state(&mut self, w: f32, h: f32, content: &str) -> ObjectId {
        let mut dict = Dictionary::new();
        dict.set("Type", Object::Name(b"XObject".to_vec()));
        dict.set("Subtype", Object::Name(b"Form".to_vec()));
        dict.set("BBox", vec![Object::Real(0.0), Object::Real(0.0), Object::Real(w), Object::Real(h)]);
        let mut stream = Stream::new(dict, content.as_bytes().to_vec());
        let _ = stream.compress();
        self.pdf.add_object(stream)
    }
}

/// The drawing of a tick, cross, circle, square or diamond in a `w` by `h` box
/// (the characters a widget's `/MK /CA` uses: `4` ✔, `8` ✘, `l` ●, `n` ■, `u` ◆).
fn mark_ops(style: char, w: f32, h: f32, color: &str) -> String {
    let s = w.min(h);
    let (cx, cy) = (w / 2.0, h / 2.0);
    let stroke = color.replace(" rg", " RG").replace(" g", " G").replace(" k", " K");
    match style {
        '8' => format!(
            "{stroke}\n{:.2} w 1 J\n{:.2} {:.2} m {:.2} {:.2} l S\n{:.2} {:.2} m {:.2} {:.2} l S\n",
            s * 0.1, cx - s * 0.28, cy - s * 0.28, cx + s * 0.28, cy + s * 0.28, cx - s * 0.28, cy + s * 0.28, cx + s * 0.28, cy - s * 0.28
        ),
        'l' => {
            let r = s * 0.27;
            let k = r * 0.5523;
            format!(
                "{color}\n{:.2} {:.2} m {:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c {:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c {:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c {:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c f\n",
                cx + r, cy, cx + r, cy + k, cx + k, cy + r, cx, cy + r, cx - k, cy + r, cx - r, cy + k, cx - r, cy,
                cx - r, cy - k, cx - k, cy - r, cx, cy - r, cx + k, cy - r, cx + r, cy - k, cx + r, cy
            )
        }
        'n' => format!("{color}\n{:.2} {:.2} {:.2} {:.2} re f\n", cx - s * 0.25, cy - s * 0.25, s * 0.5, s * 0.5),
        'u' => format!(
            "{color}\n{:.2} {:.2} m {:.2} {:.2} l {:.2} {:.2} l {:.2} {:.2} l f\n",
            cx, cy + s * 0.32, cx + s * 0.32, cy, cx, cy - s * 0.32, cx - s * 0.32, cy
        ),
        // `4` and anything else: a tick.
        _ => format!(
            "{stroke}\n{:.2} w 1 J 1 j\n{:.2} {:.2} m {:.2} {:.2} l {:.2} {:.2} l S\n",
            s * 0.12, cx - s * 0.3, cy + s * 0.02, cx - s * 0.08, cy - s * 0.24, cx + s * 0.32, cy + s * 0.3
        ),
    }
}

// -- widgets on turned pages ------------------------------------------------------

impl Document {
    /// How far (counter-clockwise, in degrees) a widget's drawing is turned so it reads
    /// upright on the page as shown: its own `/MK /R`, or the page's rotation.
    fn widget_turn(&self, widget: &Widget) -> i32 {
        let own = self
            .pdf
            .get_dictionary(widget.id)
            .ok()
            .and_then(|d| d.get(b"MK").ok())
            .and_then(|o| resolve(&self.pdf, o).as_dict().ok())
            .and_then(|mk| mk.get(b"R").ok())
            .and_then(|o| num(&self.pdf, o))
            .map(|r| (r as i32).rem_euclid(360));
        own.unwrap_or_else(|| widget.page.and_then(|p| self.page_size(p).ok()).map_or(0, |s| s.rotation))
    }
}

/// The form matrix that turns a `w` by `h` drawing so that it fills the widget's box.
fn turn_matrix(turn: i32, w: f32, h: f32) -> Option<[f32; 6]> {
    match turn {
        90 => Some([0.0, 1.0, -1.0, 0.0, h, 0.0]),
        180 => Some([-1.0, 0.0, 0.0, -1.0, w, h]),
        270 => Some([0.0, -1.0, 1.0, 0.0, 0.0, w]),
        _ => None,
    }
}
