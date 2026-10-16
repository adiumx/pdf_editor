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

use pdfcore::{FlattenOptions, NewAnnotation, RedactOptions, Shape, BlockOptions, StyleChange, Substitution, Block, Align, Alignment, Anchor, Arrange, Document, Error, FieldRef, Item, ItemKind, NewImage, NewText, Reflow, ReplaceOptions, SearchOptions, TextRun, Warning};
use serde_json::{json, Value};

pub struct PdfDoc {
    doc: Document,
    /// What `pdfcore_page` last returned, so a run id can be turned back into
    /// the run it named.
    reading: HashMap<usize, Vec<TextRun>>,
    /// What `pdfcore_items` last returned for each page.
    items: HashMap<usize, Vec<Item>>,
    /// What `pdfcore_blocks` last returned for each page.
    blocks: HashMap<usize, Vec<Block>>,
    /// Which letters use the bundled font when the original lacks some (`pdfcore_set_substitution`).
    substitution: Substitution,
}

thread_local! {
    static LAST_ERROR: RefCell<(CString, CString)> = RefCell::new((CString::default(), CString::default()));
}

fn remember(message: &str, kind: &str) {
    let clean = |text: &str| CString::new(text.replace('\0', " ")).unwrap_or_default();
    LAST_ERROR.with(|e| *e.borrow_mut() = (clean(message), clean(kind)));
}

fn text(value: Value) -> *mut c_char {
    CString::new(value.to_string()).unwrap_or_default().into_raw()
}

fn failure(error: &Error) -> Value {
    let kind = match error {
        Error::Pdf(_) => "pdf",
        Error::PasswordRequired => "password_required",
        Error::WrongPassword => "wrong_password",
        Error::NoSuchPage(_) => "no_such_page",
        Error::MissingGlyphs(_) => "missing_glyphs",
        Error::NoSuchField(_) => "no_such_field",
        Error::NotEditable(_) => "not_editable",
        Error::Rejected(_) => "rejected",
        Error::Unsupported(_) => "unsupported",
        Error::StaleRun(_) => "stale_run",
    };
    let mut out = json!({ "error": error.to_string(), "kind": kind });
    if let Error::MissingGlyphs(chars) = error {
        out["chars"] = json!(chars.iter().collect::<String>());
    }
    // Why text cannot be edited: type3, vertical_writing, symbolic_font or rtl_text.
    if let Error::NotEditable(reason) = error {
        out["reason"] = json!(reason);
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
    open_with(data, len, "")
}

/// Like `pdfcore_open`, for a document protected by `password` (UTF-8, may be null).
/// On a null return `pdfcore_last_error_kind` says whether the password was
/// needed (`password_required`) or wrong (`wrong_password`).
///
/// # Safety
/// `data` must point to `len` readable bytes; `password` is null or a NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn pdfcore_open_with_password(data: *const u8, len: usize, password: *const c_char) -> *mut PdfDoc {
    open_with(data, len, string_arg(password).unwrap_or(""))
}

unsafe fn open_with(data: *const u8, len: usize, password: &str) -> *mut PdfDoc {
    if data.is_null() {
        remember("sin datos", "bad_argument");
        return std::ptr::null_mut();
    }
    let bytes = std::slice::from_raw_parts(data, len);
    match catch_unwind(|| Document::open_with_password(bytes, password)) {
        Ok(Ok(doc)) => Box::into_raw(Box::new(PdfDoc { doc, reading: HashMap::new(), items: HashMap::new(), blocks: HashMap::new(), substitution: Substitution::PerChar })),
        Ok(Err(e)) => {
            remember(&e.to_string(), failure(&e)["kind"].as_str().unwrap_or("pdf"));
            std::ptr::null_mut()
        }
        Err(_) => {
            remember("fallo interno del motor al abrir el PDF", "panic");
            std::ptr::null_mut()
        }
    }
}

/// Why the last open on this thread failed. Valid until the next call.
#[no_mangle]
pub extern "C" fn pdfcore_last_error() -> *const c_char {
    LAST_ERROR.with(|e| e.borrow().0.as_ptr())
}

/// The kind of that failure (`pdf`, `password_required`, `wrong_password`, `unsupported`, ...).
#[no_mangle]
pub extern "C" fn pdfcore_last_error_kind() -> *const c_char {
    LAST_ERROR.with(|e| e.borrow().1.as_ptr())
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
                let (substitute, similarity) = d.doc.substitute_for(r);
                json!({
                    "id": id, "text": r.text, "font": r.font, "base_font": r.base_font,
                    "substitute_font": substitute, "substitute_similarity": similarity,
                    "size": r.size, "display": r.display, "origin": [r.origin.0, r.origin.1],
                    "invisible": r.render_mode == 3,
                    "editable": r.not_editable.is_none(), "not_editable": r.not_editable,
                    "font_embedded": r.font_embedded, "font_kind": r.font_kind.as_str(),
                })
            })
            .collect();
        d.reading.insert(page, runs);
        json!({ "width": size.width, "height": size.height, "rotation": size.rotation, "runs": listed })
    })
}

/// The page's text as lines of words: `{"lines":[{"text","display","runs":[ids],"words":[{"text","display"}],"invisible"}]}`.
/// The run ids are those of `pdfcore_page` for the same page, which this call
/// reads again (so they stay valid for editing).
///
/// # Safety
/// `doc` must be a live handle.
#[no_mangle]
pub unsafe extern "C" fn pdfcore_lines(doc: *mut PdfDoc, page: c_int) -> *mut c_char {
    let Some(d) = handle(doc) else { return bad_argument() };
    guard(|| {
        let page = page.max(0) as usize;
        let runs = match d.doc.text_runs(page) {
            Ok(r) => r,
            Err(e) => return failure(&e),
        };
        let lines = pdfcore::text_lines_of(&runs);
        d.reading.insert(page, runs);
        let listed: Vec<Value> = lines
            .iter()
            .map(|l| {
                json!({
                    "text": l.text, "display": l.display, "runs": l.runs, "invisible": l.invisible,
                    "words": l.words.iter().map(|w| json!({ "text": w.text, "display": w.display })).collect::<Vec<_>>(),
                })
            })
            .collect();
        json!({ "lines": listed })
    })
}

/// The paragraph a run is a line of: `{"paragraph":null}` or
/// `{"paragraph":{"text","runs":[ids],"display"}}`. The text is the lines joined by spaces.
///
/// # Safety
/// `doc` must be a live handle.
#[no_mangle]
pub unsafe extern "C" fn pdfcore_paragraph(doc: *mut PdfDoc, page: c_int, run_id: c_int) -> *mut c_char {
    let Some(d) = handle(doc) else { return bad_argument() };
    guard(|| {
        let page = page.max(0) as usize;
        let Some(run) = d.reading.get(&page).and_then(|r| r.get(run_id.max(0) as usize)).cloned() else {
            return json!({ "error": "ese texto ya no está donde se leyó: vuelve a leer la página", "kind": "stale_run" });
        };
        match d.doc.paragraph_of(page, &run) {
            Ok(Some(p)) => json!({ "paragraph": { "text": p.text, "runs": p.runs, "display": p.display } }),
            Ok(None) => json!({ "paragraph": null }),
            Err(e) => failure(&e),
        }
    })
}

/// Lay the paragraph a run belongs to out again with `new_text` as its whole text,
/// within the width it has: `{"ok":true,"lines","added","removed","fallback_chars","warnings"}`.
///
/// # Safety
/// `doc` must be a live handle; `new_text` a NUL-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn pdfcore_replace_paragraph(
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
        match d.doc.replace_paragraph(page, &run, new_text, allow_fallback != 0) {
            Ok(done) => {
                edited(d);
                json!({
                    "ok": true, "lines": done.lines, "added": done.added, "removed": done.removed,
                    "fallback_chars": done.fallback_chars.iter().collect::<String>(),
                    "warnings": done.warnings.iter().map(warning_json).collect::<Vec<_>>(),
                })
            }
            Err(e) => failure(&e),
        }
    })
}

/// The paragraphs of a page, each with the style of every stretch of its text:
/// `{"blocks":[{"id","text","display","lines":[{"text","display","baseline"}],
/// "spans":[{"start","end","font","base_font","size","color","bold","italic"}],
/// "justified","first_line_indent","editable","reason"?}]}`. `start`/`end` count characters
/// of `text`. Ids are good until the next edit.
///
/// # Safety
/// `doc` must be a live handle.
#[no_mangle]
pub unsafe extern "C" fn pdfcore_blocks(doc: *mut PdfDoc, page: c_int) -> *mut c_char {
    let Some(d) = handle(doc) else { return bad_argument() };
    guard(|| {
        let page = page.max(0) as usize;
        let blocks = match d.doc.blocks(page) {
            Ok(b) => b,
            Err(e) => return failure(&e),
        };
        let listed: Vec<Value> = blocks
            .iter()
            .enumerate()
            .map(|(id, b)| {
                json!({
                    "id": id, "text": b.text, "display": b.display,
                    "lines": b.lines.iter().map(|l| json!({ "text": l.text, "display": l.display, "baseline": l.baseline })).collect::<Vec<_>>(),
                    "spans": b.spans.iter().map(|s| json!({
                        "start": s.start, "end": s.end, "font": s.font, "base_font": s.base_font,
                        "size": s.size, "color": s.color, "bold": s.bold, "italic": s.italic,
                    })).collect::<Vec<_>>(),
                    "justified": b.justified, "first_line_indent": b.first_line_indent,
                    "editable": b.not_editable.is_none(), "reason": b.not_editable,
                })
            })
            .collect();
        d.blocks.insert(page, blocks);
        json!({ "blocks": listed })
    })
}

