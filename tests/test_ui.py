"""The editor driven through a real browser, with real mouse input.

These exist because the unit tests could not catch a whole class of bug: a
format bar whose controls cannot be clicked still passes every test that sets
values through the DOM. Playwright's `select_option` and `fill` bypass the mouse
the same way, so the tests here click, type and read focus.

Skipped when Playwright or a browser is not installed.
"""

from __future__ import annotations

import socket
import threading
import time

import pytest

from tests.conftest import build_pdf, build_pdf_with_every_kind_of_field, requires_fonts

playwright_api = pytest.importorskip("playwright.sync_api", reason="playwright no instalado")


def _free_port() -> int:
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


@pytest.fixture(scope="module")
def server():
    """Serve the editor on a spare port for the length of the module."""
    import uvicorn

    from app.main import app, store

    port = _free_port()
    config = uvicorn.Config(app, host="127.0.0.1", port=port, log_level="error")
    instance = uvicorn.Server(config)
    thread = threading.Thread(target=instance.run, daemon=True)
    thread.start()

    deadline = time.time() + 20
    while not instance.started and time.time() < deadline:
        time.sleep(0.05)
    if not instance.started:
        pytest.skip("el servidor de pruebas no arrancó")

    yield f"http://127.0.0.1:{port}"

    instance.should_exit = True
    thread.join(timeout=10)
    store.close_all()


def _launch(playwright):
    """Start a browser, falling back to a browser installed out of band."""
    import glob
    import os

    try:
        return playwright.chromium.launch()
    except Exception:
        root = os.environ.get("PLAYWRIGHT_BROWSERS_PATH", "/opt/pw-browsers")
        for pattern in ("chromium-*/chrome-linux/chrome", "chromium-*/chrome-mac/*/Chromium"):
            for path in sorted(glob.glob(os.path.join(root, pattern))):
                try:
                    return playwright.chromium.launch(
                        executable_path=path, args=["--no-sandbox"]
                    )
                except Exception:
                    continue
        pytest.skip("no hay navegador de Playwright instalado")


@pytest.fixture(scope="module")
def browser():
    with playwright_api.sync_playwright() as playwright:
        instance = _launch(playwright)
        yield instance
        instance.close()


@pytest.fixture
def page(browser, server, tmp_path):
    """A fresh tab with the sample document open.

    Per test rather than per module: an open native dropdown or a half-finished
    edit left behind by one test swallows the next test's clicks, and a shared
    tab makes such a failure look like a bug in the feature under test.
    """
    tab = browser.new_page(viewport={"width": 1400, "height": 900})
    errors: list[str] = []
    tab.on("pageerror", lambda exc: errors.append(str(exc)))
    tab.on("console", lambda msg: errors.append(msg.text) if msg.type == "error" else None)

    sample = tmp_path / "ejemplo.pdf"
    sample.write_bytes(build_pdf())
    tab.goto(server, wait_until="networkidle")
    tab.set_input_files("#file-input", str(sample))
    tab.wait_for_selector(".page .span", timeout=30000)

    tab.console_errors = errors  # type: ignore[attr-defined]
    yield tab
    tab.close()


def span_with(page, text: str):
    """The overlay covering a given piece of text, found by what it covers.

    Never by index: rewriting a line changes how the page splits into spans, so
    positions shift under any test that edits something. The overlay itself is
    empty, so the search goes through the text it carries as an attribute.
    """
    return page.locator(f'.page .span[data-text*="{text}"]').first


def open_editor(page, text: str = "Primera linea"):
    """Click a span the way a person would, and wait for its box."""
    target = span_with(page, text)
    target.wait_for(state="visible", timeout=10000)
    target.click()
    page.wait_for_selector(".span.is-editing .span__input", timeout=10000)


@requires_fonts
class TestClickingText:
    def test_clicking_a_span_puts_the_caret_in_it(self, page):
        """The box used to open with focus left on the body, so whatever the
        user typed next went nowhere."""
        open_editor(page)
        focused = page.evaluate(
            "() => document.activeElement?.classList?.contains('span__input') || false"
        )
        assert focused, "el foco no llegó al cuadro de edición"

    def test_typing_after_clicking_reaches_the_text(self, page):
        open_editor(page)
        before = page.eval_on_selector(".span.is-editing .span__input", "e => e.textContent")
        page.keyboard.type("ZZZ")
        after = page.eval_on_selector(".span.is-editing .span__input", "e => e.textContent")
        assert after != before and "ZZZ" in after


