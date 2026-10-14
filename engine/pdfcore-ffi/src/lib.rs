//! The C interface.
//!
//! Dart's FFI calls C functions, so everything here is `extern "C"`, takes and
//! returns plain pointers and numbers, and carries structured data as JSON
//! text: one parser on the Dart side instead of a C struct per type.
//!
//! - A document is an opaque handle from `pdfcore_open`, freed by `pdfcore_close`.
//! - Every other function returns a JSON string the caller frees with
//!   `pdfcore_free_string`. Success is the data, or `{"ok":true}`; failure is
//!   `{"error":"...","kind":"..."}` (and `"chars"` for missing glyphs).
//! - A text run is named by its `id` in the last reading of its page. Any edit,
//!   undo or redo invalidates every id: read the page again.
//! - Nothing unwinds across the boundary: a panic becomes an error.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{c_char, c_int, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};

use pdfcore::{Align, Anchor, Document, Error, NewText, TextRun};
use serde_json::{json, Value};

pub struct PdfDoc {
    doc: Document,
    /// What `pdfcore_page` last returned, so a run id can be turned back into
    /// the run it named.
    reading: HashMap<usize, Vec<TextRun>>,
}

thread_local! {
    static LAST_ERROR: RefCell<CString> = RefCell::new(CString::default());
}

fn remember(message: &str) {
    LAST_ERROR.with(|e| *e.borrow_mut() = CString::new(message.replace('\0', " ")).unwrap_or_default());
}

fn text(value: Value) -> *mut c_char {
    CString::new(value.to_string()).unwrap_or_default().into_raw()
}

fn failure(error: &Error) -> Value {
    let kind = match error {
        Error::Pdf(_) => "pdf",
        Error::NoSuchPage(_) => "no_such_page",
        Error::MissingGlyphs(_) => "missing_glyphs",
        Error::NoSuchField(_) => "no_such_field",
        Error::Rejected(_) => "rejected",
        Error::Unsupported(_) => "unsupported",
    };
    let mut out = json!({ "error": error.to_string(), "kind": kind });
    if let Error::MissingGlyphs(chars) = error {
        out["chars"] = json!(chars.iter().collect::<String>());
    }
    out
}

fn guard(body: impl FnOnce() -> Value) -> *mut c_char {
    match catch_unwind(AssertUnwindSafe(body)) {
        Ok(value) => text(value),
        Err(_) => text(json!({ "error": "fallo interno del motor", "kind": "panic" })),
    }
}

/// # Safety
/// `ptr` must be null or a NUL-terminated string valid for the call.
unsafe fn string_arg<'a>(ptr: *const c_char) -> Option<&'a str> {
    if ptr.is_null() { None } else { CStr::from_ptr(ptr).to_str().ok() }
}

/// # Safety
/// `doc` must be a live handle from `pdfcore_open`.
unsafe fn handle<'a>(doc: *mut PdfDoc) -> Option<&'a mut PdfDoc> {
    doc.as_mut()
}

const BAD_ARGUMENT: &str = r#"{"error":"argumento no válido","kind":"bad_argument"}"#;

fn bad_argument() -> *mut c_char {
    CString::new(BAD_ARGUMENT).unwrap().into_raw()
}

// -- lifetime ----------------------------------------------------------------

/// Open a PDF from `len` bytes at `data`. Null on failure; see `pdfcore_last_error`.
///
/// # Safety
/// `data` must point to `len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn pdfcore_open(data: *const u8, len: usize) -> *mut PdfDoc {
    if data.is_null() {
        remember("sin datos");
        return std::ptr::null_mut();
    }
    let bytes = std::slice::from_raw_parts(data, len);
    match catch_unwind(|| Document::open(bytes)) {
        Ok(Ok(doc)) => Box::into_raw(Box::new(PdfDoc { doc, reading: HashMap::new() })),
        Ok(Err(e)) => {
            remember(&e.to_string());
            std::ptr::null_mut()
        }
        Err(_) => {
            remember("fallo interno del motor al abrir el PDF");
            std::ptr::null_mut()
        }
    }
}

/// Why the last `pdfcore_open` on this thread failed. Valid until the next call.
#[no_mangle]
pub extern "C" fn pdfcore_last_error() -> *const c_char {
    LAST_ERROR.with(|e| e.borrow().as_ptr())
}

/// # Safety
/// `doc` must be null or a handle not yet closed.
#[no_mangle]
pub unsafe extern "C" fn pdfcore_close(doc: *mut PdfDoc) {
    if !doc.is_null() {
        drop(Box::from_raw(doc));
    }
}

/// # Safety
/// `s` must be null or a string returned by this library, not yet freed.
#[no_mangle]
pub unsafe extern "C" fn pdfcore_free_string(s: *mut c_char) {
    if !s.is_null() {
        drop(CString::from_raw(s));
    }
}