/// Lay a block out again with `new_text` as its whole text, every letter keeping its style
/// (typed letters take the style of the one before): `{"ok":true,"lines","kept","rewritten",
/// "added","removed","fallback_chars","warnings"}`. Error kind `stale_block` when the block
/// was read before an edit.
///
/// # Safety
/// `doc` must be a live handle; `new_text` a NUL-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn pdfcore_replace_block(
    doc: *mut PdfDoc,
    page: c_int,
    block_id: c_int,
    new_text: *const c_char,
    allow_fallback: c_int,
) -> *mut c_char {
    let (Some(d), Some(new_text)) = (handle(doc), string_arg(new_text)) else { return bad_argument() };
    guard(|| {
        let page = page.max(0) as usize;
        let Some(block) = d.blocks.get(&page).and_then(|b| b.get(block_id.max(0) as usize)).cloned() else {
            return json!({ "error": "ese párrafo ya no está donde se leyó: vuelve a leer la página", "kind": "stale_block" });
        };
        match d.doc.replace_block(page, &block, new_text, allow_fallback != 0) {
            Ok(done) => {
                edited(d);
                block_edit_json(&done)
            }
            Err(e) => failure(&e),
        }
    })
}

/// What to do with a run that has letters its font lacks (and the caller allowed the bundled
/// font): 0 only those letters (default), 1 the whole run, 2 the whole run unless the bundled
/// face is a metric clone of the original. Returns `{"ok":true}`.
///
/// # Safety
/// `doc` must be a live handle.
#[no_mangle]
pub unsafe extern "C" fn pdfcore_set_substitution(doc: *mut PdfDoc, policy: c_int) -> *mut c_char {
    let Some(d) = handle(doc) else { return bad_argument() };
    guard(|| {
        d.substitution = match policy {
            1 => Substitution::WholeRun,
            2 => Substitution::Auto,
            _ => Substitution::PerChar,
        };
        json!({ "ok": true })
    })
}

/// The fonts the document uses and what editing them needs: `{"fonts":[{"base_font","kind",
/// "pages","runs","needed","registered","bundled","similarity","source":null|{"name","license","url"}}]}`.
/// `needed` is true for fonts that are not embedded or only embedded in part: the real font is
/// wanted to edit them exactly (`pdfcore_register_font`).
///
/// # Safety
/// `doc` must be a live handle.
#[no_mangle]
pub unsafe extern "C" fn pdfcore_required_fonts(doc: *mut PdfDoc) -> *mut c_char {
    let Some(d) = handle(doc) else { return bad_argument() };
    guard(|| match d.doc.required_fonts() {
        Ok(fonts) => json!({
            "fonts": fonts.iter().map(|f| json!({
                "base_font": f.base_font, "kind": f.class.as_str(), "pages": f.pages, "runs": f.runs,
                "needed": f.needed, "registered": f.registered, "bundled": f.bundled, "similarity": f.similarity,
                "source": f.source.as_ref().map(|s| json!({ "name": s.name, "license": s.license, "url": s.url })),
            })).collect::<Vec<_>>()
        }),
        Err(e) => failure(&e),
    })
}

/// Hand the engine a TrueType font the person may use, for the letters other fonts of its family
/// lack: `{"ok":true,"family","bold","italic"}`. Error kinds: `unsupported` (not TrueType, not
/// a font), `rejected` (the licence forbids embedding, too big, too many).
///
/// # Safety
/// `doc` must be a live handle; `data` must point to `len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn pdfcore_register_font(doc: *mut PdfDoc, data: *const u8, len: usize) -> *mut c_char {
    let Some(d) = handle(doc) else { return bad_argument() };
    if data.is_null() {
        return bad_argument();
    }
    let bytes = std::slice::from_raw_parts(data, len);
    guard(|| match d.doc.register_font(bytes) {
        Ok(f) => json!({ "ok": true, "family": f.family, "bold": f.bold, "italic": f.italic }),
        Err(e) => failure(&e),
    })
}

fn block_options(p: &Value) -> BlockOptions {
    BlockOptions {
        allow_fallback: p.get("allow_fallback").and_then(Value::as_bool).unwrap_or(false),
        justify: p.get("justify").and_then(Value::as_bool),
        push_down: p.get("push_down").and_then(Value::as_bool).unwrap_or(false),
        width: p.get("width").and_then(Value::as_f64).map(|w| w as f32),
    }
}

fn block_edit_json(done: &pdfcore::BlockEdit) -> Value {
    json!({
        "ok": true, "lines": done.lines, "kept": done.kept, "rewritten": done.rewritten,
        "added": done.added, "removed": done.removed,
        "fallback_chars": done.fallback_chars.iter().collect::<String>(),
        "substitutes": substitutes_json(&done.substitutes),
        "warnings": done.warnings.iter().map(warning_json).collect::<Vec<_>>(),
    })
}

/// The block `block_id` of the last `pdfcore_blocks` reading of `page`, or the stale error.
fn block_of(d: &PdfDoc, page: usize, block_id: c_int) -> std::result::Result<Block, Value> {
    d.blocks.get(&page).and_then(|b| b.get(block_id.max(0) as usize)).cloned().ok_or_else(|| {
        json!({ "error": "ese párrafo ya no está donde se leyó: vuelve a leer la página", "kind": "stale_block" })
    })
}

fn bad_params() -> Value {
    json!({ "error": "parámetros no válidos", "kind": "bad_argument" })
}

/// As `pdfcore_replace_block`, with options as JSON: `{"allow_fallback":bool,"justify":null|bool,
/// "push_down":bool}`. `push_down` moves what is below the block down by the room added lines take
/// (what would not fit on the page stays and a `below_page` warning says so). `justify` forces
/// the lines justified (`true`) or ragged (`false`).
///
/// # Safety
/// `doc` must be a live handle; `new_text` and `params` NUL-terminated UTF-8 strings.
#[no_mangle]
pub unsafe extern "C" fn pdfcore_replace_block_ex(
    doc: *mut PdfDoc,
    page: c_int,
    block_id: c_int,
    new_text: *const c_char,
    params: *const c_char,
) -> *mut c_char {
    let (Some(d), Some(new_text), Some(params)) = (handle(doc), string_arg(new_text), string_arg(params)) else { return bad_argument() };
    guard(|| {
        let Ok(p) = serde_json::from_str::<Value>(params) else { return bad_params() };
        let page = page.max(0) as usize;
        let block = match block_of(d, page, block_id) {
            Ok(b) => b,
            Err(e) => return e,
        };
        match d.doc.replace_block_with(page, &block, new_text, block_options(&p)) {
            Ok(done) => {
                edited(d);
                block_edit_json(&done)
            }
            Err(e) => failure(&e),
        }
    })
}

/// Replace the letters `[start, end)` (characters of the block's `text`) with `new_text`, which takes
/// the style of the first letter it replaces. `params` as in `pdfcore_replace_block_ex`.
///
/// # Safety
/// `doc` must be a live handle; `new_text` and `params` NUL-terminated UTF-8 strings.
#[no_mangle]
pub unsafe extern "C" fn pdfcore_replace_range(
    doc: *mut PdfDoc,
    page: c_int,
    block_id: c_int,
    start: c_int,
    end: c_int,
    new_text: *const c_char,
    params: *const c_char,
) -> *mut c_char {
    let (Some(d), Some(new_text), Some(params)) = (handle(doc), string_arg(new_text), string_arg(params)) else { return bad_argument() };
    guard(|| {
        let Ok(p) = serde_json::from_str::<Value>(params) else { return bad_params() };
        let page = page.max(0) as usize;
        let block = match block_of(d, page, block_id) {
            Ok(b) => b,
            Err(e) => return e,
        };
        match d.doc.replace_range(page, &block, start.max(0) as usize, end.max(0) as usize, new_text, block_options(&p)) {
            Ok(done) => {
                edited(d);
                block_edit_json(&done)
            }
            Err(e) => failure(&e),
        }
    })
}

/// Change the style of the letters `[start, end)` of a block: `params` has `bold`, `italic` (bool),
/// `size` (points), `color` ([r,g,b] in 0..1), all optional, plus the options of
/// `pdfcore_replace_block_ex`. Bold/italic use the document's own font of that face when the page
/// has one; otherwise a bundled one, which needs `allow_fallback` (`rejected` without it).
///
/// # Safety
/// `doc` must be a live handle; `params` a NUL-terminated UTF-8 JSON string.
#[no_mangle]
pub unsafe extern "C" fn pdfcore_set_block_style(
    doc: *mut PdfDoc,
    page: c_int,
    block_id: c_int,
    start: c_int,
    end: c_int,
    params: *const c_char,
) -> *mut c_char {
    let (Some(d), Some(params)) = (handle(doc), string_arg(params)) else { return bad_argument() };
    guard(|| {
        let Ok(p) = serde_json::from_str::<Value>(params) else { return bad_params() };
        let page = page.max(0) as usize;
        let block = match block_of(d, page, block_id) {
            Ok(b) => b,
            Err(e) => return e,
        };
        let color = p.get("color").and_then(Value::as_array).and_then(|c| {
            let v: Vec<f32> = c.iter().filter_map(|x| x.as_f64().map(|f| f as f32)).collect();
            (v.len() == 3).then(|| [v[0], v[1], v[2]])
        });
        let change = StyleChange {
            bold: p.get("bold").and_then(Value::as_bool),
            italic: p.get("italic").and_then(Value::as_bool),
            size: p.get("size").and_then(Value::as_f64).map(|v| v as f32),
            color,
        };
        match d.doc.set_style(page, &block, start.max(0) as usize, end.max(0) as usize, change, block_options(&p)) {
            Ok(done) => {
                edited(d);
                block_edit_json(&done)
            }
            Err(e) => failure(&e),
        }
    })
}