@requires_fonts
class TestFormatBar:
    """Its controls used to be unclickable: a preventDefault meant to keep the
    text box open also stopped the bar's own inputs from taking focus."""

    def test_the_bar_appears_when_text_is_being_edited(self, page):
        open_editor(page)
        assert page.locator("#formatbar").is_visible()

    def test_clicking_the_size_field_focuses_it(self, page):
        open_editor(page)
        page.locator("#fmt-size").click()
        assert page.evaluate("() => document.activeElement?.id") == "fmt-size"

    def test_typing_a_size_changes_the_field(self, page):
        open_editor(page)
        page.locator("#fmt-size").click()
        page.keyboard.press("Control+a")
        page.keyboard.type("21")
        assert page.locator("#fmt-size").input_value() == "21"

    def test_clicking_the_font_list_focuses_it(self, page):
        open_editor(page)
        page.locator("#fmt-family").click()
        assert page.evaluate("() => document.activeElement?.id") == "fmt-family"

    def test_the_font_list_offers_more_than_the_generic_families(self, page):
        open_editor(page)
        labels = page.eval_on_selector_all("#fmt-family optgroup", "els => els.map(e => e.label)")
        options = page.eval_on_selector_all("#fmt-family option", "els => els.length")
        assert any("Estándar" in label for label in labels)
        assert any("Instaladas" in label for label in labels)
        assert options > 6, "el desplegable debería ofrecer bastante más que las genéricas"

    def test_the_size_field_is_not_rewritten_while_being_typed_in(self, page):
        """Re-rendering the bar on every state change used to clobber input."""
        open_editor(page)
        page.locator("#fmt-size").click()
        page.keyboard.press("Control+a")
        page.keyboard.type("3")
        page.keyboard.type("3")
        assert page.locator("#fmt-size").input_value() == "33"


@requires_fonts
class TestApplyingChanges:
    def test_a_size_change_reaches_the_document(self, page):
        # A line of its own, so the change takes the single-line path and the
        # text stays findable; re-wrapping is covered on its own below.
        open_editor(page, "Titulo en negrita")
        original = float(page.locator("#fmt-size").input_value())
        assert original != 26

        page.locator("#fmt-size").click()
        page.keyboard.press("Control+a")
        page.keyboard.type("26")
        page.keyboard.press("Enter")  # Enter inside the bar applies
        page.wait_for_function("() => !document.querySelector('.span.is-editing')", timeout=15000)
        page.wait_for_selector(".page .span", timeout=15000)

        # Read it back from the document, not from the bar's leftover state.
        open_editor(page, "Titulo en negrita")
        assert float(page.locator("#fmt-size").input_value()) == pytest.approx(26, abs=0.6)

    def test_a_font_change_reaches_the_document(self, page):
        open_editor(page, "Titulo en negrita")
        page.locator("#fmt-family").click()
        page.keyboard.press("Escape")  # close the native list, keep the box open
        page.select_option("#fmt-family", "Liberation Mono")
        page.click("#fmt-apply")
        page.wait_for_function("() => !document.querySelector('.span.is-editing')", timeout=15000)
        page.wait_for_selector(".page .span", timeout=15000)

        open_editor(page, "Titulo en negrita")
        assert "Mono" in page.locator("#fmt-family").input_value()

    def test_no_console_errors_were_raised_along_the_way(self, page):
        assert page.console_errors == []


@requires_fonts
class TestReflow:
    """Text that runs across several lines re-wraps; a line on its own does not."""

    def test_the_reflow_button_appears_only_for_multi_line_text(self, page):
        open_editor(page, "Primera linea")
        assert page.locator("#fmt-reflow").is_visible()

        page.keyboard.press("Escape")
        open_editor(page, "Una linea en sans")
        assert page.locator("#fmt-reflow").is_hidden()

    def test_re_wrapping_keeps_every_word(self, page):
        open_editor(page, "Primera linea")
        page.click("#fmt-reflow")
        page.wait_for_function("() => !document.querySelector('.span.is-editing')", timeout=15000)
        page.wait_for_selector(".page .span", timeout=15000)
        text = " ".join(
            page.eval_on_selector_all(".page .span", "els => els.map(e => e.dataset.text)")
        )
        for word in ("Primera", "documento", "Segunda", "debajo"):
            assert word in text, f"se perdió «{word}» al reajustar"


@requires_fonts
class TestUnsavedWork:
    """The document lives in the server's memory and nowhere else, so leaving
    the page throws the edits away. It used to do that silently."""

    def _edit_something(self, page):
        open_editor(page)
        page.keyboard.type("XX")
        page.keyboard.press("Enter")
        page.wait_for_function("() => !document.querySelector('.span.is-editing')", timeout=15000)
        page.wait_for_selector(".page .span", timeout=15000)

    def test_a_freshly_opened_document_has_nothing_to_lose(self, page):
        assert page.evaluate("() => window.__editor?.hasUnsavedWork") in (False, None)

    def test_leaving_after_an_edit_is_challenged(self, page):
        self._edit_something(page)
        # Playwright answers the browser's own leave prompt; seeing it asked at
        # all is the point.
        asked = []
        page.on("dialog", lambda dialog: (asked.append(dialog.message), dialog.accept()))
        page.evaluate("""() => {
            const event = new Event('beforeunload', { cancelable: true });
            window.dispatchEvent(event);
            window.__leaveWasBlocked = event.defaultPrevented;
        }""")
        assert page.evaluate("() => window.__leaveWasBlocked") is True

    def test_leaving_without_edits_is_not_challenged(self, page):
        page.evaluate("""() => {
            const event = new Event('beforeunload', { cancelable: true });
            window.dispatchEvent(event);
            window.__leaveWasBlocked = event.defaultPrevented;
        }""")
        assert page.evaluate("() => window.__leaveWasBlocked") is False

    def test_opening_another_pdf_over_unsaved_edits_asks_first(self, page, tmp_path):
        self._edit_something(page)
        asked = []
        page.on("dialog", lambda dialog: (asked.append(dialog.message), dialog.dismiss()))
        other = tmp_path / "otro.pdf"
        other.write_bytes(build_pdf())
        page.set_input_files("#file-input", str(other))
        page.wait_for_timeout(600)
        assert asked, "no preguntó nada antes de descartar el trabajo"
        assert "sin guardar" in asked[0]

    def test_the_editing_box_has_spell_checking_on(self, page):
        open_editor(page)
        assert page.eval_on_selector(".span.is-editing .span__input", "e => e.spellcheck") is True