/// # Safety
/// `p`/`len` must be exactly what `pdfcore_save` returned, not yet freed.
#[no_mangle]
pub unsafe extern "C" fn pdfcore_free_bytes(p: *mut u8, len: usize) {
    if !p.is_null() {
        drop(Vec::from_raw_parts(p, len, len));
    }
}

// -- reading -----------------------------------------------------------------

/// # Safety
/// `doc` must be a live handle.
#[no_mangle]
pub unsafe extern "C" fn pdfcore_page_count(doc: *mut PdfDoc) -> c_int {
    handle(doc).map(|d| d.doc.page_count() as c_int).unwrap_or(-1)
}

/// A page's size and every run of text on it, as JSON:
/// `{"width","height","rotation","runs":[{"id","text","font","base_font","size","display":[x0,y0,x1,y1],"origin":[x,y],"invisible"}]}`
///
/// # Safety
/// `doc` must be a live handle.
#[no_mangle]
pub unsafe extern "C" fn pdfcore_page(doc: *mut PdfDoc, page: c_int) -> *mut c_char {
    let Some(d) = handle(doc) else { return bad_argument() };
    guard(|| {
        let page = page.max(0) as usize;
        let size = match d.doc.page_size(page) {
            Ok(s) => s,
            Err(e) => return failure(&e),
        };
        let runs = match d.doc.text_runs(page) {
            Ok(r) => r,
            Err(e) => return failure(&e),
        };
        let listed: Vec<Value> = runs
            .iter()
            .enumerate()
            .map(|(id, r)| {
                json!({
                    "id": id, "text": r.text, "font": r.font, "base_font": r.base_font,
                    "size": r.size, "display": r.display, "origin": [r.origin.0, r.origin.1],
                    "invisible": r.render_mode == 3,
                })
            })
            .collect();
        d.reading.insert(page, runs);
        json!({ "width": size.width, "height": size.height, "rotation": size.rotation, "runs": listed })
    })
}

/// Every form field, as a JSON array (see the README for the shape).
///
/// # Safety
/// `doc` must be a live handle.
#[no_mangle]
pub unsafe extern "C" fn pdfcore_fields(doc: *mut PdfDoc) -> *mut c_char {
    let Some(d) = handle(doc) else { return bad_argument() };
    guard(|| {
        let fields: Vec<Value> = d
            .doc
            .fields()
            .iter()
            .map(|f| {
                json!({
                    "name": f.name, "kind": format!("{:?}", f.kind).to_lowercase(),
                    "value": f.value, "checked": f.checked,
                    "read_only": f.read_only, "required": f.required,
                    "multiline": f.multiline, "password": f.password, "comb": f.comb,
                    "editable": f.editable, "max_len": f.max_len,
                    "options": f.options.iter().map(|(e, l)| json!({"export": e, "label": l})).collect::<Vec<_>>(),
                    "widgets": f.widgets.iter().map(|w| json!({
                        "page": w.page, "display": w.display, "on_state": w.on_state,
                    })).collect::<Vec<_>>(),
                })
            })
            .collect();
        json!({ "fields": fields })
    })
}

// -- editing -----------------------------------------------------------------

fn edited(d: &mut PdfDoc) {
    d.reading.clear();
}

/// Replace a run's text. `allow_fallback` != 0 draws characters the document's
/// font lacks with a bundled font. Invalidates every run id.
///
/// # Safety
/// `doc` must be a live handle; `new_text` a NUL-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn pdfcore_replace_text(
    doc: *mut PdfDoc,
    page: c_int,
    run_id: c_int,
    new_text: *const c_char,
    allow_fallback: c_int,
) -> *mut c_char {
    let (Some(d), Some(new_text)) = (handle(doc), string_arg(new_text)) else { return bad_argument() };
    guard(|| {
        let page = page.max(0) as usize;
        let Some(run) = d.reading.get(&page).and_then(|r| r.get(run_id.max(0) as usize)).cloned() else {
            return json!({ "error": "ese texto ya no está donde se leyó: vuelve a leer la página", "kind": "stale_run" });
        };
        let result = if allow_fallback != 0 {
            d.doc.replace_text_with_fallback(page, &run, new_text)
        } else {
            d.doc.replace_text(page, &run, new_text)
        };
        match result {
            Ok(done) => {
                edited(d);
                json!({
                    "ok": true, "old_width": done.old_width, "new_width": done.new_width,
                    "fallback_chars": done.fallback_chars.iter().collect::<String>(),
                    "reread": done.reread,
                })
            }
            Err(e) => failure(&e),
        }
    })
}

