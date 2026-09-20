"""HTTP surface of the editor.

The server holds the open documents and does everything that needs PyMuPDF —
rendering pages, reading their text, rewriting it. The browser holds no PDF
logic at all: it shows rendered pages, lets the user type over them, and posts
back operations describing what changed.
"""

from __future__ import annotations

import io
import uuid
from contextlib import asynccontextmanager
from pathlib import Path
from typing import Any

import pymupdf
from fastapi import Body, FastAPI, File, Form, HTTPException, Query, UploadFile
from fastapi.responses import FileResponse, JSONResponse, Response
from fastapi.staticfiles import StaticFiles

from . import __version__
from .editor import EditError, apply_operations, pages_touched
from .extract import extract_page, page_summaries
from .forms import page_fields
from .ocr import OcrUnavailable, recognise, support
from .search import find, replace_operations
from .store import DocumentError, DocumentStore

STATIC_DIR = Path(__file__).parent / "static"

MAX_IMAGE_BYTES = 25 * 1024 * 1024
MAX_RENDER_ZOOM = 4.0

store = DocumentStore()


@asynccontextmanager
async def lifespan(app: FastAPI):
    yield
    store.close_all()


app = FastAPI(title="Editor de PDF", version=__version__, lifespan=lifespan)


@app.exception_handler(DocumentError)
async def _document_error(_request, exc: DocumentError) -> JSONResponse:
    return JSONResponse({"detail": str(exc)}, status_code=404)


@app.exception_handler(EditError)
async def _edit_error(_request, exc: EditError) -> JSONResponse:
    return JSONResponse({"detail": str(exc)}, status_code=400)


def _document_payload(document) -> dict[str, Any]:
    return {
        **document.state(),
        "pages": page_summaries(document.doc),
        "ocr": support().as_dict(),
    }


# -- documents -------------------------------------------------------------


@app.post("/api/documents")
async def upload_document(
    file: UploadFile = File(...),
    password: str | None = Form(default=None),
) -> dict[str, Any]:
    """Open an uploaded PDF and return its page geometry."""
    data = await file.read()
    document = store.open(data, file.filename or "documento.pdf", password)
    return _document_payload(document)


@app.get("/api/documents/{doc_id}")
async def get_document(doc_id: str) -> dict[str, Any]:
    return _document_payload(store.get(doc_id))


@app.delete("/api/documents/{doc_id}")
async def close_document(doc_id: str) -> dict[str, bool]:
    store.close(doc_id)
    return {"closed": True}


@app.get("/api/documents/{doc_id}/fonts")
async def list_fonts(doc_id: str) -> dict[str, Any]:
    """Fonts the toolbar offers: the document's own first, then generics."""
    document = store.get(doc_id)
    with document.lock:
        return {"families": document.resolver.available_families()}


# -- pages -----------------------------------------------------------------


@app.get("/api/documents/{doc_id}/pages/{pno}/render")
async def render_page(
    doc_id: str,
    pno: int,
    zoom: float = Query(default=1.5, gt=0.1, le=MAX_RENDER_ZOOM),
    revision: int = Query(default=0),
) -> Response:
    """Render one page to PNG at ``zoom`` times its natural size."""
    document = store.get(doc_id)
    with document.lock:
        if not 0 <= pno < document.doc.page_count:
            raise HTTPException(404, f"La página {pno + 1} no existe")
        matrix = pymupdf.Matrix(zoom, zoom)
        pixmap = document.doc[pno].get_pixmap(matrix=matrix, alpha=False)
        png = pixmap.tobytes("png")
    return Response(png, media_type="image/png", headers={"Cache-Control": "no-store"})


@app.get("/api/documents/{doc_id}/pages/{pno}/text")
async def get_page_text(doc_id: str, pno: int) -> dict[str, Any]:
    """The editable lines of one page, with the font data behind each span.

    The form fields are added here rather than in the extraction itself: a
    field is not page content, and the text reader has no business knowing
    about the document's form.
    """
    document = store.get(doc_id)
    with document.lock:
        if not 0 <= pno < document.doc.page_count:
            raise HTTPException(404, f"La página {pno + 1} no existe")
        page = extract_page(document.doc, pno, document.resolver)
        page["fields"] = page_fields(document.doc[pno])
        return page


# -- editing ---------------------------------------------------------------


@app.post("/api/documents/{doc_id}/operations")
async def post_operations(doc_id: str, payload: dict = Body(...)) -> dict[str, Any]:
    """Apply a batch of edit operations.

    The batch is all-or-nothing: the document is snapshotted first, and put back
    exactly as it was if any operation in the batch fails.
    """
    document = store.get(doc_id)
    operations = payload.get("operations") or []
    if not isinstance(operations, list):
        raise HTTPException(400, "«operations» debe ser una lista")
    if not operations:
        return {**document.state(), "warnings": []}

    with document.lock:
        before = document.doc.tobytes(garbage=0, deflate=True)
        # Record only the pages this batch will change.
        document.snapshot(pages_touched(operations))
        try:
            warnings = apply_operations(
                document.doc, document.resolver, operations, document.assets
            )
        except EditError:
            document.rollback(before)
            document.undo_stack.pop()
            raise
        except Exception as exc:
            document.rollback(before)
            document.undo_stack.pop()
            raise EditError(f"No se pudo aplicar la edición: {exc}") from exc
        return {**document.state(), "warnings": [w.as_dict() for w in warnings]}