@requires_fonts
class TestFindBar:
    def _search(self, page, query):
        page.keyboard.press("Control+f")
        page.wait_for_selector("#findbar:not([hidden])", timeout=5000)
        page.fill("#find-query", query)
        page.wait_for_timeout(700)

    def test_ctrl_f_opens_it(self, page):
        assert page.locator("#findbar").is_hidden()
        page.keyboard.press("Control+f")
        page.wait_for_selector("#findbar:not([hidden])", timeout=5000)
        assert page.evaluate("() => document.activeElement?.id") == "find-query"

    def test_escape_closes_it(self, page):
        self._search(page, "linea")
        page.keyboard.press("Escape")
        page.wait_for_timeout(300)
        assert page.locator("#findbar").is_hidden()

    def test_results_are_counted_and_highlighted(self, page):
        self._search(page, "linea")
        assert page.locator("#find-count").text_content().startswith("1 de ")
        assert page.locator(".hit").count() >= 2
        assert page.locator(".hit--current").count() == 1

    def test_stepping_moves_the_current_result(self, page):
        self._search(page, "linea")
        first = page.locator("#find-count").text_content()
        page.click("#find-next")
        page.wait_for_timeout(300)
        assert page.locator("#find-count").text_content() != first
        assert page.locator(".hit--current").count() == 1

    def test_a_query_with_no_results_says_so(self, page):
        self._search(page, "zzzznoexiste")
        assert "sin resultados" in page.locator("#find-count").text_content()
        assert page.locator(".hit").count() == 0

    def test_replacing_changes_the_document(self, page):
        page.on("dialog", lambda dialog: dialog.accept())
        self._search(page, "Primera")
        page.fill("#find-replacement", "REEMPLAZADA")
        page.click("#find-replace-all")
        page.wait_for_timeout(3500)
        texts = " ".join(
            page.eval_on_selector_all(".page .span", "els => els.map(e => e.dataset.text)")
        )
        assert "REEMPLAZADA" in texts
        assert "Primera" not in texts

    def test_no_console_errors_while_searching(self, page):
        self._search(page, "linea")
        page.click("#find-next")
        page.wait_for_timeout(400)
        assert page.console_errors == []


@requires_fonts
class TestMoveTool:
    def test_dragging_a_block_moves_it(self, page):
        page.click('[data-tool="move"]')
        target = span_with(page, "Primera linea")
        before = target.bounding_box()

        page.mouse.move(before["x"] + before["width"] / 2, before["y"] + before["height"] / 2)
        page.mouse.down()
        page.mouse.move(
            before["x"] + before["width"] / 2 + 120,
            before["y"] + before["height"] / 2 + 140,
            steps=10,
        )
        assert page.locator(".ghost").count() >= 1, "no se ve dónde caería"
        page.mouse.up()
        page.wait_for_timeout(3500)

        after = span_with(page, "Primera linea").bounding_box()
        assert after["x"] > before["x"] + 60
        assert after["y"] > before["y"] + 80

    def test_the_ghost_disappears_after_dropping(self, page):
        page.click('[data-tool="move"]')
        target = span_with(page, "Primera linea")
        box = target.bounding_box()
        page.mouse.move(box["x"] + 20, box["y"] + box["height"] / 2)
        page.mouse.down()
        page.mouse.move(box["x"] + 90, box["y"] + 100, steps=6)
        page.mouse.up()
        page.wait_for_timeout(3000)
        assert page.locator(".ghost").count() == 0

    def test_moving_does_not_open_the_editor(self, page):
        page.click('[data-tool="move"]')
        target = span_with(page, "Titulo en negrita")
        box = target.bounding_box()
        page.mouse.move(box["x"] + 10, box["y"] + box["height"] / 2)
        page.mouse.down()
        page.mouse.move(box["x"] + 60, box["y"] + 60, steps=5)
        page.mouse.up()
        page.wait_for_timeout(2500)
        assert page.locator(".span.is-editing").count() == 0