/// Where every character of a block's text is: `{"boxes":[[x0,y0,x1,y1],...]}`, one per character
/// of `text` as a reader shows the page. For placing a caret or a selection; the space that joins
/// two lines is an empty box at the end of the first.
///
/// # Safety
/// `doc` must be a live handle.
#[no_mangle]
pub unsafe extern "C" fn pdfcore_block_chars(doc: *mut PdfDoc, page: c_int, block_id: c_int) -> *mut c_char {
    let Some(d) = handle(doc) else { return bad_argument() };
    guard(|| match block_of(d, page.max(0) as usize, block_id) {
        Ok(b) => json!({ "boxes": b.char_boxes }),
        Err(e) => e,
    })
}

/// The item at a point of the page (as a reader shows it): `{"hit":null}` or `{"hit":{"kind",
/// "id","display","name"?,"widget"?,"block"?,"char"?,"caret"?}}`. A field is hit before text and text
/// before a picture. For text, `block` is the id of its block, `char` the letter under the point
/// and `caret` the insertion point (0..=length) nearest to it. This reads the page's items and
/// blocks again, so the ids it returns are the current ones.
///
/// # Safety
/// `doc` must be a live handle.
#[no_mangle]
pub unsafe extern "C" fn pdfcore_hit_test(doc: *mut PdfDoc, page: c_int, x: f32, y: f32) -> *mut c_char {
    let Some(d) = handle(doc) else { return bad_argument() };
    guard(|| {
        let page = page.max(0) as usize;
        let (items, blocks) = match (d.doc.page_items(page), d.doc.blocks(page)) {
            (Ok(i), Ok(b)) => (i, b),
            (Err(e), _) | (_, Err(e)) => return failure(&e),
        };
        let hit = match d.doc.hit_test(page, x, y) {
            Ok(h) => h,
            Err(e) => return failure(&e),
        };
        d.items.insert(page, items);
        d.blocks.insert(page, blocks);
        match hit {
            None => json!({ "hit": null }),
            Some(h) => {
                let mut out = json!({
                    "kind": match h.kind { ItemKind::Text => "text", ItemKind::Image => "image", ItemKind::Field => "field" },
                    "id": h.item, "display": h.display,
                });
                if let Some((name, widget)) = &h.field {
                    out["name"] = json!(name);
                    out["widget"] = json!(widget);
                }
                out["block"] = json!(h.block);
                out["char"] = json!(h.char_index);
                out["caret"] = json!(h.caret);
                json!({ "hit": out })
            }
        }
    })
}

/// Take items off the page: `{"ids":[..]}` of the last `pdfcore_items`. Text stops drawing,
/// pictures are cancelled, a field's widget leaves the page and the form. One undo step.
///
/// # Safety
/// `doc` must be a live handle; `params` a NUL-terminated JSON string.
#[no_mangle]
pub unsafe extern "C" fn pdfcore_delete_items(doc: *mut PdfDoc, page: c_int, params: *const c_char) -> *mut c_char {
    let (Some(d), Some(params)) = (handle(doc), string_arg(params)) else { return bad_argument() };
    guard(|| {
        let page = page.max(0) as usize;
        let Ok(p) = serde_json::from_str::<Value>(params) else { return bad_params() };
        let items = match chosen(d, page, p.get("ids").unwrap_or(&Value::Null)) {
            Ok(i) => i,
            Err(e) => return e,
        };
        let result = d.doc.delete_items(page, &items);
        finished(d, result)
    })
}

/// Resize an item of the last `pdfcore_items` to the box `[x0,y0,x1,y1]` (as a reader shows the
/// page): a picture is scaled, a field gets the box, text is laid out again within the width and
/// moved to the corner (its height follows from the text).
///
/// # Safety
/// `doc` must be a live handle.
#[no_mangle]
pub unsafe extern "C" fn pdfcore_resize_item(doc: *mut PdfDoc, page: c_int, id: c_int, x0: f32, y0: f32, x1: f32, y1: f32) -> *mut c_char {
    let Some(d) = handle(doc) else { return bad_argument() };
    guard(|| {
        let page = page.max(0) as usize;
        let items = match chosen(d, page, &json!([id])) {
            Ok(i) => i,
            Err(e) => return e,
        };
        let result = d.doc.resize_item(page, &items[0], [x0, y0, x1, y1]);
        finished(d, result)
    })
}

/// Build the look of the form's fields again from their values (`only_missing` != 0: only fields
/// with a widget that has none): `{"ok":true,"rebuilt":n,"skipped":n}`.
///
/// # Safety
/// `doc` must be a live handle.
#[no_mangle]
pub unsafe extern "C" fn pdfcore_refresh_appearances(doc: *mut PdfDoc, only_missing: c_int) -> *mut c_char {
    let Some(d) = handle(doc) else { return bad_argument() };
    guard(|| match d.doc.refresh_appearances(only_missing != 0) {
        Ok(done) => {
            edited(d);
            json!({ "ok": true, "rebuilt": done.rebuilt, "skipped": done.skipped })
        }
        Err(e) => failure(&e),
    })
}

/// One edit of a batch, with what it points to as it was when the batch began.
enum Step {
    ReplaceText { page: usize, run: TextRun, text: String, options: ReplaceOptions },
    ReplaceBlock { page: usize, block: Block, text: String, options: BlockOptions },
    ReplaceRange { page: usize, block: Block, start: usize, end: usize, text: String, options: BlockOptions },
    SetStyle { page: usize, block: Block, start: usize, end: usize, change: StyleChange, options: BlockOptions },
    Move { page: usize, moves: Vec<(Item, (f32, f32))> },
    Delete { page: usize, items: Vec<Item> },
    Resize { page: usize, item: Item, display: [f32; 4] },
    SetField { name: String, value: String },
    AddText { page: usize, params: Value },
    DeleteRuns { page: usize, runs: Vec<TextRun> },
}

fn gone(what: &str) -> Error {
    Error::Rejected(format!("{what} ya no está donde se leyó: en un lote, cada operación debe apuntar a algo distinto"))
}

fn same_place(a: (f32, f32), b: (f32, f32)) -> bool {
    (a.0 - b.0).abs() < 0.5 && (a.1 - b.1).abs() < 0.5
}

/// The run now at the place and with the text `old` had.
fn fresh_run(doc: &Document, page: usize, old: &TextRun) -> pdfcore::Result<TextRun> {
    doc.text_runs(page)?
        .into_iter()
        .find(|r| r.xobject == old.xobject && r.text == old.text && same_place(r.origin, old.origin))
        .ok_or_else(|| gone("ese texto"))
}

fn fresh_block(doc: &Document, page: usize, old: &Block) -> pdfcore::Result<Block> {
    let first = &old.runs[0];
    doc.blocks(page)?
        .into_iter()
        .find(|b| b.runs.first().is_some_and(|r| r.xobject == first.xobject && r.text == first.text && same_place(r.origin, first.origin)))
        .ok_or_else(|| gone("ese párrafo"))
}

fn fresh_item(doc: &Document, page: usize, old: &Item) -> pdfcore::Result<Item> {
    doc.page_items(page)?
        .into_iter()
        .find(|it| {
            it.kind == old.kind
                && match old.kind {
                    ItemKind::Text => old.runs.first().zip(it.runs.first()).is_some_and(|(a, b)| a.text == b.text && same_place(a.origin, b.origin)),
                    ItemKind::Image => old.picture.as_ref().zip(it.picture.as_ref()).is_some_and(|(a, b)| a.name == b.name && a.display.iter().zip(b.display).all(|(x, y)| (x - y).abs() < 0.5)),
                    ItemKind::Field => it.field == old.field,
                }
        })
        .ok_or_else(|| gone("ese elemento"))
}