@app.post("/api/documents/{doc_id}/ocr")
async def run_ocr(doc_id: str, payload: dict = Body(default={})) -> dict[str, Any]:
    """Read the text off scanned pages so they can be edited.

    Slow by nature — every page is rendered and handed to Tesseract — so the
    client is told which pages were recognised rather than guessing.
    """
    document = store.get(doc_id)
    pages = payload.get("pages")
    if pages is not None and not isinstance(pages, list):
        raise HTTPException(400, "«pages» debe ser una lista")

    with document.lock:
        before = document.doc.tobytes(garbage=0, deflate=True)
        document.snapshot()
        try:
            recognised = recognise(
                document.doc,
                [int(p) for p in pages] if pages is not None else None,
                language=payload.get("language") or None,
            )
        except OcrUnavailable as exc:
            document.rollback(before)
            document.undo_stack.pop()
            raise HTTPException(503, str(exc)) from exc
        except Exception as exc:
            document.rollback(before)
            document.undo_stack.pop()
            raise EditError(f"El reconocimiento falló: {exc}") from exc

        if not recognised:
            document.undo_stack.pop()
        return {**_document_payload(document), "recognised": recognised}


@app.post("/api/documents/{doc_id}/search")
async def search(doc_id: str, payload: dict = Body(...)) -> dict[str, Any]:
    """Every occurrence of a string, in reading order."""
    document = store.get(doc_id)
    query = str(payload.get("query") or "")
    with document.lock:
        matches = find(
            document.doc,
            document.resolver,
            query,
            match_case=bool(payload.get("match_case")),
            whole_word=bool(payload.get("whole_word")),
        )
    return {"count": len(matches), "matches": [match.as_dict() for match in matches]}


@app.post("/api/documents/{doc_id}/replace")
async def replace(doc_id: str, payload: dict = Body(...)) -> dict[str, Any]:
    """Replace every occurrence, as one undoable step."""
    document = store.get(doc_id)
    query = str(payload.get("query") or "")
    if not query:
        raise HTTPException(400, "No hay nada que buscar")
    replacement = str(payload.get("replacement") or "")

    with document.lock:
        operations, count = replace_operations(
            document.doc,
            document.resolver,
            query,
            replacement,
            match_case=bool(payload.get("match_case")),
            whole_word=bool(payload.get("whole_word")),
        )
        if not operations:
            return {**document.state(), "replaced": 0, "warnings": []}

        before = document.doc.tobytes(garbage=0, deflate=True)
        document.snapshot(pages_touched(operations))
        try:
            warnings = apply_operations(
                document.doc, document.resolver, operations, document.assets
            )
        except Exception as exc:
            document.rollback(before)
            document.undo_stack.pop()
            raise EditError(f"No se pudo reemplazar: {exc}") from exc

        return {
            **document.state(),
            "replaced": count,
            "warnings": [w.as_dict() for w in warnings],
        }


@app.post("/api/documents/{doc_id}/undo")
async def undo(doc_id: str) -> dict[str, Any]:
    document = store.get(doc_id)
    with document.lock:
        changed = document.undo()
        return {**document.state(), "changed": changed, "pages": page_summaries(document.doc)}


@app.post("/api/documents/{doc_id}/redo")
async def redo(doc_id: str) -> dict[str, Any]:
    document = store.get(doc_id)
    with document.lock:
        changed = document.redo()
        return {**document.state(), "changed": changed, "pages": page_summaries(document.doc)}


@app.post("/api/documents/{doc_id}/assets")
async def upload_asset(doc_id: str, file: UploadFile = File(...)) -> dict[str, Any]:
    """Stage an image so a later ``insert_image`` operation can place it."""
    document = store.get(doc_id)
    data = await file.read()
    if not data:
        raise HTTPException(400, "La imagen está vacía.")
    if len(data) > MAX_IMAGE_BYTES:
        raise HTTPException(400, "La imagen supera el tamaño máximo de 25 MB.")
    try:
        pixmap = pymupdf.Pixmap(io.BytesIO(data))
        width, height = pixmap.width, pixmap.height
    except Exception as exc:
        raise HTTPException(400, f"Formato de imagen no reconocido: {exc}") from exc

    asset_id = uuid.uuid4().hex
    with document.lock:
        document.assets[asset_id] = data
    return {"asset": asset_id, "width": width, "height": height}


@app.get("/api/documents/{doc_id}/download")
async def download(doc_id: str) -> Response:
    """The edited PDF."""
    document = store.get(doc_id)
    with document.lock:
        data = document.export()
        name = document.name
    if not name.lower().endswith(".pdf"):
        name += ".pdf"
    ascii_name = name.encode("ascii", "ignore").decode() or "documento.pdf"
    return Response(
        data,
        media_type="application/pdf",
        headers={
            "Content-Disposition": f'attachment; filename="{ascii_name}"',
            "Cache-Control": "no-store",
        },
    )


# -- static ----------------------------------------------------------------


@app.get("/")
async def index() -> FileResponse:
    return FileResponse(STATIC_DIR / "index.html")


app.mount("/static", StaticFiles(directory=STATIC_DIR), name="static")