@requires_fonts
class TestContinuationPage:
    """Text pushed past the foot of the sheet is carried onto a page of its
    own. Nothing asked for that page, so the editor has to notice it appeared."""

    def test_the_rail_shows_the_page_that_appeared(self, page, tmp_path, browser, server):
        from tests.conftest import build_pdf_with_section_below

        crowded = tmp_path / "apretado.pdf"
        crowded.write_bytes(build_pdf_with_section_below(gap=8, top=700))
        tab = browser.new_page(viewport={"width": 1400, "height": 900})
        try:
            tab.goto(server, wait_until="networkidle")
            tab.set_input_files("#file-input", str(crowded))
            tab.wait_for_selector(".page .span", timeout=30000)
            tab.wait_for_function("() => document.querySelectorAll('.thumb').length === 1", timeout=15000)

            target = tab.locator('.page .span[data-text*="pal00"]').first
            target.click()
            tab.wait_for_selector(".span.is-editing .span__input", timeout=10000)
            tab.select_option("#fmt-fit", "push")
            tab.keyboard.press("End")
            tab.keyboard.type(" " + " ".join(["anadido"] * 30))
            tab.keyboard.press("Enter")
            tab.wait_for_function(
                "() => document.querySelectorAll('.thumb').length === 2", timeout=30000
            )
            assert tab.locator(".page").count() == 2
        finally:
            tab.close()


@requires_fonts
class TestGridAndSnapping:
    """A grid to place things against, and guides read off the page so a block
    can be lined up with what is already there."""

    def _open_bar(self, page):
        page.keyboard.press("g")
        page.wait_for_selector("#gridbar:not([hidden])", timeout=5000)

    def _block_box(self, page):
        return page.evaluate("""() => {
            const view = window.__editor.pages.get(0);
            const block = view.lines.filter(l => l.text.includes('Primera linea'));
            const rects = block.map(l => l.bbox);
            return [Math.min(...rects.map(r=>r[0])), Math.min(...rects.map(r=>r[1])),
                    Math.max(...rects.map(r=>r[2])), Math.max(...rects.map(r=>r[3]))];
        }""")

    def test_the_bar_opens_with_its_key(self, page):
        assert page.locator("#gridbar").is_hidden()
        self._open_bar(page)
        assert page.locator("#grid-step").locator("option").count() > 3

    def test_showing_the_grid_rules_the_page(self, page):
        self._open_bar(page)
        assert page.eval_on_selector(".page__layer", "e => e.style.backgroundImage") == ""
        page.check("#grid-show")
        page.wait_for_timeout(400)
        assert "gradient" in page.eval_on_selector(".page__layer", "e => e.style.backgroundImage")

    def test_hiding_it_again_leaves_the_page_clean(self, page):
        self._open_bar(page)
        page.check("#grid-show")
        page.wait_for_timeout(300)
        page.uncheck("#grid-show")
        page.wait_for_timeout(300)
        assert page.eval_on_selector(".page__layer", "e => e.style.backgroundImage") == ""

    def test_snapping_to_the_grid_lands_on_a_ruled_line(self, page):
        self._open_bar(page)
        page.locator("#grid-snap").set_checked(True)
        page.locator("#grid-guides").set_checked(False)
        result = page.evaluate("""() => {
            const ed = window.__editor, g = ed.grid, view = ed.pages.get(0);
            const block = view.lines.filter(l => l.text.includes('Primera linea'));
            const rects = block.map(l => l.bbox);
            const box = [Math.min(...rects.map(r=>r[0])), Math.min(...rects.map(r=>r[1])),
                         Math.max(...rects.map(r=>r[2])), Math.max(...rects.map(r=>r[3]))];
            const out = g.snapDelta(view, box, 61.3, 44.7, {});
            const edges = [box[0]+out.dx, (box[0]+box[2])/2+out.dx, box[2]+out.dx,
                           box[1]+out.dy, (box[1]+box[3])/2+out.dy, box[3]+out.dy];
            return { step: g.spacing, edges };
        }""")
        step = result["step"]
        assert any(
            abs((edge % step)) < 0.01 or abs((edge % step) - step) < 0.01
            for edge in result["edges"]
        ), "ningún borde quedó sobre una línea de la cuadrícula"

    def test_snapping_to_guides_lands_on_something_already_on_the_page(self, page):
        self._open_bar(page)
        page.locator("#grid-snap").set_checked(False)
        page.locator("#grid-guides").set_checked(True)
        result = page.evaluate("""() => {
            const ed = window.__editor, g = ed.grid, view = ed.pages.get(0);
            const block = view.lines.filter(l => l.text.includes('Primera linea'));
            const rects = block.map(l => l.bbox);
            const box = [Math.min(...rects.map(r=>r[0])), Math.min(...rects.map(r=>r[1])),
                         Math.max(...rects.map(r=>r[2])), Math.max(...rects.map(r=>r[3]))];
            const targets = g.candidates(view, block);
            // Aim just past one of the page's own edges.
            const aim = targets.xs[0] - box[0] + 1.2;
            const out = g.snapDelta(view, box, aim, 0, { exclude: block });
            return { xs: targets.xs, landed: [box[0]+out.dx, box[2]+out.dx], marks: out.marks };
        }""")
        assert result["marks"], "no señaló con qué se alineó"
        assert any(
            any(abs(edge - target) < 0.05 for target in result["xs"])
            for edge in result["landed"]
        ), "no se alineó con ningún borde del documento"

    def test_a_block_never_lines_up_with_itself(self, page):
        self._open_bar(page)
        excluded = page.evaluate("""() => {
            const ed = window.__editor, g = ed.grid, view = ed.pages.get(0);
            const block = view.lines.filter(l => l.text.includes('Primera linea'));
            const own = block.map(l => Math.round(l.bbox[0] * 10) / 10);
            const targets = g.candidates(view, block);
            return own.some(v => targets.xs.includes(v));
        }""")
        assert excluded is False

    def test_shift_keeps_the_drag_on_one_axis(self, page):
        self._open_bar(page)
        result = page.evaluate("""() => {
            const ed = window.__editor, g = ed.grid, view = ed.pages.get(0);
            const box = [72, 90, 200, 105];
            return {
                wide: g.snapDelta(view, box, 80, 9, { constrain: true }),
                tall: g.snapDelta(view, box, 7, 90, { constrain: true }),
            };
        }""")
        assert result["wide"]["dy"] == 0
        assert result["tall"]["dx"] == 0

    def test_the_arrow_keys_move_a_picked_block(self, page):
        page.click('[data-tool="move"]')
        target = span_with(page, "Primera linea")
        target.click()
        page.wait_for_timeout(600)
        assert page.locator(".span.is-picked").count() >= 1, "el clic no marcó nada"

        before = span_with(page, "Primera linea").bounding_box()
        page.keyboard.press("ArrowRight")
        page.wait_for_timeout(3000)
        after = span_with(page, "Primera linea").bounding_box()
        moved = after["x"] - before["x"]
        assert 0.5 < moved < 4, f"se movió {moved} px, se esperaba un punto"

    def test_a_letter_typed_in_a_field_is_not_a_shortcut(self, page):
        """Typing "g" into the search box used to open the grid bar."""
        page.keyboard.press("Control+f")
        page.wait_for_selector("#findbar:not([hidden])", timeout=5000)
        page.locator("#find-query").click()
        page.keyboard.type("gte")
        page.wait_for_timeout(400)
        assert page.locator("#gridbar").is_hidden(), "la G abrió la cuadrícula"
        assert page.locator("#find-query").input_value() == "gte"

    def test_a_plain_click_picks_without_nudging(self, page):
        """Con el ajuste activo, un clic quieto producía un desplazamiento hasta
        la línea más cercana: el bloque se movía solo por haberlo señalado."""
        self._open_bar(page)
        page.locator("#grid-snap").set_checked(True)
        page.click('[data-tool="move"]')
        before = span_with(page, "Primera linea").bounding_box()
        span_with(page, "Primera linea").click()
        page.wait_for_timeout(1500)
        assert page.locator(".span.is-picked").count() >= 1, "el clic no marcó nada"
        after = span_with(page, "Primera linea").bounding_box()
        assert abs(after["x"] - before["x"]) < 0.5, "el clic movió el bloque"
        assert abs(after["y"] - before["y"]) < 0.5, "el clic movió el bloque"