/// Several edits as ONE undo step; if any fails, none is applied. `params` is `{"ops":[...]}` and
/// each op has `"op"` and `"page"` and points to ids of the last readings (`pdfcore_page`,
/// `pdfcore_blocks`, `pdfcore_items`), all as they were when the batch began:
/// `replace_text` (`run`, `text`, `allow_fallback`, `reflow`), `replace_block` (`block`, `text` and the
/// options of `pdfcore_replace_block_ex`), `replace_range` (+ `start`, `end`), `set_block_style`
/// (`block`, `start`, `end`, `bold`, `italic`, `size`, `color`), `move_items` (`moves`),
/// `delete_items` (`ids`), `resize_item` (`id`, `box`), `set_field` (`name`, `value`), `add_text` (the
/// parameters of `pdfcore_add_text`), `delete_runs` (`runs`: ids of the last `pdfcore_page`). The ops
/// must point at different things (what one changes the next cannot find again; the batch then
/// fails whole). Returns `{"ok":true,"results":[...]}`, one entry per op.
///
/// # Safety
/// `doc` must be a live handle; `params` a NUL-terminated JSON string.
#[no_mangle]
pub unsafe extern "C" fn pdfcore_batch(doc: *mut PdfDoc, params: *const c_char) -> *mut c_char {
    let (Some(d), Some(params)) = (handle(doc), string_arg(params)) else { return bad_argument() };
    guard(|| {
        let Ok(p) = serde_json::from_str::<Value>(params) else { return bad_params() };
        let Some(ops) = p.get("ops").and_then(Value::as_array) else { return bad_params() };
        let mut steps: Vec<Step> = Vec::new();
        for op in ops {
            let page = op.get("page").and_then(Value::as_u64).unwrap_or(0) as usize;
            let id = |key: &str| op.get(key).and_then(Value::as_i64).unwrap_or(-1) as c_int;
            let text = op.get("text").and_then(Value::as_str).unwrap_or("").to_string();
            let count = |key: &str| op.get(key).and_then(Value::as_u64).unwrap_or(0) as usize;
            let step = match op.get("op").and_then(Value::as_str).unwrap_or("") {
                "replace_text" => {
                    let Some(run) = d.reading.get(&page).and_then(|r| r.get(id("run").max(0) as usize)).cloned() else {
                        return json!({ "error": "ese texto ya no está donde se leyó: vuelve a leer la página", "kind": "stale_run" });
                    };
                    let reflow = match op.get("reflow").and_then(Value::as_i64).unwrap_or(1) {
                        0 => Reflow::None,
                        2 => Reflow::Centered,
                        3 => Reflow::RightAligned,
                        _ => Reflow::Following,
                    };
                    let options = ReplaceOptions {
                        allow_fallback: op.get("allow_fallback").and_then(Value::as_bool).unwrap_or(false),
                        reflow,
                        substitution: d.substitution,
                    };
                    Step::ReplaceText { page, run, text, options }
                }
                kind @ ("replace_block" | "replace_range" | "set_block_style") => {
                    let block = match block_of(d, page, id("block")) {
                        Ok(b) => b,
                        Err(e) => return e,
                    };
                    let options = block_options(op);
                    match kind {
                        "replace_block" => Step::ReplaceBlock { page, block, text, options },
                        "replace_range" => Step::ReplaceRange { page, block, start: count("start"), end: count("end"), text, options },
                        _ => {
                            let color = op.get("color").and_then(Value::as_array).and_then(|c| {
                                let v: Vec<f32> = c.iter().filter_map(|x| x.as_f64().map(|f| f as f32)).collect();
                                (v.len() == 3).then(|| [v[0], v[1], v[2]])
                            });
                            let change = StyleChange {
                                bold: op.get("bold").and_then(Value::as_bool),
                                italic: op.get("italic").and_then(Value::as_bool),
                                size: op.get("size").and_then(Value::as_f64).map(|v| v as f32),
                                color,
                            };
                            Step::SetStyle { page, block, start: count("start"), end: count("end"), change, options }
                        }
                    }
                }
                "move_items" => {
                    let Some(list) = op.get("moves").and_then(Value::as_array) else { return bad_params() };
                    let mut moves = Vec::new();
                    for m in list {
                        let items = match chosen(d, page, &json!([m.get("id")])) {
                            Ok(i) => i,
                            Err(e) => return e,
                        };
                        let num = |k: &str| m.get(k).and_then(Value::as_f64).unwrap_or(0.0) as f32;
                        moves.push((items[0].clone(), (num("dx"), num("dy"))));
                    }
                    Step::Move { page, moves }
                }
                "delete_items" => match chosen(d, page, op.get("ids").unwrap_or(&Value::Null)) {
                    Ok(items) => Step::Delete { page, items },
                    Err(e) => return e,
                },
                "resize_item" => {
                    let items = match chosen(d, page, &json!([op.get("id")])) {
                        Ok(i) => i,
                        Err(e) => return e,
                    };
                    let b: Vec<f32> = op.get("box").and_then(Value::as_array).map(|a| a.iter().filter_map(|x| x.as_f64().map(|f| f as f32)).collect()).unwrap_or_default();
                    if b.len() != 4 {
                        return bad_params();
                    }
                    Step::Resize { page, item: items[0].clone(), display: [b[0], b[1], b[2], b[3]] }
                }
                "add_text" => {
                    if new_text_of(op).is_none() {
                        return json!({ "error": "faltan text, x o y", "kind": "bad_argument" });
                    }
                    Step::AddText { page, params: op.clone() }
                }
                "delete_runs" => {
                    let Some(ids) = op.get("runs").and_then(Value::as_array) else { return bad_params() };
                    let mut runs = Vec::new();
                    for id in ids {
                        match id.as_u64().and_then(|n| d.reading.get(&page).and_then(|r| r.get(n as usize))) {
                            Some(run) => runs.push(run.clone()),
                            None => return json!({ "error": "ese texto ya no está donde se leyó: vuelve a leer la página", "kind": "stale_run" }),
                        }
                    }
                    Step::DeleteRuns { page, runs }
                }
                "set_field" => Step::SetField { name: op.get("name").and_then(Value::as_str).unwrap_or("").to_string(), value: op.get("value").and_then(Value::as_str).unwrap_or("").to_string() },
                _ => return json!({ "error": "operación de lote desconocida", "kind": "bad_argument" }),
            };
            steps.push(step);
        }
        let outcome = d.doc.batch(|doc| {
            let mut results: Vec<Value> = Vec::new();
            for step in &steps {
                results.push(match step {
                    Step::ReplaceText { page, run, text, options } => {
                        let run = fresh_run(doc, *page, run)?;
                        let done = doc.replace_text_with(*page, &run, text, *options)?;
                        json!({ "ok": true, "old_width": done.old_width, "new_width": done.new_width, "substitutes": substitutes_json(&done.substitutes),
                                "warnings": done.warnings.iter().map(warning_json).collect::<Vec<_>>() })
                    }
                    Step::ReplaceBlock { page, block, text, options } => {
                        let block = fresh_block(doc, *page, block)?;
                        block_edit_json(&doc.replace_block_with(*page, &block, text, *options)?)
                    }
                    Step::ReplaceRange { page, block, start, end, text, options } => {
                        let block = fresh_block(doc, *page, block)?;
                        block_edit_json(&doc.replace_range(*page, &block, *start, *end, text, *options)?)
                    }
                    Step::SetStyle { page, block, start, end, change, options } => {
                        let block = fresh_block(doc, *page, block)?;
                        block_edit_json(&doc.set_style(*page, &block, *start, *end, *change, *options)?)
                    }
                    Step::Move { page, moves } => {
                        let mut fresh = Vec::new();
                        for (item, delta) in moves {
                            fresh.push((fresh_item(doc, *page, item)?, *delta));
                        }
                        doc.move_items(*page, &fresh)?;
                        json!({ "ok": true })
                    }
                    Step::Delete { page, items } => {
                        let mut fresh = Vec::new();
                        for item in items {
                            fresh.push(fresh_item(doc, *page, item)?);
                        }
                        doc.delete_items(*page, &fresh)?;
                        json!({ "ok": true })
                    }
                    Step::Resize { page, item, display } => {
                        let item = fresh_item(doc, *page, item)?;
                        doc.resize_item(*page, &item, *display)?;
                        json!({ "ok": true })
                    }
                    Step::SetField { name, value } => {
                        doc.set_field_value(name, value)?;
                        json!({ "ok": true })
                    }
                    Step::AddText { page, params } => {
                        let new = new_text_of(params).ok_or_else(|| Error::Rejected("faltan text, x o y".into()))?;
                        let added = doc.add_text(*page, &new)?;
                        json!({ "ok": true, "display": added.display, "lines": added.lines })
                    }
                    Step::DeleteRuns { page, runs } => {
                        // Found again by what they say and where (what the batch did before may have
                        // renumbered them).
                        let now = doc.text_runs(*page)?;
                        let mut ids = Vec::new();
                        for old in runs {
                            let at = (0..now.len())
                                .find(|i| {
                                    let r = &now[*i];
                                    !ids.contains(i) && r.xobject == old.xobject && r.text == old.text && same_place(r.origin, old.origin)
                                })
                                .ok_or_else(|| Error::StaleRun("ese texto ya no está donde se leyó".into()))?;
                            ids.push(at);
                        }
                        let done = doc.delete_runs(*page, &ids)?;
                        json!({ "ok": true, "runs": done.runs, "chars": done.chars })
                    }
                });
            }
            Ok(results)
        });
        match outcome {
            Ok(results) => {
                edited(d);
                json!({ "ok": true, "results": results })
            }
            Err(e) => {
                edited(d);
                failure(&e)
            }
        }
    })
}

fn point_of(v: Option<&Value>) -> Option<(f32, f32)> {
    let a = v?.as_array()?;
    Some((a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32))
}

fn rect_of(v: Option<&Value>) -> Option<[f32; 4]> {
    let a = v?.as_array()?;
    if a.len() != 4 {
        return None;
    }
    let n: Vec<f32> = a.iter().filter_map(|x| x.as_f64().map(|f| f as f32)).collect();
    (n.len() == 4).then(|| [n[0], n[1], n[2], n[3]])
}

fn color_of_json(v: Option<&Value>) -> Option<[f32; 3]> {
    let a = v?.as_array()?;
    let n: Vec<f32> = a.iter().filter_map(|x| x.as_f64().map(|f| f as f32)).collect();
    (n.len() == 3).then(|| [n[0], n[1], n[2]])
}

/// The annotations of a page: `{"annotations":[{"index","kind","subtype","display","contents",
/// "author","color","quads","has_appearance","hidden"}]}`. `kind` is `highlight`, `underline`,
/// `strike_out`, `squiggly`, `note`, `free_text`, `ink`, `square`, `circle`, `line`, `link`, `stamp`,
/// `popup`, `widget` or `other`. Indexes are good until the next edit.
///
/// # Safety
/// `doc` must be a live handle.
#[no_mangle]
pub unsafe extern "C" fn pdfcore_annotations(doc: *mut PdfDoc, page: c_int) -> *mut c_char {
    let Some(d) = handle(doc) else { return bad_argument() };
    guard(|| match d.doc.annotations(page.max(0) as usize) {
        Ok(list) => json!({
            "annotations": list.iter().map(|a| json!({
                "index": a.index, "kind": a.kind.as_str(), "subtype": a.subtype, "display": a.display,
                "contents": a.contents, "author": a.author, "color": a.color, "quads": a.quads,
                "has_appearance": a.has_appearance, "hidden": a.hidden,
            })).collect::<Vec<_>>()
        }),
        Err(e) => failure(&e),
    })
}

