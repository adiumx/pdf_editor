"""Reading and filling a PDF's interactive form.

A form field is not page content: it is an annotation whose value lives in the
document's field list, which is why editing the page never reaches it and why
filling one in is its own operation rather than a kind of text edit.

Fields are addressed by xref, like the marks in :mod:`app.extract`. An xref is
stable for as long as the document is open, which is exactly as long as the
client holds onto it: the page is re-read after every operation.
"""

from __future__ import annotations

from typing import Any

import pymupdf

from .extract import to_display

# What the browser calls each kind of field. A listbox and a combobox are both
# a choice; what separates them is whether the value has to be one of the
# options, which is reported separately.
KINDS = {
    pymupdf.PDF_WIDGET_TYPE_TEXT: "text",
    pymupdf.PDF_WIDGET_TYPE_CHECKBOX: "checkbox",
    pymupdf.PDF_WIDGET_TYPE_RADIOBUTTON: "radio",
    pymupdf.PDF_WIDGET_TYPE_COMBOBOX: "combo",
    pymupdf.PDF_WIDGET_TYPE_LISTBOX: "list",
    pymupdf.PDF_WIDGET_TYPE_SIGNATURE: "signature",
    pymupdf.PDF_WIDGET_TYPE_BUTTON: "button",
}

# Field flags, from the PDF specification's table of field attributes.
_READ_ONLY = 1 << 0
_REQUIRED = 1 << 1
_MULTILINE = 1 << 12
_PASSWORD = 1 << 13

# Kinds nothing here can fill in: a push button runs a script, and a signature
# needs a certificate and a private key, neither of which this editor has.
UNFILLABLE = {"button", "signature"}


class FormError(ValueError):
    """A field cannot be filled in as asked."""


def page_fields(page: pymupdf.Page) -> list[dict[str, Any]]:
    """Every form field of a page, in the space the browser draws it in.

    Read out into plain values as each one is visited: a live widget belongs to
    the walk that produced it, and reading one afterwards is not safe.
    """
    fields: list[dict[str, Any]] = []
    for widget in page.widgets():
        kind = KINDS.get(widget.field_type, "unknown")
        flags = widget.field_flags or 0
        rect = to_display(page, pymupdf.Rect(widget.rect))
        fields.append(
            {
                "xref": widget.xref,
                "name": widget.field_name or "",
                "label": widget.field_label or "",
                "kind": kind,
                "bbox": [round(v, 2) for v in rect],
                "value": _readable(widget, kind),
                "options": list(widget.choice_values or []),
                "on_state": widget.on_state() if kind in ("checkbox", "radio") else None,
                "readonly": bool(flags & _READ_ONLY) or kind in UNFILLABLE,
                "required": bool(flags & _REQUIRED),
                "multiline": bool(flags & _MULTILINE),
                "password": bool(flags & _PASSWORD),
                "maxlen": int(widget.text_maxlen or 0),
                "fontsize": float(widget.text_fontsize or 0),
            }
        )
    return fields


def _readable(widget: pymupdf.Widget, kind: str) -> Any:
    """A field's value as the browser wants it: a tick is true or false."""
    value = widget.field_value
    if kind in ("checkbox", "radio"):
        # An unticked box reads "Off"; a ticked one reads whatever its on state
        # happens to be called, which differs from document to document.
        return bool(value) and str(value) != "Off"
    return "" if value is None else str(value)


def set_field(page: pymupdf.Page, xref: int, value: Any) -> dict[str, Any]:
    """Fill one field in, and report what it now holds.

    Ticking one button of a radio group unticks its siblings: they share a
    value, held by the field they all hang off, so there is only one to set.
    """
    emptied = False
    for widget in page.widgets():
        if widget.xref != xref:
            continue
        kind = KINDS.get(widget.field_type, "unknown")
        flags = widget.field_flags or 0
        if kind in UNFILLABLE:
            raise FormError(
                "Una firma digital no se puede rellenar desde aquí."
                if kind == "signature"
                else "Ese campo es un botón, no se rellena."
            )
        if flags & _READ_ONLY:
            raise FormError(f"El campo «{widget.field_name}» es de solo lectura.")

        if kind in ("checkbox", "radio"):
            widget.field_value = bool(value)
        elif kind in ("combo", "list"):
            options = list(widget.choice_values or [])
            text = "" if value is None else str(value)
            # A combo box may allow anything typed into it; a list box may not.
            if options and text not in options and kind == "list":
                raise FormError(f"«{text}» no es una de las opciones del campo.")
            widget.field_value = text
        else:
            text = "" if value is None else str(value)
            limit = int(widget.text_maxlen or 0)
            if limit and len(text) > limit:
                raise FormError(
                    f"El campo «{widget.field_name}» admite {limit} caracteres."
                )
            widget.field_value = text
            emptied = not text
        widget.update()
        break
    else:
        raise FormError("Ese campo ya no está en la página.")

    if emptied:
        _empty_text_field(page, xref)

    # Read back from the page rather than from the widget just written: a radio
    # group's value is held once for the whole group, so what a button ends up
    # showing is not always what was asked of it.
    for field in page_fields(page):
        if field["xref"] == xref:
            return field
    raise FormError("Ese campo ya no está en la página.")  # pragma: no cover


def _empty_text_field(page: pymupdf.Page, xref: int) -> None:
    """Clear a text field outright.

    Assigning an empty string is quietly ignored: nothing is written and the
    old value stays. So the value is emptied in the object itself — and then
    the box is redrawn from a *freshly read* field, because redrawing the one
    that was just written would put its former contents straight back.
    """
    page.parent.xref_set_key(xref, "V", "()")
    for widget in page.widgets():
        if widget.xref == xref:
            widget.update()
            break


def has_form(doc: pymupdf.Document) -> bool:
    """Whether the document carries an interactive form at all."""
    return bool(doc.is_form_pdf)


def signed_field_names(doc: pymupdf.Document) -> list[str]:
    """Who signed the document, for every signature that actually went in.

    A signature *field* just means the form has a place for one; ``is_signed``
    is what says whether it is filled. Only a signed one is worth warning
    about — an edit does not invalidate a signature nobody has put there yet.

    The name shown is the signer's, from the signature itself (``/V/Name``,
    the way a signing tool records who signed), not the field's own name —
    ``firma`` tells the person editing nothing that "you are about to
    invalidate a signature" doesn't already say, while a name does. It falls
    back to the field's name only when the signature carries none.

    Read fresh rather than cached: it has to reflect whatever the document was
    opened with or has since become, and a form is rare enough that scanning
    every page for it costs no more than the page summary already does.
    """
    names: list[str] = []
    for page in doc:
        for widget in page.widgets():
            if widget.field_type != pymupdf.PDF_WIDGET_TYPE_SIGNATURE or not widget.is_signed:
                continue
            ok, raw = doc.xref_get_key(widget.xref, "V/Name")
            signer = raw if ok == "string" else ""
            names.append(signer or widget.field_name or "")
    return names