@requires_fonts
class TestAnnotating:
    """Marks put on the page with the mouse, and taken off with it."""

    def _pick_tool(self, page, kind="highlight"):
        page.click('[data-tool="mark"]')
        page.wait_for_selector("#markbar:not([hidden])", timeout=5000)
        page.locator("#mark-kind").select_option(kind)
        page.wait_for_timeout(100)

    def _drag_over(self, page, text):
        """Drag across a line of text the way a highlighter is used."""
        box = span_with(page, text).bounding_box()
        page.mouse.move(box["x"] + 1, box["y"] + box["height"] / 2)
        page.mouse.down()
        page.mouse.move(box["x"] + box["width"] - 1, box["y"] + box["height"] / 2, steps=8)
        page.mouse.up()
        page.wait_for_timeout(2500)

    def _marks(self, page):
        return page.evaluate(
            "() => (window.__editor.pages.get(0).marks || []).map(m => [m.kind, m.color])"
        )

    def _mark_boxes(self, page):
        return page.evaluate(
            "() => (window.__editor.pages.get(0).marks || []).map(m => m.bbox)"
        )

    def test_the_bar_comes_up_with_the_tool(self, page):
        assert page.locator("#markbar").is_hidden()
        self._pick_tool(page)
        assert page.locator("#mark-kind").locator("option").count() == 5

    def test_the_bar_goes_away_with_the_tool(self, page):
        self._pick_tool(page)
        page.click('[data-tool="select"]')
        page.wait_for_timeout(300)
        assert page.locator("#markbar").is_hidden()

    def test_dragging_over_a_line_highlights_it(self, page):
        self._pick_tool(page)
        assert self._marks(page) == []
        self._drag_over(page, "Primera linea")
        assert [kind for kind, _ in self._marks(page)] == ["highlight"]
        # The sweep has no height of its own; the mark takes it from the line.
        box = self._mark_boxes(page)[0]
        assert box[3] - box[1] > 8, f"la marca quedó de {box[3] - box[1]:.1f} pt"

    def test_the_colour_chosen_is_the_colour_drawn(self, page):
        self._pick_tool(page)
        page.locator("#mark-color").evaluate(
            "el => { el.value = '#66ccff'; el.dispatchEvent(new Event('input')); }"
        )
        self._drag_over(page, "Primera linea")
        assert self._marks(page) == [["highlight", "#66ccff"]]

    def test_another_kind_of_mark_can_be_chosen(self, page):
        self._pick_tool(page, "strikeout")
        self._drag_over(page, "Primera linea")
        assert [kind for kind, _ in self._marks(page)] == ["strikeout"]

    def test_clicking_a_mark_takes_it_off(self, page):
        self._pick_tool(page)
        self._drag_over(page, "Primera linea")
        assert len(self._marks(page)) == 1
        page.locator(".page .markhit").first.click()
        page.wait_for_timeout(2500)
        assert self._marks(page) == []

    def test_marks_are_only_clickable_with_the_tool_in_hand(self, page):
        """Otherwise they would swallow every click meant for the text."""
        self._pick_tool(page)
        self._drag_over(page, "Primera linea")
        page.click('[data-tool="select"]')
        page.wait_for_timeout(300)
        clickable = page.eval_on_selector(
            ".page .markhit", "el => getComputedStyle(el).pointerEvents"
        )
        assert clickable == "none"

    def test_the_text_underneath_is_still_there(self, page):
        """A highlight marks the words; it does not replace them."""
        self._pick_tool(page)
        self._drag_over(page, "Primera linea")
        assert span_with(page, "Primera linea").count() >= 1

    def test_undo_takes_the_mark_back_off(self, page):
        self._pick_tool(page)
        self._drag_over(page, "Primera linea")
        page.keyboard.press("Control+z")
        page.wait_for_timeout(2500)
        assert self._marks(page) == []

    def test_its_key_reaches_the_tool(self, page):
        page.keyboard.press("a")
        page.wait_for_timeout(400)
        assert page.locator("#markbar").is_visible()

    def test_no_console_errors_while_annotating(self, page):
        self._pick_tool(page)
        self._drag_over(page, "Primera linea")
        page.locator(".page .markhit").first.click()
        page.wait_for_timeout(1500)
        assert page.console_errors == []


