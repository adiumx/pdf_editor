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