/// Add an annotation: `{"shape":"highlight"|"underline"|"strike_out"|"squiggly" (+"boxes":[[x0,y0,x1,y1]..]),
/// "note" (+"at":[x,y]), "free_text" (+"rect","size"), "ink" (+"strokes":[[[x,y]..]..],"width"),
/// "square"|"circle" (+"rect","fill":[r,g,b]?,"width"), "line" (+"from","to","width"),
/// "color":[r,g,b],"contents","author","opacity"}` in points as a reader shows the page.
/// Returns `{"ok":true,"index"}`; free text the font cannot draw is `missing_glyphs`.
///
/// # Safety
/// `doc` must be a live handle; `params` a NUL-terminated JSON string.
#[no_mangle]
pub unsafe extern "C" fn pdfcore_add_annotation(doc: *mut PdfDoc, page: c_int, params: *const c_char) -> *mut c_char {
    let (Some(d), Some(params)) = (handle(doc), string_arg(params)) else { return bad_argument() };
    guard(|| {
        let Ok(p) = serde_json::from_str::<Value>(params) else { return bad_params() };
        let boxes = || -> Vec<[f32; 4]> { p.get("boxes").and_then(Value::as_array).map(|a| a.iter().filter_map(|b| rect_of(Some(b))).collect()).unwrap_or_default() };
        let width = p.get("width").and_then(Value::as_f64).unwrap_or(1.5) as f32;
        let fill = color_of_json(p.get("fill"));
        let shape = match p.get("shape").and_then(Value::as_str).unwrap_or("") {
            "highlight" => Shape::Highlight(boxes()),
            "underline" => Shape::Underline(boxes()),
            "strike_out" => Shape::StrikeOut(boxes()),
            "squiggly" => Shape::Squiggly(boxes()),
            "note" => match point_of(p.get("at")) {
                Some(at) => Shape::Note { at },
                None => return bad_params(),
            },
            "free_text" => match rect_of(p.get("rect")) {
                Some(rect) => Shape::FreeText { rect, size: p.get("size").and_then(Value::as_f64).unwrap_or(12.0) as f32 },
                None => return bad_params(),
            },
            "ink" => {
                let strokes: Vec<Vec<(f32, f32)>> = p
                    .get("strokes")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().map(|s| s.as_array().map(|pts| pts.iter().filter_map(|pt| point_of(Some(pt))).collect()).unwrap_or_default()).collect())
                    .unwrap_or_default();
                Shape::Ink { strokes, width }
            }
            "square" | "circle" => match rect_of(p.get("rect")) {
                Some(rect) if p.get("shape").and_then(Value::as_str) == Some("square") => Shape::Square { rect, fill, width },
                Some(rect) => Shape::Circle { rect, fill, width },
                None => return bad_params(),
            },
            "line" => match (point_of(p.get("from")), point_of(p.get("to"))) {
                (Some(from), Some(to)) => Shape::Line { from, to, width },
                _ => return bad_params(),
            },
            _ => return json!({ "error": "forma de anotación desconocida", "kind": "bad_argument" }),
        };
        let new = NewAnnotation {
            shape,
            color: color_of_json(p.get("color")).unwrap_or([1.0, 0.9, 0.2]),
            contents: p.get("contents").and_then(Value::as_str).unwrap_or("").to_string(),
            author: p.get("author").and_then(Value::as_str).unwrap_or("").to_string(),
            opacity: p.get("opacity").and_then(Value::as_f64).unwrap_or(1.0) as f32,
        };
        match d.doc.add_annotation(page.max(0) as usize, &new) {
            Ok(index) => {
                edited(d);
                json!({ "ok": true, "index": index })
            }
            Err(e) => failure(&e),
        }
    })
}

/// Remove the annotation at `index` of the page's list (and its pop-up).
///
/// # Safety
/// `doc` must be a live handle.
#[no_mangle]
pub unsafe extern "C" fn pdfcore_delete_annotation(doc: *mut PdfDoc, page: c_int, index: c_int) -> *mut c_char {
    let Some(d) = handle(doc) else { return bad_argument() };
    guard(|| {
        let result = d.doc.delete_annotation(page.max(0) as usize, index.max(0) as usize);
        finished(d, result)
    })
}

/// Change what an annotation says (a free-text box is drawn again).
///
/// # Safety
/// `doc` must be a live handle; `contents` a NUL-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn pdfcore_set_annotation_contents(doc: *mut PdfDoc, page: c_int, index: c_int, contents: *const c_char) -> *mut c_char {
    let (Some(d), Some(contents)) = (handle(doc), string_arg(contents)) else { return bad_argument() };
    guard(|| {
        let result = d.doc.set_annotation_contents(page.max(0) as usize, index.max(0) as usize, contents);
        finished(d, result)
    })
}

/// Move an annotation by `(dx, dy)` points as a reader shows the page.
///
/// # Safety
/// `doc` must be a live handle.
#[no_mangle]
pub unsafe extern "C" fn pdfcore_move_annotation(doc: *mut PdfDoc, page: c_int, index: c_int, dx: f32, dy: f32) -> *mut c_char {
    let Some(d) = handle(doc) else { return bad_argument() };
    guard(|| {
        let result = d.doc.move_annotation(page.max(0) as usize, index.max(0) as usize, dx, dy);
        finished(d, result)
    })
}

/// Draw annotations and/or form fields into their pages and remove them: `{"pages":[..]|null,
/// "annotations":bool,"forms":bool}` (both default true). Returns `{"ok":true,"drawn","removed",
/// "skipped"}`. Signature fields are never flattened (`skipped`). A signed document stops being valid.
///
/// # Safety
/// `doc` must be a live handle; `params` a NUL-terminated JSON string.
#[no_mangle]
pub unsafe extern "C" fn pdfcore_flatten(doc: *mut PdfDoc, params: *const c_char) -> *mut c_char {
    let (Some(d), Some(params)) = (handle(doc), string_arg(params)) else { return bad_argument() };
    guard(|| {
        let Ok(p) = serde_json::from_str::<Value>(params) else { return bad_params() };
        let pages: Option<Vec<usize>> = p.get("pages").and_then(Value::as_array).map(|a| a.iter().filter_map(|n| n.as_u64().map(|n| n as usize)).collect());
        let options = FlattenOptions {
            annotations: p.get("annotations").and_then(Value::as_bool).unwrap_or(true),
            forms: p.get("forms").and_then(Value::as_bool).unwrap_or(true),
        };
        match d.doc.flatten(pages.as_deref(), options) {
            Ok(done) => {
                edited(d);
                json!({ "ok": true, "drawn": done.drawn, "removed": done.removed, "skipped": done.skipped })
            }
            Err(e) => failure(&e),
        }
    })
}

/// Take the text under boxes out of the file and cover the place: `{"boxes":[[x0,y0,x1,y1]..],
/// "fill":[r,g,b],"remove_images":bool}`. Returns `{"ok":true,"chars","runs","images_removed",
/// "images_kept","annotations_removed"}`. Text inside a form XObject refuses the whole redaction
/// (`unsupported`). Pictures that touch a box stay (covered) unless `remove_images`.
///
/// # Safety
/// `doc` must be a live handle; `params` a NUL-terminated JSON string.
#[no_mangle]
pub unsafe extern "C" fn pdfcore_redact(doc: *mut PdfDoc, page: c_int, params: *const c_char) -> *mut c_char {
    let (Some(d), Some(params)) = (handle(doc), string_arg(params)) else { return bad_argument() };
    guard(|| {
        let Ok(p) = serde_json::from_str::<Value>(params) else { return bad_params() };
        let boxes: Vec<[f32; 4]> = p.get("boxes").and_then(Value::as_array).map(|a| a.iter().filter_map(|b| rect_of(Some(b))).collect()).unwrap_or_default();
        let options = RedactOptions {
            fill: color_of_json(p.get("fill")).unwrap_or([0.0, 0.0, 0.0]),
            remove_images: p.get("remove_images").and_then(Value::as_bool).unwrap_or(false),
        };
        match d.doc.redact(page.max(0) as usize, &boxes, &options) {
            Ok(done) => {
                edited(d);
                json!({
                    "ok": true, "chars": done.chars, "runs": done.runs, "images_removed": done.images_removed,
                    "images_kept": done.images_kept, "annotations_removed": done.annotations_removed,
                })
            }
            Err(e) => failure(&e),
        }
    })
}

/// What each page holds and whether it needs OCR: `{"pages":[{"page","kind":"native"|
/// "scanned_no_text"|"scanned_with_text_layer"|"image_only"|"blank","visible_chars",
/// "invisible_chars","images","image_coverage","needs_ocr"}]}`. The engine recognises no
/// characters; this says where the app should offer it.
///
/// # Safety
/// `doc` must be a live handle.
#[no_mangle]
pub unsafe extern "C" fn pdfcore_ocr_report(doc: *mut PdfDoc) -> *mut c_char {
    let Some(d) = handle(doc) else { return bad_argument() };
    guard(|| match d.doc.ocr_report() {
        Ok(pages) => json!({
            "pages": pages.iter().map(|r| json!({
                "page": r.page, "kind": r.kind.as_str(), "visible_chars": r.visible_chars,
                "invisible_chars": r.invisible_chars, "images": r.images, "image_coverage": r.image_coverage,
                "needs_ocr": r.needs_ocr,
            })).collect::<Vec<_>>()
        }),
        Err(e) => failure(&e),
    })
}