@requires_fonts
class TestAlignment:
    """Several blocks picked at once, lined up or spaced out."""

    def _pick(self, page, texts):
        page.click('[data-tool="move"]')
        for index, text in enumerate(texts):
            target = span_with(page, text)
            target.click(modifiers=["Shift"] if index else [])
            page.wait_for_timeout(500)

    def test_deltas_line_the_left_edges_up(self, page):
        out = page.evaluate("""async () => {
            const m = await import('/static/js/align.js');
            return m.alignmentDeltas('left', [[10, 10, 60, 30], [100, 50, 130, 70]]);
        }""")
        assert out == [[0, 0], [-90, 0]]

    def test_spreading_needs_three_blocks(self, page):
        out = page.evaluate("""async () => {
            const m = await import('/static/js/align.js');
            return m.alignmentDeltas('spread-h', [[10, 10, 60, 30], [100, 50, 130, 70]]);
        }""")
        assert out is None

    def test_spreading_leaves_the_outermost_alone_and_evens_the_gaps(self, page):
        out = page.evaluate("""async () => {
            const m = await import('/static/js/align.js');
            const boxes = [[10, 0, 60, 20], [100, 0, 130, 20], [200, 0, 280, 20]];
            const deltas = m.alignmentDeltas('spread-h', boxes);
            const moved = boxes.map((b, i) => [b[0] + deltas[i][0], b[2] + deltas[i][0]]);
            return { deltas, gaps: [moved[1][0] - moved[0][1], moved[2][0] - moved[1][1]] };
        }""")
        assert out["deltas"][0] == [0, 0] and out["deltas"][2] == [0, 0]
        assert out["gaps"][0] == pytest.approx(out["gaps"][1], abs=0.01)

    def test_an_unknown_arrangement_does_nothing(self, page):
        out = page.evaluate("""async () => {
            const m = await import('/static/js/align.js');
            return m.alignmentDeltas('diagonal', [[0, 0, 1, 1], [2, 2, 3, 3]]);
        }""")
        assert out is None

    def test_one_block_shows_no_bar(self, page):
        page.click('[data-tool="move"]')
        span_with(page, "Primera linea").click()
        page.wait_for_timeout(600)
        assert page.locator("#alignbar").is_hidden()

    def test_shift_clicking_a_second_block_brings_the_bar_up(self, page):
        self._pick(page, ["Primera linea", "Titulo en negrita"])
        assert page.locator("#alignbar").is_visible()
        assert "2 bloques" in page.locator("#align-count").inner_text()

    def test_shift_clicking_the_same_block_again_drops_it(self, page):
        self._pick(page, ["Primera linea", "Titulo en negrita"])
        span_with(page, "Titulo en negrita").click(modifiers=["Shift"])
        page.wait_for_timeout(600)
        assert page.locator("#alignbar").is_hidden()

    def test_a_plain_click_starts_the_selection_over(self, page):
        self._pick(page, ["Primera linea", "Titulo en negrita"])
        span_with(page, "Una linea en sans serif").click()
        page.wait_for_timeout(600)
        assert page.evaluate("() => window.__editor.picked.blocks.length") == 1

    def test_spreading_is_disabled_until_there_are_three(self, page):
        self._pick(page, ["Primera linea", "Titulo en negrita"])
        assert page.locator('[data-arrange="spread-h"]').is_disabled()
        assert page.locator('[data-arrange="left"]').is_enabled()

    def test_aligning_moves_the_text_on_the_page(self, page):
        self._pick(page, ["Primera linea", "Titulo en negrita"])
        before = [
            span_with(page, t).bounding_box()["x"]
            for t in ("Primera linea", "Titulo en negrita")
        ]
        page.click('[data-arrange="right"]')
        page.wait_for_timeout(4000)
        after = [
            span_with(page, t).bounding_box()["x"]
            for t in ("Primera linea", "Titulo en negrita")
        ]
        assert after != before, "no se movió nada"
        right = [
            span_with(page, t).bounding_box()
            for t in ("Primera linea", "Titulo en negrita")
        ]
        edges = [box["x"] + box["width"] for box in right]
        assert edges[0] == pytest.approx(edges[1], abs=4), edges

    def test_the_format_bars_own_alignment_still_works(self, page):
        """Both bars carry alignment buttons, and they once shared an
        attribute. This is the other one, on the path a person takes to it."""
        open_editor(page, "Primera linea")
        before = span_with(page, "Primera linea").bounding_box()["x"]
        # Alignment is within the line's own width, so a line that already
        # fills it has nowhere to go. Shorten it first.
        page.keyboard.press("Control+a")
        page.keyboard.type("Corto")
        page.click('#fmt-align [data-align="right"]')
        page.keyboard.press("Enter")
        page.wait_for_timeout(4000)
        after = span_with(page, "Corto").bounding_box()["x"]
        assert after > before + 5, f"la línea no se alineó a la derecha: {before} -> {after}"
        assert page.console_errors == []

    def test_no_console_errors_while_aligning(self, page):
        self._pick(page, ["Primera linea", "Titulo en negrita"])
        page.click('[data-arrange="left"]')
        page.wait_for_timeout(3000)
        assert page.console_errors == []


