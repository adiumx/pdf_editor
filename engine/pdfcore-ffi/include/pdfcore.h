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
