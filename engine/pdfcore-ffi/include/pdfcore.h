/* C interface to the pdfcore engine, for Dart FFI. See pdfcore-ffi/src/lib.rs.
 *
 * Every function that returns `char*` returns JSON: the data on success (or
 * {"ok":true}), {"error":"...","kind":"..."} on failure. Free it with
 * pdfcore_free_string. A text run is named by its "id" in the last reading of
 * its page; any edit, undo or redo invalidates every id.
 */
#ifndef PDFCORE_H
#define PDFCORE_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct PdfDoc PdfDoc;

/* Opening, closing, freeing. */
PdfDoc *pdfcore_open(const uint8_t *data, size_t len); /* NULL on failure */
/* For a document protected by a password. On NULL, pdfcore_last_error_kind() is
   "password_required" or "wrong_password" when that is the reason. */
PdfDoc *pdfcore_open_with_password(const uint8_t *data, size_t len, const char *password);
const char *pdfcore_last_error_kind(void);
const char *pdfcore_last_error(void);                  /* valid until the next call */
void pdfcore_close(PdfDoc *doc);
void pdfcore_free_string(char *s);
void pdfcore_free_bytes(uint8_t *p, size_t len);

/* Reading. */
int pdfcore_page_count(PdfDoc *doc);
char *pdfcore_page(PdfDoc *doc, int page);
char *pdfcore_fields(PdfDoc *doc);
char *pdfcore_lines(PdfDoc *doc, int page); /* {"lines":[{"text","display","runs","words":[{"text","display"}],"invisible"}]} */
/* flags: 1 match case, 2 whole words, 4 accents matter */
char *pdfcore_find(PdfDoc *doc, const char *query, int flags); /* {"matches":[{"page","text","line","display","run","within_run"}]} */
char *pdfcore_replace_all(PdfDoc *doc, const char *find, const char *replacement, int flags, int allow_fallback, int reflow); /* {"ok":true,"replaced","skipped"} */
/* params: {"x","y","width","height"?} in the page as shown; {"ok":true,"display","pixels":[w,h]} */
char *pdfcore_add_image(PdfDoc *doc, int page, const uint8_t *data, size_t len, const char *params);
char *pdfcore_rotate_page(PdfDoc *doc, int page, int degrees);
char *pdfcore_delete_page(PdfDoc *doc, int page);
char *pdfcore_move_page(PdfDoc *doc, int from, int to);
/* pages: which pages of the other document (NULL for all). {"ok":true,"added","pages"} */
char *pdfcore_insert_pages(PdfDoc *doc, const uint8_t *data, size_t len, const char *password,
                           const int *pages, size_t n_pages, int at);
/* NULL on failure (see pdfcore_last_error / _kind); free with pdfcore_free_bytes */
uint8_t *pdfcore_extract_pages(PdfDoc *doc, const int *pages, size_t n_pages, size_t *len);
/* {"paragraph":null} or {"paragraph":{"text","runs","display"}} */
char *pdfcore_paragraph(PdfDoc *doc, int page, int run_id);
/* new_text is the paragraph's whole text; {"ok":true,"lines","added","removed","fallback_chars","warnings"} */
char *pdfcore_required_fonts(PdfDoc *doc);
char *pdfcore_register_font(PdfDoc *doc, const uint8_t *data, size_t len);
char *pdfcore_replace_block_ex(PdfDoc *doc, int page, int block_id, const char *new_text, const char *params_json);
char *pdfcore_replace_range(PdfDoc *doc, int page, int block_id, int start, int end, const char *new_text, const char *params_json);
char *pdfcore_set_block_style(PdfDoc *doc, int page, int block_id, int start, int end, const char *params_json);
char *pdfcore_block_chars(PdfDoc *doc, int page, int block_id);
char *pdfcore_set_substitution(PdfDoc *doc, int policy);
char *pdfcore_blocks(PdfDoc *doc, int page);
char *pdfcore_replace_block(PdfDoc *doc, int page, int block_id, const char *new_text, int allow_fallback);
char *pdfcore_replace_paragraph(PdfDoc *doc, int page, int run_id, const char *new_text, int allow_fallback);
/* box in the page as shown; {"ok":true,"display":[x0,y0,x1,y1]} with where it ended up */
char *pdfcore_move_field(PdfDoc *doc, const char *name, int widget, float x0, float y0, float x1, float y1);
/* {"fields":[{"name","widget"}],"how":"align_left"|"align_right"|"align_top"|"align_bottom"|
   "center_horizontally"|"center_vertically"|"distribute_horizontally"|"distribute_vertically"|
   "same_width"|"same_height"} */
char *pdfcore_arrange_fields(PdfDoc *doc, const char *params);
/* what can be moved on a page: {"items":[{"id","kind":"text"|"image"|"field","display","text","lines"?,"name"?,"widget"?}]} */
char *pdfcore_items(PdfDoc *doc, int page);
char *pdfcore_move_items(PdfDoc *doc, int page, const char *params);   /* {"moves":[{"id","dx","dy"}]} */
char *pdfcore_arrange_items(PdfDoc *doc, int page, const char *params); /* {"ids":[..],"how":"align_left"|...} */
char *pdfcore_space_lines(PdfDoc *doc, int page, const char *params);   /* {"ids":[..],"leading":null|points} */
char *pdfcore_align_paragraph(PdfDoc *doc, int page, int run_id, int align); /* 0 left, 1 centre, 2 right */
char *pdfcore_set_fields_alignment(PdfDoc *doc, const char *params);    /* {"names":[..],"align":"left"|"center"|"right"} */
char *pdfcore_signatures(PdfDoc *doc); /* {"signatures":[{"field","signer"}]} */

/* Editing. */
char *pdfcore_replace_text(PdfDoc *doc, int page, int run_id, const char *new_text, int allow_fallback);
/* reflow: 0 nothing moves, 1 what follows on the line moves (default),
 * 2 the line keeps its centre, 3 the line keeps its right edge. */
char *pdfcore_replace_text_ex(PdfDoc *doc, int page, int run_id, const char *new_text, int allow_fallback, int reflow);
char *pdfcore_add_text(PdfDoc *doc, int page, const char *params_json);
char *pdfcore_set_field(PdfDoc *doc, const char *name, const char *value, int ignore_read_only);

/* History and saving. */
char *pdfcore_undo(PdfDoc *doc);
char *pdfcore_redo(PdfDoc *doc);
char *pdfcore_history(PdfDoc *doc);
uint8_t *pdfcore_save(PdfDoc *doc, size_t *len); /* free with pdfcore_free_bytes */

#ifdef __cplusplus
}
#endif

#endif /* PDFCORE_H */