@requires_fonts
class TestFillingAFormIn:
    """The document's own form fields, filled in with the mouse and keyboard."""

    @pytest.fixture
    def form_page(self, browser, server, tmp_path):
        tab = browser.new_page(viewport={"width": 1400, "height": 1000})
        errors = []
        tab.on("console", lambda m: errors.append(m.text) if m.type == "error" else None)
        sample = tmp_path / "formulario.pdf"
        sample.write_bytes(build_pdf_with_every_kind_of_field())
        tab.goto(server, wait_until="networkidle")
        tab.set_input_files("#file-input", str(sample))
        tab.wait_for_selector(".page .formfield", timeout=30000)
        tab.console_errors = errors  # type: ignore[attr-defined]
        yield tab
        tab.close()

    def _field(self, page, name):
        return page.evaluate(
            "(name) => (window.__editor.pages.get(0).fields || [])"
            ".filter(f => f.name === name).map(f => f.value)", name,
        )

    def test_every_field_gets_a_control(self, form_page):
        kinds = form_page.evaluate(
            "() => [...document.querySelectorAll('.page .formfield')]"
            ".map(e => e.className.split(' ')[1])"
        )
        assert "formfield--text" in kinds
        assert "formfield--checkbox" in kinds
        assert "formfield--combo" in kinds
        assert kinds.count("formfield--radio") == 3

    def test_a_signature_gets_no_control(self, form_page):
        """Nothing here can sign, so it is shown as the document draws it."""
        assert form_page.locator(".formfield--signature").count() == 0

    def test_typing_into_a_field_reaches_the_document(self, form_page):
        box = form_page.locator(".formfield--text input").first
        box.click()
        form_page.keyboard.press("Control+a")
        form_page.keyboard.type("Beatriz Lopez")
        form_page.keyboard.press("Enter")
        form_page.wait_for_timeout(3000)
        assert self._field(form_page, "nombre") == ["Beatriz Lopez"]

    def test_a_letter_typed_into_a_field_is_not_a_tool_shortcut(self, form_page):
        """"m" in a name used to switch to the move tool."""
        box = form_page.locator(".formfield--text input").first
        box.click()
        form_page.keyboard.press("Control+a")
        form_page.keyboard.type("Guillermo")
        form_page.wait_for_timeout(400)
        assert form_page.evaluate("() => window.__editor.tool") == "select"
        assert box.input_value() == "Guillermo"

    def test_ticking_a_box_reaches_the_document(self, form_page):
        assert self._field(form_page, "acepto") == [False]
        form_page.locator(".formfield--checkbox input").first.click()
        form_page.wait_for_timeout(3000)
        assert self._field(form_page, "acepto") == [True]

    def test_choosing_from_a_list_reaches_the_document(self, form_page):
        form_page.locator(".formfield--list select").first.select_option("Pro")
        form_page.wait_for_timeout(3000)
        assert self._field(form_page, "plan") == ["Pro"]

    def test_a_locked_field_cannot_be_typed_into(self, form_page):
        locked = form_page.locator(".formfield.is-locked input").first
        assert locked.is_disabled()

    def test_ticking_one_radio_unticks_its_siblings(self, form_page):
        form_page.locator(".formfield--radio input").nth(2).click()
        form_page.wait_for_timeout(3000)
        assert self._field(form_page, "respuesta") == [False, False, True]
        form_page.locator(".formfield--radio input").nth(0).click()
        form_page.wait_for_timeout(3000)
        assert self._field(form_page, "respuesta") == [True, False, False]

    def test_filling_a_field_can_be_undone(self, form_page):
        form_page.locator(".formfield--checkbox input").first.click()
        form_page.wait_for_timeout(3000)
        assert self._field(form_page, "acepto") == [True]
        form_page.keyboard.press("Control+z")
        form_page.wait_for_timeout(3000)
        assert self._field(form_page, "acepto") == [False]

    def test_clearing_a_field_sticks(self, form_page):
        box = form_page.locator(".formfield--text input").first
        box.click()
        form_page.keyboard.press("Control+a")
        form_page.keyboard.press("Delete")
        form_page.keyboard.press("Enter")
        form_page.wait_for_timeout(3000)
        assert self._field(form_page, "nombre") == [""]

    def test_the_bars_own_labels_still_work(self, form_page):
        """The form fields and the bars' labels were both called "field", so
        styling the one laid the other over the bar and swallowed its clicks."""
        form_page.locator(".span").first.click()
        form_page.wait_for_selector(".span.is-editing .span__input", timeout=10000)
        form_page.locator("#fmt-size").click()
        assert form_page.evaluate("() => document.activeElement.id") == "fmt-size"

    def test_no_console_errors_while_filling_it_in(self, form_page):
        form_page.locator(".formfield--checkbox input").first.click()
        form_page.wait_for_timeout(2000)
        assert form_page.console_errors == []