/// The file as opened with the changes appended as an update (every byte of the original stays at
/// the start; a signature keeps its byte range). Like `pdfcore_save`: a buffer of `*len` bytes freed
/// with `pdfcore_free_bytes`, or null with the reason in `pdfcore_last_error`.
///
/// # Safety
/// `doc` must be a live handle; `len` a valid pointer.
#[no_mangle]
pub unsafe extern "C" fn pdfcore_save_incremental(doc: *mut PdfDoc, len: *mut usize) -> *mut u8 {
    if len.is_null() {
        return std::ptr::null_mut();
    }
    *len = 0;
    let Some(d) = handle(doc) else { return std::ptr::null_mut() };
    match catch_unwind(AssertUnwindSafe(|| d.doc.save_incremental())) {
        Ok(Ok(bytes)) => {
            let mut bytes = bytes.into_boxed_slice().into_vec();
            bytes.shrink_to_fit();
            *len = bytes.len();
            let ptr = bytes.as_mut_ptr();
            std::mem::forget(bytes);
            ptr
        }
        Ok(Err(e)) => {
            remember(&e.to_string(), failure(&e)["kind"].as_str().unwrap_or("pdf"));
            std::ptr::null_mut()
        }
        Err(_) => {
            remember("fallo interno del motor al guardar", "panic");
            std::ptr::null_mut()
        }
    }
}

/// The vector graphics a page paints, in painting order, as a reader shows the page (top-left
/// origin, y down; rotation and `cm` applied; form XObjects followed): `{"graphics":[{"kind":
/// "rect"|"line"|"path","display":[x0,y0,x1,y1],"filled","stroked","fill":[r,g,b]|null,
/// "stroke":[r,g,b]|null,"line_width","xobject":null|name,"ends":null|[[x,y],[x,y]]}]}`. Letters
/// drawn as outlines are graphics too: filter by size.
///
/// # Safety
/// `doc` must be a live handle.
#[no_mangle]
pub unsafe extern "C" fn pdfcore_graphics(doc: *mut PdfDoc, page: c_int) -> *mut c_char {
    let Some(d) = handle(doc) else { return bad_argument() };
    guard(|| match d.doc.page_graphics(page.max(0) as usize) {
        Ok(list) => json!({
            "graphics": list.iter().map(|g| json!({
                "kind": g.kind.as_str(), "display": g.display, "filled": g.filled, "stroked": g.stroked,
                "fill": g.fill, "stroke": g.stroke, "line_width": g.line_width, "xobject": g.xobject,
                "ends": g.ends.map(|e| json!([[e[0].0, e[0].1], [e[1].0, e[1].1]])),
            })).collect::<Vec<_>>()
        }),
        Err(e) => failure(&e),
    })
}

/// Delete text strings for real (the letters leave the file; nothing around them moves):
/// `{"runs":[ids of the last pdfcore_page reading]}`. Returns `{"ok":true,"runs","chars"}`, one
/// undo step. `stale_run` when an id is not in the last reading, `unsupported` for text inside a
/// form XObject or written top to bottom; in both cases nothing changes.
///
/// # Safety
/// `doc` must be a live handle; `params` a NUL-terminated JSON string.
#[no_mangle]
pub unsafe extern "C" fn pdfcore_delete_runs(doc: *mut PdfDoc, page: c_int, params: *const c_char) -> *mut c_char {
    let (Some(d), Some(params)) = (handle(doc), string_arg(params)) else { return bad_argument() };
    guard(|| {
        let Ok(p) = serde_json::from_str::<Value>(params) else { return bad_params() };
        let page = page.max(0) as usize;
        let Some(ids) = p.get("runs").and_then(Value::as_array) else { return bad_params() };
        let stale = || json!({ "error": "ese texto ya no está donde se leyó: vuelve a leer la página", "kind": "stale_run" });
        let Some(reading) = d.reading.get(&page) else { return stale() };
        let mut chosen = Vec::new();
        for id in ids {
            match id.as_u64().map(|n| n as usize).filter(|n| *n < reading.len()) {
                Some(n) => chosen.push(n),
                None => return stale(),
            }
        }
        match d.doc.delete_runs(page, &chosen) {
            Ok(done) => {
                edited(d);
                json!({ "ok": true, "runs": done.runs, "chars": done.chars })
            }
            Err(e) => failure(&e),
        }
    })
}

/// A `NewText` from the JSON of `pdfcore_add_text`.
fn new_text_of(p: &Value) -> Option<NewText<'_>> {
    let (body, x, y) = (p["text"].as_str()?, p["x"].as_f64()?, p["y"].as_f64()?);
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
    Some(new)
}

fn arrange_of(name: &str) -> Option<Arrange> {
    Some(match name {
        "align_left" => Arrange::AlignLeft,
        "align_right" => Arrange::AlignRight,
        "align_top" => Arrange::AlignTop,
        "align_bottom" => Arrange::AlignBottom,
        "center_horizontally" => Arrange::CenterHorizontally,
        "center_vertically" => Arrange::CenterVertically,
        "distribute_horizontally" => Arrange::DistributeHorizontally,
        "distribute_vertically" => Arrange::DistributeVertically,
        "same_width" => Arrange::SameWidth,
        "same_height" => Arrange::SameHeight,
        _ => return None,
    })
}

fn align_of(name: &str) -> Option<Align> {
    Some(match name {
        "left" => Align::Left,
        "center" => Align::Center,
        "right" => Align::Right,
        _ => return None,
    })
}

/// What can be moved on a page, as `{"items":[{"id","kind":"text"|"image"|"field","display",
/// "text","lines"?,"name"?,"widget"?}]}`, top to bottom. Ids are good until the next edit.
///
/// # Safety
/// `doc` must be a live handle.
#[no_mangle]
pub unsafe extern "C" fn pdfcore_items(doc: *mut PdfDoc, page: c_int) -> *mut c_char {
    let Some(d) = handle(doc) else { return bad_argument() };
    guard(|| {
        let page = page.max(0) as usize;
        let items = match d.doc.page_items(page) {
            Ok(i) => i,
            Err(e) => return failure(&e),
        };
        let listed: Vec<Value> = items
            .iter()
            .enumerate()
            .map(|(id, i)| {
                let mut out = json!({
                    "id": id, "display": i.display, "text": i.text,
                    "kind": match i.kind { ItemKind::Text => "text", ItemKind::Image => "image", ItemKind::Field => "field" },
                });
                if i.kind == ItemKind::Text {
                    out["lines"] = json!(i.runs.len());
                }
                if let Some((name, widget)) = &i.field {
                    out["name"] = json!(name);
                    out["widget"] = json!(widget);
                }
                out
            })
            .collect();
        d.items.insert(page, items);
        json!({ "items": listed })
    })
}

/// The items a JSON list of ids names, from the last `pdfcore_items` of the page.
fn chosen(d: &PdfDoc, page: usize, ids: &Value) -> std::result::Result<Vec<Item>, Value> {
    let stale = || json!({ "error": "esos elementos ya no están donde se leyeron: vuelve a leer la página", "kind": "stale_item" });
    let list = d.items.get(&page).ok_or_else(stale)?;
    let ids = ids.as_array().ok_or_else(|| json!({ "error": "faltan los elementos", "kind": "bad_argument" }))?;
    ids.iter()
        .map(|id| id.as_u64().and_then(|n| list.get(n as usize)).cloned().ok_or_else(stale))
        .collect()
}

fn finished(d: &mut PdfDoc, result: pdfcore::Result<()>) -> Value {
    match result {
        Ok(()) => {
            edited(d);
            json!({ "ok": true })
        }
        Err(e) => failure(&e),
    }
}

/// Move items: `{"moves":[{"id","dx","dy"}]}`, points as a reader shows the page.
///
/// # Safety
/// `doc` must be a live handle; `params` a NUL-terminated JSON string.
#[no_mangle]
pub unsafe extern "C" fn pdfcore_move_items(doc: *mut PdfDoc, page: c_int, params: *const c_char) -> *mut c_char {
    let (Some(d), Some(params)) = (handle(doc), string_arg(params)) else { return bad_argument() };
    guard(|| {
        let page = page.max(0) as usize;
        let Ok(p) = serde_json::from_str::<Value>(params) else {
            return json!({ "error": "parámetros no válidos", "kind": "bad_argument" });
        };
        let Some(list) = p.get("moves").and_then(Value::as_array) else {
            return json!({ "error": "faltan los movimientos", "kind": "bad_argument" });
        };
        let mut moves = Vec::new();
        for m in list {
            let items = match chosen(d, page, &json!([m.get("id")])) {
                Ok(i) => i,
                Err(e) => return e,
            };
            let num = |k: &str| m.get(k).and_then(Value::as_f64).unwrap_or(0.0) as f32;
            moves.push((items[0].clone(), (num("dx"), num("dy"))));
        }
        let result = d.doc.move_items(page, &moves);
        finished(d, result)
    })
}

/// Line items up: `{"ids":[..],"how":"align_left"|...}` (the names of `pdfcore_arrange_fields`).
///
/// # Safety
/// `doc` must be a live handle; `params` a NUL-terminated JSON string.
#[no_mangle]
pub unsafe extern "C" fn pdfcore_arrange_items(doc: *mut PdfDoc, page: c_int, params: *const c_char) -> *mut c_char {
    let (Some(d), Some(params)) = (handle(doc), string_arg(params)) else { return bad_argument() };
    guard(|| {
        let page = page.max(0) as usize;
        let Ok(p) = serde_json::from_str::<Value>(params) else {
            return json!({ "error": "parámetros no válidos", "kind": "bad_argument" });
        };
        let Some(how) = p.get("how").and_then(Value::as_str).and_then(arrange_of) else {
            return json!({ "error": "«how» no es una forma de alinear conocida", "kind": "bad_argument" });
        };
        let items = match chosen(d, page, p.get("ids").unwrap_or(&Value::Null)) {
            Ok(i) => i,
            Err(e) => return e,
        };
        let result = d.doc.arrange_items(page, &items, how);
        finished(d, result)
    })
}