/// Add text. `params` is JSON: `{"text","x","y"}` plus optional `"anchor"`
/// (`"top_left"`|`"baseline"`), `"size"`, `"color":[r,g,b]`, `"serif"`, `"bold"`,
/// `"italic"`, `"align"` (`"left"`|`"center"`|`"right"`), `"max_width"`.
///
/// # Safety
/// `doc` must be a live handle; `params` a NUL-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn pdfcore_add_text(doc: *mut PdfDoc, page: c_int, params: *const c_char) -> *mut c_char {
    let (Some(d), Some(params)) = (handle(doc), string_arg(params)) else { return bad_argument() };
    guard(|| {
        let Ok(p) = serde_json::from_str::<Value>(params) else {
            return json!({ "error": "los parámetros no son JSON válido", "kind": "bad_argument" });
        };
        let (Some(body), Some(x), Some(y)) = (p["text"].as_str(), p["x"].as_f64(), p["y"].as_f64()) else {
            return json!({ "error": "faltan text, x o y", "kind": "bad_argument" });
        };
        let mut new = NewText::new(body, x as f32, y as f32);
        new.anchor = if p["anchor"] == "baseline" { Anchor::Baseline } else { Anchor::TopLeft };
        if let Some(size) = p["size"].as_f64() {
            new.size = size as f32;
        }
        if let Some(c) = p["color"].as_array().filter(|c| c.len() == 3) {
            new.color = [0, 1, 2].map(|i| c[i].as_f64().unwrap_or(0.0) as f32);
        }
        new.serif = p["serif"].as_bool().unwrap_or(false);
        new.bold = p["bold"].as_bool().unwrap_or(false);
        new.italic = p["italic"].as_bool().unwrap_or(false);
        new.align = match p["align"].as_str() {
            Some("center") => Align::Center,
            Some("right") => Align::Right,
            _ => Align::Left,
        };
        new.max_width = p["max_width"].as_f64().map(|w| w as f32);
        match d.doc.add_text(page.max(0) as usize, &new) {
            Ok(added) => {
                edited(d);
                json!({ "ok": true, "display": added.display, "lines": added.lines })
            }
            Err(e) => failure(&e),
        }
    })
}

/// Fill a form field. `ignore_read_only` != 0 fills one the form marks read-only
/// (ask the person first).
///
/// # Safety
/// `doc` must be a live handle; the strings NUL-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn pdfcore_set_field(
    doc: *mut PdfDoc,
    name: *const c_char,
    value: *const c_char,
    ignore_read_only: c_int,
) -> *mut c_char {
    let (Some(d), Some(name), Some(value)) = (handle(doc), string_arg(name), string_arg(value)) else {
        return bad_argument();
    };
    guard(|| match d.doc.set_field_value_with(name, value, ignore_read_only != 0) {
        Ok(()) => {
            edited(d);
            json!({ "ok": true })
        }
        Err(e) => failure(&e),
    })
}

// -- history and saving ------------------------------------------------------

/// # Safety
/// `doc` must be a live handle.
#[no_mangle]
pub unsafe extern "C" fn pdfcore_undo(doc: *mut PdfDoc) -> *mut c_char {
    let Some(d) = handle(doc) else { return bad_argument() };
    guard(|| {
        let done = d.doc.undo();
        edited(d);
        json!({ "ok": done, "can_undo": d.doc.can_undo(), "can_redo": d.doc.can_redo() })
    })
}

/// # Safety
/// `doc` must be a live handle.
#[no_mangle]
pub unsafe extern "C" fn pdfcore_redo(doc: *mut PdfDoc) -> *mut c_char {
    let Some(d) = handle(doc) else { return bad_argument() };
    guard(|| {
        let done = d.doc.redo();
        edited(d);
        json!({ "ok": done, "can_undo": d.doc.can_undo(), "can_redo": d.doc.can_redo() })
    })
}

/// # Safety
/// `doc` must be a live handle.
#[no_mangle]
pub unsafe extern "C" fn pdfcore_history(doc: *mut PdfDoc) -> *mut c_char {
    let Some(d) = handle(doc) else { return bad_argument() };
    guard(|| json!({ "can_undo": d.doc.can_undo(), "can_redo": d.doc.can_redo() }))
}

/// The document as PDF bytes. On success returns a buffer of `*len` bytes the
/// caller frees with `pdfcore_free_bytes`; on failure returns null and `*len` is 0.
///
/// # Safety
/// `doc` must be a live handle; `len` a valid pointer.
#[no_mangle]
pub unsafe extern "C" fn pdfcore_save(doc: *mut PdfDoc, len: *mut usize) -> *mut u8 {
    if len.is_null() {
        return std::ptr::null_mut();
    }
    *len = 0;
    let Some(d) = handle(doc) else { return std::ptr::null_mut() };
    match catch_unwind(AssertUnwindSafe(|| d.doc.save_to_bytes())) {
        Ok(Ok(bytes)) => {
            let mut bytes = bytes.into_boxed_slice().into_vec();
            bytes.shrink_to_fit();
            *len = bytes.len();
            let ptr = bytes.as_mut_ptr();
            std::mem::forget(bytes);
            ptr
        }
        Ok(Err(e)) => {
            remember(&e.to_string());
            std::ptr::null_mut()
        }
        Err(_) => {
            remember("fallo interno del motor al guardar");
            std::ptr::null_mut()
        }
    }
}