@requires_fonts
class TestErasingAsksFirst:
    """Erasing takes the glyphs out of the file rather than covering them, so
    it says what it is about to take and waits for a yes."""

    def _drag_over(self, page, text, margin=4):
        box = span_with(page, text).bounding_box()
        page.mouse.move(box["x"] - margin, box["y"] - margin)
        page.mouse.down()
        page.mouse.move(box["x"] + box["width"] + margin,
                        box["y"] + box["height"] + margin, steps=8)
        page.mouse.up()

    def _erase(self, page, text, answer=True, margin=4):
        asked = []
        page.once("dialog", lambda d: (asked.append(d.message), d.accept() if answer else d.dismiss()))
        page.click('[data-tool="erase"]')
        self._drag_over(page, text, margin)
        page.wait_for_timeout(3500)
        return asked

    def test_it_asks_before_erasing(self, page):
        asked = self._erase(page, "Primera linea")
        assert asked, "borró sin preguntar"

    def test_the_question_quotes_the_text_that_would_go(self, page):
        asked = self._erase(page, "Primera linea", answer=False)
        assert "Primera linea del documento" in asked[0], asked[0]

    def test_the_question_says_it_leaves_nothing_to_recover(self, page):
        asked = self._erase(page, "Primera linea", answer=False)
        assert "recuperarlo" in asked[0], asked[0]

    def test_saying_no_leaves_the_text_alone(self, page):
        self._erase(page, "Primera linea", answer=False)
        assert span_with(page, "Primera linea").count() >= 1, "borró tras decir que no"

    def test_saying_yes_erases_it(self, page):
        self._erase(page, "Primera linea", answer=True)
        assert span_with(page, "Primera linea").count() == 0, "no llegó a borrar"

    def test_an_empty_area_is_not_worth_asking_about(self, page):
        asked = []
        page.once("dialog", lambda d: (asked.append(d.message), d.dismiss()))
        page.click('[data-tool="erase"]')
        box = page.locator(".page").first.bounding_box()
        page.mouse.move(box["x"] + box["width"] - 90, box["y"] + box["height"] - 90)
        page.mouse.down()
        page.mouse.move(box["x"] + box["width"] - 20, box["y"] + box["height"] - 20, steps=6)
        page.mouse.up()
        page.wait_for_timeout(2500)
        assert asked == [], f"preguntó por un rectángulo vacío: {asked}"

    def test_no_console_errors_while_erasing(self, page):
        self._erase(page, "Primera linea")
        assert page.console_errors == []