/// Even leading for text items: `{"ids":[..],"leading":null|points}`; `null` averages the gaps there are.
///
/// # Safety
/// `doc` must be a live handle; `params` a NUL-terminated JSON string.
#[no_mangle]
pub unsafe extern "C" fn pdfcore_space_lines(doc: *mut PdfDoc, page: c_int, params: *const c_char) -> *mut c_char {
    let (Some(d), Some(params)) = (handle(doc), string_arg(params)) else { return bad_argument() };
    guard(|| {
        let page = page.max(0) as usize;
        let Ok(p) = serde_json::from_str::<Value>(params) else {
            return json!({ "error": "parámetros no válidos", "kind": "bad_argument" });
        };
        let items = match chosen(d, page, p.get("ids").unwrap_or(&Value::Null)) {
            Ok(i) => i,
            Err(e) => return e,
        };
        let leading = p.get("leading").and_then(Value::as_f64).map(|v| v as f32);
        let result = d.doc.space_lines(page, &items, leading);
        finished(d, result)
    })
}

/// Align the lines of the paragraph a run belongs to: `align` 0 left, 1 centre, 2 right.
///
/// # Safety
/// `doc` must be a live handle.
#[no_mangle]
pub unsafe extern "C" fn pdfcore_align_paragraph(doc: *mut PdfDoc, page: c_int, run_id: c_int, align: c_int) -> *mut c_char {
    let Some(d) = handle(doc) else { return bad_argument() };
    guard(|| {
        let page = page.max(0) as usize;
        let Some(run) = d.reading.get(&page).and_then(|r| r.get(run_id.max(0) as usize)).cloned() else {
            return json!({ "error": "ese texto ya no está donde se leyó: vuelve a leer la página", "kind": "stale_run" });
        };
        let align = match align {
            1 => Align::Center,
            2 => Align::Right,
            _ => Align::Left,
        };
        let result = d.doc.align_paragraph(page, &run, align);
        finished(d, result)
    })
}

/// Align the text inside fields: `{"names":[..],"align":"left"|"center"|"right"}`.
///
/// # Safety
/// `doc` must be a live handle; `params` a NUL-terminated JSON string.
#[no_mangle]
pub unsafe extern "C" fn pdfcore_set_fields_alignment(doc: *mut PdfDoc, params: *const c_char) -> *mut c_char {
    let (Some(d), Some(params)) = (handle(doc), string_arg(params)) else { return bad_argument() };
    guard(|| {
        let Ok(p) = serde_json::from_str::<Value>(params) else {
            return json!({ "error": "parámetros no válidos", "kind": "bad_argument" });
        };
        let Some(align) = p.get("align").and_then(Value::as_str).and_then(align_of) else {
            return json!({ "error": "«align» debe ser left, center o right", "kind": "bad_argument" });
        };
        let Some(names) = p.get("names").and_then(Value::as_array) else {
            return json!({ "error": "faltan los campos", "kind": "bad_argument" });
        };
        let names: Vec<String> = names.iter().filter_map(|n| n.as_str().map(str::to_string)).collect();
        let result = d.doc.set_fields_alignment(&names, align);
        finished(d, result)
    })
}

/// Put a field's widget (index `widget` of its widgets) in the box `x0,y0,x1,y1`, in the page
/// as a reader shows it. The box is kept on the page; returns `{"ok":true,"display":[..]}`
/// with where it ended up. The field is redrawn at its new size.
///
/// # Safety
/// `doc` must be a live handle; `name` a NUL-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn pdfcore_move_field(
    doc: *mut PdfDoc,
    name: *const c_char,
    widget: c_int,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
) -> *mut c_char {
    let (Some(d), Some(name)) = (handle(doc), string_arg(name)) else { return bad_argument() };
    guard(|| match d.doc.move_field(name, widget.max(0) as usize, [x0, y0, x1, y1]) {
        Ok(display) => {
            edited(d);
            json!({ "ok": true, "display": display })
        }
        Err(e) => failure(&e),
    })
}

/// Line fields up as one step. `params` is JSON: `{"fields":[{"name","widget"}],"how":"..."}` with
/// `how` one of `align_left`, `align_right`, `align_top`, `align_bottom`, `center_horizontally`,
/// `center_vertically`, `distribute_horizontally`, `distribute_vertically`, `same_width`, `same_height`.
///
/// # Safety
/// `doc` must be a live handle; `params` a NUL-terminated JSON string.
#[no_mangle]
pub unsafe extern "C" fn pdfcore_arrange_fields(doc: *mut PdfDoc, params: *const c_char) -> *mut c_char {
    let (Some(d), Some(params)) = (handle(doc), string_arg(params)) else { return bad_argument() };
    guard(|| {
        let Ok(p) = serde_json::from_str::<Value>(params) else {
            return json!({ "error": "parámetros no válidos", "kind": "bad_argument" });
        };
        let Some(how) = p.get("how").and_then(Value::as_str).and_then(arrange_of) else {
            return json!({ "error": "«how» no es una forma de alinear conocida", "kind": "bad_argument" });
        };
        let items: Option<Vec<FieldRef>> = p.get("fields").and_then(Value::as_array).map(|list| {
            list.iter()
                .filter_map(|f| {
                    Some(FieldRef {
                        name: f.get("name")?.as_str()?.to_string(),
                        widget: f.get("widget").and_then(Value::as_u64).unwrap_or(0) as usize,
                    })
                })
                .collect()
        });
        let Some(items) = items else { return json!({ "error": "faltan los campos", "kind": "bad_argument" }) };
        match d.doc.arrange_fields(&items, how) {
            Ok(()) => {
                edited(d);
                json!({ "ok": true })
            }
            Err(e) => failure(&e),
        }
    })
}

/// Search flags: 1 match case, 2 whole words only, 4 accents matter.
fn search_options(flags: c_int) -> SearchOptions {
    SearchOptions { case_sensitive: flags & 1 != 0, whole_word: flags & 2 != 0, accent_sensitive: flags & 4 != 0 }
}

/// Every occurrence of `query` in the document:
/// `{"matches":[{"page","text","line","display","run","within_run":[start,end]|null}]}`.
///
/// # Safety
/// `doc` must be a live handle; `query` a NUL-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn pdfcore_find(doc: *mut PdfDoc, query: *const c_char, flags: c_int) -> *mut c_char {
    let (Some(d), Some(query)) = (handle(doc), string_arg(query)) else { return bad_argument() };
    guard(|| {
        let matches: Vec<Value> = d
            .doc
            .find_text(query, search_options(flags))
            .iter()
            .map(|m| {
                json!({
                    "page": m.page, "text": m.text, "line": m.line, "display": m.display, "run": m.run,
                    "within_run": m.within_run.map(|(a, b)| json!([a, b])),
                })
            })
            .collect();
        json!({ "matches": matches })
    })
}

/// Replace every occurrence of `find` with `replacement` as one undoable step:
/// `{"ok":true,"replaced":n,"skipped":n}`. `reflow` is as in `pdfcore_replace_text_ex`.
///
/// # Safety
/// `doc` must be a live handle; both strings NUL-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn pdfcore_replace_all(
    doc: *mut PdfDoc,
    find: *const c_char,
    replacement: *const c_char,
    flags: c_int,
    allow_fallback: c_int,
    reflow: c_int,
) -> *mut c_char {
    let (Some(d), Some(find), Some(replacement)) = (handle(doc), string_arg(find), string_arg(replacement)) else {
        return bad_argument();
    };
    guard(|| {
        let reflow = match reflow {
            0 => Reflow::None,
            2 => Reflow::Centered,
            3 => Reflow::RightAligned,
            _ => Reflow::Following,
        };
        let options = ReplaceOptions { allow_fallback: allow_fallback != 0, reflow, substitution: d.substitution };
        match d.doc.replace_all(find, replacement, search_options(flags), options) {
            Ok(done) => {
                edited(d);
                json!({ "ok": true, "replaced": done.replaced, "skipped": done.skipped })
            }
            Err(e) => failure(&e),
        }
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
                    "editable": f.editable, "multi_select": f.multi_select, "order": f.order, "max_len": f.max_len,
                    "signed": f.signed, "signer": f.signer,
                    "options": f.options.iter().map(|(e, l)| json!({"export": e, "label": l})).collect::<Vec<_>>(),
                    "widgets": f.widgets.iter().map(|w| json!({
                        "page": w.page, "display": w.display, "on_state": w.on_state,
                        "has_appearance": w.has_appearance,
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
    d.items.clear();
    d.blocks.clear();
}

/// `[{"char":"ñ","via":"encoding"|"bundled_font","font","similarity"}]`: letters the original font
/// lacked, what drew them and how close (0-100) that face is to the original.
fn substitutes_json(list: &[pdfcore::Substitute]) -> Vec<Value> {
    list.iter().map(|s| json!({ "char": s.ch.to_string(), "via": s.via.as_str(), "font": s.font, "similarity": s.similarity })).collect()
}

fn warning_json(w: &Warning) -> Value {
    match w {
        Warning::ExceedsPage { by } => json!({ "kind": "exceeds_page", "by": by }),
        Warning::ExceedsMargin { by } => json!({ "kind": "exceeds_margin", "by": by }),
        Warning::FollowerNotMoved { text } => json!({ "kind": "follower_not_moved", "text": text }),
        Warning::LineMayBeAligned { alignment } => json!({
            "kind": "line_may_be_aligned",
            "alignment": match alignment { Alignment::Centered => "centered", Alignment::Right => "right" },
        }),
        Warning::ReflowFailed => json!({ "kind": "reflow_failed" }),
        Warning::OverlapsText { text } => json!({ "kind": "overlaps_text", "text": text }),
        Warning::BelowPage { by } => json!({ "kind": "below_page", "by": by }),
        Warning::SharedForm { pages } => json!({ "kind": "shared_form", "pages": pages }),
    }
}

fn replace(d: &mut PdfDoc, page: c_int, run_id: c_int, new_text: &str, allow_fallback: c_int, reflow: c_int) -> Value {
    let page = page.max(0) as usize;
    let Some(run) = d.reading.get(&page).and_then(|r| r.get(run_id.max(0) as usize)).cloned() else {
        return json!({ "error": "ese texto ya no está donde se leyó: vuelve a leer la página", "kind": "stale_run" });
    };
    let reflow = match reflow {
        0 => Reflow::None,
        2 => Reflow::Centered,
        3 => Reflow::RightAligned,
        _ => Reflow::Following,
    };
    let options = ReplaceOptions { allow_fallback: allow_fallback != 0, reflow, substitution: d.substitution };
    match d.doc.replace_text_with(page, &run, new_text, options) {
        Ok(done) => {
            edited(d);
            json!({
                "ok": true, "old_width": done.old_width, "new_width": done.new_width,
                "fallback_chars": done.fallback_chars.iter().collect::<String>(),
                "substitutes": substitutes_json(&done.substitutes),
                "reread": done.reread, "moved": done.moved,
                "warnings": done.warnings.iter().map(warning_json).collect::<Vec<_>>(),
            })
        }
        Err(e) => failure(&e),
    }
}

/// Replace a run's text, moving what follows on its line (`reflow` 1). The
/// original entry point, kept so that existing callers keep working; see
/// `pdfcore_replace_text_ex`.
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
    pdfcore_replace_text_ex(doc, page, run_id, new_text, allow_fallback, 1)
}

/// Replace a run's text. `allow_fallback` != 0 draws characters the document's
/// font lacks with a bundled font. `reflow` says what happens to the rest of
/// the line: 0 nothing, 1 what follows moves (default), 2 the line keeps its
/// centre, 3 the line keeps its right edge. Invalidates every run id.
///
/// # Safety
/// `doc` must be a live handle; `new_text` a NUL-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn pdfcore_replace_text_ex(
    doc: *mut PdfDoc,
    page: c_int,
    run_id: c_int,
    new_text: *const c_char,
    allow_fallback: c_int,
    reflow: c_int,
) -> *mut c_char {
    let (Some(d), Some(new_text)) = (handle(doc), string_arg(new_text)) else { return bad_argument() };
    guard(|| replace(d, page, run_id, new_text, allow_fallback, reflow))
}

/// The completed digital signatures: `{"signatures":[{"field","signer"}]}`.
/// Editing invalidates them, so say so before the first edit.
///
/// # Safety
/// `doc` must be a live handle.
#[no_mangle]
pub unsafe extern "C" fn pdfcore_signatures(doc: *mut PdfDoc) -> *mut c_char {
    let Some(d) = handle(doc) else { return bad_argument() };
    guard(|| {
        let list: Vec<Value> = d.doc.signatures().iter().map(|s| json!({ "field": s.field, "signer": s.signer })).collect();
        json!({ "signatures": list })
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

/// Draw a JPEG or PNG on a page. `params` is JSON: `{"x","y","width","height"?}`, a
/// box in the page as a reader shows it (origin top-left); without `height` the
/// picture keeps its proportions. Returns `{"ok":true,"display":[..],"pixels":[w,h]}`.
///
/// # Safety
/// `doc` must be a live handle; `data` must point to `len` bytes; `params` a
/// NUL-terminated JSON string.
#[no_mangle]
pub unsafe extern "C" fn pdfcore_add_image(doc: *mut PdfDoc, page: c_int, data: *const u8, len: usize, params: *const c_char) -> *mut c_char {
    let (Some(d), Some(params)) = (handle(doc), string_arg(params)) else { return bad_argument() };
    if data.is_null() {
        return bad_argument();
    }
    let picture = std::slice::from_raw_parts(data, len);
    guard(|| {
        let Ok(p) = serde_json::from_str::<Value>(params) else {
            return json!({ "error": "parámetros no válidos", "kind": "bad_argument" });
        };
        let num = |key: &str| p.get(key).and_then(Value::as_f64).map(|v| v as f32);
        let (Some(x), Some(y), Some(width)) = (num("x"), num("y"), num("width")) else {
            return json!({ "error": "faltan x, y o width", "kind": "bad_argument" });
        };
        let new = NewImage { data: picture, x, y, width, height: num("height") };
        match d.doc.add_image(page.max(0) as usize, &new) {
            Ok(done) => {
                edited(d);
                json!({ "ok": true, "display": done.display, "pixels": [done.pixels.0, done.pixels.1] })
            }
            Err(e) => failure(&e),
        }
    })
}

// -- pages ------------------------------------------------------------------

fn page_edit(d: &mut PdfDoc, body: impl FnOnce(&mut Document) -> pdfcore::Result<()>) -> Value {
    match body(&mut d.doc) {
        Ok(()) => {
            edited(d);
            json!({ "ok": true, "pages": d.doc.page_count() })
        }
        Err(e) => failure(&e),
    }
}

/// Turn a page by `degrees` (a multiple of 90, clockwise).
///
/// # Safety
/// `doc` must be a live handle.
#[no_mangle]
pub unsafe extern "C" fn pdfcore_rotate_page(doc: *mut PdfDoc, page: c_int, degrees: c_int) -> *mut c_char {
    let Some(d) = handle(doc) else { return bad_argument() };
    guard(|| page_edit(d, |doc| doc.rotate_page(page.max(0) as usize, degrees)))
}

/// Remove a page (and the fields only it held). Invalidates every run id.
///
/// # Safety
/// `doc` must be a live handle.
#[no_mangle]
pub unsafe extern "C" fn pdfcore_delete_page(doc: *mut PdfDoc, page: c_int) -> *mut c_char {
    let Some(d) = handle(doc) else { return bad_argument() };
    guard(|| page_edit(d, |doc| doc.delete_page(page.max(0) as usize)))
}

/// Move the page at `from` so it ends up at index `to`.
///
/// # Safety
/// `doc` must be a live handle.
#[no_mangle]
pub unsafe extern "C" fn pdfcore_move_page(doc: *mut PdfDoc, from: c_int, to: c_int) -> *mut c_char {
    let Some(d) = handle(doc) else { return bad_argument() };
    guard(|| page_edit(d, |doc| doc.move_page(from.max(0) as usize, to.max(0) as usize)))
}

/// Bring pages of another PDF in before page `at` (the page count appends):
/// `{"ok":true,"added":n,"pages":total}`. `pages` lists which pages of the other
/// document (`n_pages` of them; null for all). `password` may be null.
///
/// # Safety
/// `doc` must be a live handle; `data` must point to `len` bytes; `pages`, when
/// not null, to `n_pages` ints; `password`, when not null, to a NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn pdfcore_insert_pages(
    doc: *mut PdfDoc,
    data: *const u8,
    len: usize,
    password: *const c_char,
    pages: *const c_int,
    n_pages: usize,
    at: c_int,
) -> *mut c_char {
    let Some(d) = handle(doc) else { return bad_argument() };
    if data.is_null() {
        return bad_argument();
    }
    let source = std::slice::from_raw_parts(data, len);
    let password = string_arg(password).unwrap_or("");
    let chosen: Option<Vec<usize>> = if pages.is_null() {
        None
    } else {
        Some(std::slice::from_raw_parts(pages, n_pages).iter().map(|p| (*p).max(0) as usize).collect())
    };
    guard(|| match d.doc.insert_pages(source, password, chosen.as_deref(), at.max(0) as usize) {
        Ok(added) => {
            edited(d);
            json!({ "ok": true, "added": added, "pages": d.doc.page_count() })
        }
        Err(e) => failure(&e),
    })
}

/// A new PDF with only the `n_pages` pages listed, in that order. Returns a
/// buffer of `*len` bytes to free with `pdfcore_free_bytes`, or null with the
/// reason in `pdfcore_last_error` / `pdfcore_last_error_kind`.
///
/// # Safety
/// `doc` must be a live handle; `pages` must point to `n_pages` ints; `len` a valid pointer.
#[no_mangle]
pub unsafe extern "C" fn pdfcore_extract_pages(doc: *mut PdfDoc, pages: *const c_int, n_pages: usize, len: *mut usize) -> *mut u8 {
    if len.is_null() {
        return std::ptr::null_mut();
    }
    *len = 0;
    let Some(d) = handle(doc) else { return std::ptr::null_mut() };
    if pages.is_null() {
        remember("argumento no válido", "bad_argument");
        return std::ptr::null_mut();
    }
    let chosen: Vec<usize> = std::slice::from_raw_parts(pages, n_pages).iter().map(|p| (*p).max(0) as usize).collect();
    match catch_unwind(AssertUnwindSafe(|| d.doc.extract_pages(&chosen))) {
        Ok(Ok(bytes)) => {
            let mut bytes = bytes.into_boxed_slice().into_vec();
            bytes.shrink_to_fit();
            *len = bytes.len();
            let ptr = bytes.as_mut_ptr();
            std::mem::forget(bytes);
            ptr
        }
        Ok(Err(e)) => {
            remember(&e.to_string(), failure(&e)["kind"].as_str().unwrap_or("pdf"));
            std::ptr::null_mut()
        }
        Err(_) => {
            remember("fallo interno del motor", "panic");
            std::ptr::null_mut()
        }
    }
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
            remember(&e.to_string(), "pdf");
            std::ptr::null_mut()
        }
        Err(_) => {
            remember("fallo interno del motor al guardar", "panic");
            std::ptr::null_mut()
        }
    }
}
