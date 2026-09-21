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


@requires_fonts
class TestUsingItWithAFinger:
    """The editor driven by touch, as it would be on a tablet reaching the
    machine it runs on over the network.

    The touches are dispatched through the browser's own input pipeline rather
    than synthesised in the page, so what is exercised is the real path from a
    finger to a handler — including the translation into pointer events, which
    is the whole point of listening for those.
    """

    @pytest.fixture
    def touch_page(self, browser, server, tmp_path):
        context = browser.new_context(has_touch=True, viewport={"width": 1100, "height": 900})
        tab = context.new_page()
        errors = []
        tab.on("console", lambda m: errors.append(m.text) if m.type == "error" else None)
        sample = tmp_path / "ejemplo.pdf"
        sample.write_bytes(build_pdf())
        tab.goto(server, wait_until="networkidle")
        tab.set_input_files("#file-input", str(sample))
        tab.wait_for_selector(".page .span", timeout=30000)
        tab.console_errors = errors  # type: ignore[attr-defined]
        tab.cdp = context.new_cdp_session(tab)  # type: ignore[attr-defined]
        yield tab
        context.close()

    def _touch(self, page, kind, x=0.0, y=0.0):
        page.cdp.send("Input.dispatchTouchEvent", {
            "type": kind,
            "touchPoints": [] if kind == "touchEnd" else [{"x": x, "y": y}],
        })

    def _tap(self, page, x, y):
        self._touch(page, "touchStart", x, y)
        self._touch(page, "touchEnd")

    def _drag(self, page, x, y, dx, dy, steps=6):
        self._touch(page, "touchStart", x, y)
        for step in range(1, steps + 1):
            self._touch(page, "touchMove", x + dx * step / steps, y + dy * step / steps)
        self._touch(page, "touchEnd")

    def _centre(self, page, text):
        box = span_with(page, text).bounding_box()
        return box["x"] + box["width"] / 2, box["y"] + box["height"] / 2

    def test_the_browser_reports_a_touchscreen(self, touch_page):
        assert touch_page.evaluate("() => navigator.maxTouchPoints") >= 1

    def test_a_finger_drags_a_block(self, touch_page):
        """With mouse listeners this did nothing at all: a finger produces no
        mousedown of its own to hang a drag off."""
        touch_page.click('[data-tool="move"]')
        before = span_with(touch_page, "Primera linea").bounding_box()
        self._drag(touch_page, before["x"] + before["width"] / 2,
                   before["y"] + before["height"] / 2, 72, 60)
        touch_page.wait_for_timeout(3500)
        after = span_with(touch_page, "Primera linea").bounding_box()
        assert after["x"] - before["x"] > 20, f"no se movió: {before['x']} -> {after['x']}"
        assert after["y"] - before["y"] > 20

    def test_a_tap_opens_the_editor(self, touch_page):
        self._tap(touch_page, *self._centre(touch_page, "Primera linea"))
        touch_page.wait_for_selector(".span.is-editing .span__input", timeout=10000)

    def test_a_tap_with_the_move_tool_picks_without_shifting(self, touch_page):
        touch_page.click('[data-tool="move"]')
        before = span_with(touch_page, "Primera linea").bounding_box()
        self._tap(touch_page, *self._centre(touch_page, "Primera linea"))
        touch_page.wait_for_timeout(1500)
        assert touch_page.locator(".span.is-picked").count() >= 1, "el toque no marcó nada"
        after = span_with(touch_page, "Primera linea").bounding_box()
        assert abs(after["x"] - before["x"]) < 1, "el toque movió el bloque"

    def test_a_finger_draws_a_marquee(self, touch_page):
        """The text tool's rectangle, drawn with a finger."""
        touch_page.click('[data-tool="text"]')
        box = touch_page.locator(".page").first.bounding_box()
        self._drag(touch_page, box["x"] + 60, box["y"] + 380, 220, 40)
        touch_page.wait_for_timeout(1200)
        assert touch_page.locator(".newbox").count() == 1

    def test_a_finger_highlights_text(self, touch_page):
        touch_page.keyboard.press("a")
        touch_page.wait_for_selector("#markbar:not([hidden])", timeout=5000)
        box = span_with(touch_page, "Primera linea").bounding_box()
        self._drag(touch_page, box["x"] + 2, box["y"] + box["height"] / 2,
                   box["width"] - 4, 0)
        touch_page.wait_for_timeout(3000)
        marks = touch_page.evaluate("() => window.__editor.pages.get(0).marks.length")
        assert marks == 1, f"el dedo no dejó marca ({marks})"

    def test_a_drag_on_the_page_does_not_scroll_it_away(self, touch_page):
        """With a tool that draws or moves, the gesture belongs to the tool;
        letting the browser scroll under it would make them unusable."""
        for tool in ("move", "text", "erase", "mark"):
            touch_page.click(f'[data-tool="{tool}"]')
            touch_page.wait_for_timeout(150)
            blocked = touch_page.evaluate("""() => {
              const page = document.querySelector('.page');
              const layer = page.querySelector('.page__layer');
              const span = page.querySelector('.span');
              return {
                layer: getComputedStyle(layer).touchAction,
                span: span ? getComputedStyle(span).touchAction : null,
              };
            }""")
            assert "none" in (blocked["layer"], blocked["span"]), f"{tool}: {blocked}"

    def test_the_editing_tool_leaves_scrolling_alone(self, touch_page):
        """Nothing is dragged with it, and scrolling is how the document is
        moved about with a finger."""
        touch_page.click('[data-tool="select"]')
        touch_page.wait_for_timeout(150)
        actions = touch_page.evaluate("""() => {
          const page = document.querySelector('.page');
          return [getComputedStyle(page.querySelector('.page__layer')).touchAction,
                  getComputedStyle(page.querySelector('.span')).touchAction];
        }""")
        assert "none" not in actions, actions

    def test_what_can_be_touched_shows_without_a_hover(self, touch_page):
        """There is no pointer to hover with, so nothing would ever reveal it."""
        touch_page.click('[data-tool="select"]')
        visible = touch_page.evaluate(
            "() => getComputedStyle(document.querySelector('.span')).backgroundColor"
        )
        assert visible not in ("rgba(0, 0, 0, 0)", "transparent"), visible

    def test_a_form_field_can_be_filled_in_with_a_finger(self, browser, server, tmp_path):
        context = browser.new_context(has_touch=True, viewport={"width": 1100, "height": 900})
        tab = context.new_page()
        sample = tmp_path / "formulario.pdf"
        sample.write_bytes(build_pdf_with_every_kind_of_field())
        tab.goto(server, wait_until="networkidle")
        tab.set_input_files("#file-input", str(sample))
        tab.wait_for_selector(".page .formfield", timeout=30000)
        try:
            tab.tap(".formfield--checkbox input")
            tab.wait_for_timeout(3000)
            values = tab.evaluate(
                "() => window.__editor.pages.get(0).fields"
                ".filter(f => f.name === 'acepto').map(f => f.value)"
            )
            assert values == [True]
        finally:
            context.close()

    def test_no_console_errors_under_touch(self, touch_page):
        touch_page.click('[data-tool="move"]')
        box = span_with(touch_page, "Primera linea").bounding_box()
        self._drag(touch_page, box["x"] + 10, box["y"] + 6, 60, 40)
        touch_page.wait_for_timeout(2500)
        assert touch_page.console_errors == []

    def test_blocks_can_be_added_to_the_selection_without_a_keyboard(self, touch_page):
        """Shift and Ctrl are not on a tablet's screen, so the toolbar carries
        the same choice as a switch that stays down."""
        touch_page.click('[data-tool="move"]')
        assert touch_page.locator("#btn-add-pick").is_visible()
        self._tap(touch_page, *self._centre(touch_page, "Primera linea"))
        touch_page.wait_for_timeout(1200)
        touch_page.tap("#btn-add-pick")
        self._tap(touch_page, *self._centre(touch_page, "Titulo en negrita"))
        touch_page.wait_for_timeout(1200)
        assert touch_page.evaluate("() => window.__editor.picked.blocks.length") == 2
        assert touch_page.locator("#alignbar").is_visible()

    def test_the_switch_is_only_offered_where_it_means_something(self, touch_page):
        touch_page.click('[data-tool="select"]')
        touch_page.wait_for_timeout(200)
        assert touch_page.locator("#btn-add-pick").is_hidden()

    def test_leaving_the_move_tool_turns_the_switch_off(self, touch_page):
        touch_page.click('[data-tool="move"]')
        touch_page.tap("#btn-add-pick")
        assert touch_page.evaluate("() => window.__editor.addToSelection") is True
        touch_page.click('[data-tool="select"]')
        touch_page.click('[data-tool="move"]')
        touch_page.wait_for_timeout(200)
        assert touch_page.evaluate("() => window.__editor.addToSelection") is False


@requires_fonts
class TestWarningAboutASignedDocument:
    """A standing banner, not a toast that clears itself: a signature does not
    become valid again by waiting."""

    def _signed_pdf(self):
        import pymupdf as _pymupdf

        from tests.conftest import build_pdf_with_every_kind_of_field, field_named, sign_field

        doc = _pymupdf.open(stream=build_pdf_with_every_kind_of_field(), filetype="pdf")
        xref = field_named(doc, "firma")["xref"]
        sign_field(doc, xref)
        data = doc.tobytes()
        doc.close()
        return data

    def _open(self, page, tmp_path, data, name="firmado.pdf"):
        sample = tmp_path / name
        sample.write_bytes(data)
        page.set_input_files("#file-input", str(sample))
        page.wait_for_selector(".page .formfield", timeout=30000)

    def test_a_signed_document_shows_the_banner(self, page, tmp_path):
        self._open(page, tmp_path, self._signed_pdf())
        assert page.locator("#signed-banner").is_visible()
        assert "Cesar Ruiz" in page.locator("#signed-names").inner_text()

    def test_an_unsigned_document_shows_nothing(self, page):
        assert page.locator("#signed-banner").is_hidden()

    def test_the_banner_stays_up_after_an_edit(self, page, tmp_path):
        self._open(page, tmp_path, self._signed_pdf())
        page.locator(".formfield--checkbox input").first.click()
        page.wait_for_timeout(3000)
        assert page.locator("#signed-banner").is_visible()

    def test_no_console_errors(self, page, tmp_path):
        self._open(page, tmp_path, self._signed_pdf())
        assert page.console_errors == []


@requires_fonts
class TestDuplicatingABlock:
    """Ctrl+D, or the toolbar button, copies whatever is picked."""

    def _pick(self, page, text="Primera linea"):
        page.click('[data-tool="move"]')
        span_with(page, text).click()
        page.wait_for_timeout(600)

    def test_the_button_is_hidden_with_nothing_picked(self, page):
        page.click('[data-tool="move"]')
        assert page.locator("#btn-duplicate").is_hidden()

    def test_picking_a_block_shows_the_button(self, page):
        self._pick(page)
        assert page.locator("#btn-duplicate").is_visible()

    def test_the_button_duplicates_it(self, page):
        self._pick(page)
        before = page.locator('.span[data-text*="Primera linea"]').count()
        page.click("#btn-duplicate")
        page.wait_for_timeout(3500)
        after = page.locator('.span[data-text*="Primera linea"]').count()
        assert after == before + 1

    def test_ctrl_d_duplicates_it(self, page):
        self._pick(page)
        before = page.locator('.span[data-text*="Primera linea"]').count()
        page.keyboard.press("Control+d")
        page.wait_for_timeout(3500)
        after = page.locator('.span[data-text*="Primera linea"]').count()
        assert after == before + 1

    def test_the_copy_is_offset_from_the_original(self, page):
        self._pick(page)
        before = span_with(page, "Primera linea").bounding_box()
        page.click("#btn-duplicate")
        page.wait_for_timeout(3500)
        boxes = page.locator('.span[data-text*="Primera linea"]').all()
        positions = [b.bounding_box() for b in boxes]
        assert len(positions) == 2
        assert any(
            abs(p["y"] - before["y"]) > 5 or abs(p["x"] - before["x"]) > 5
            for p in positions
        ), "la copia quedó encima del original"

    def test_ctrl_d_does_nothing_with_no_selection(self, page):
        page.click('[data-tool="move"]')
        before = page.locator('.span[data-text*="Primera linea"]').count()
        page.keyboard.press("Control+d")
        page.wait_for_timeout(1500)
        assert page.locator('.span[data-text*="Primera linea"]').count() == before

    def test_ctrl_d_in_a_text_field_is_not_hijacked(self, page):
        """Typing into a field should never trigger a page shortcut."""
        page.keyboard.press("Control+f")
        page.wait_for_selector("#findbar:not([hidden])", timeout=5000)
        page.locator("#find-query").click()
        page.keyboard.type("algo")
        page.keyboard.press("Control+d")
        page.wait_for_timeout(400)
        assert page.locator("#find-query").input_value() == "algo"

    def test_no_console_errors_while_duplicating(self, page):
        self._pick(page)
        page.click("#btn-duplicate")
        page.wait_for_timeout(2500)
        assert page.console_errors == []


@requires_fonts
class TestFittingAPhoneScreen:
    """The desktop default zoom runs a page wider than a phone screen. What a
    narrow screen sees from the very first frame is what was reported broken:
    a slice out of the middle of a line instead of the page."""

    @pytest.fixture
    def phone_page(self, browser, server, tmp_path):
        tab = browser.new_page(viewport={"width": 412, "height": 915}, has_touch=True)
        errors = []
        tab.on("console", lambda m: errors.append(m.text) if m.type == "error" else None)
        sample = tmp_path / "ejemplo.pdf"
        sample.write_bytes(build_pdf())
        tab.goto(server, wait_until="networkidle")
        tab.set_input_files("#file-input", str(sample))
        tab.wait_for_selector(".page .span", timeout=30000)
        tab.console_errors = errors  # type: ignore[attr-defined]
        yield tab
        tab.close()

    def test_the_page_fits_inside_the_screen(self, phone_page):
        page_box = phone_page.locator(".page").first.bounding_box()
        viewport = phone_page.viewport_size
        assert page_box["width"] <= viewport["width"], (
            f"la página mide {page_box['width']} px en una pantalla de {viewport['width']} px"
        )

    def test_the_zoom_field_shows_ajustar(self, phone_page):
        assert phone_page.locator("#zoom").input_value() == "fit"

    def test_a_wide_screen_keeps_the_usual_default(self, page):
        """The desktop default is untouched: only a narrow screen changes."""
        assert page.locator("#zoom").input_value() == "1.5"
        page_box = page.locator(".page").first.bounding_box()
        assert page_box["width"] > 700, "el ancho por defecto cambió también en escritorio"

    def test_rotating_the_phone_refits(self, phone_page):
        phone_page.set_viewport_size({"width": 915, "height": 412})
        phone_page.wait_for_timeout(600)
        phone_page.evaluate("window.dispatchEvent(new Event('resize'))")
        phone_page.wait_for_timeout(3000)
        page_box = phone_page.locator(".page").first.bounding_box()
        assert page_box["width"] <= 915

    def test_picking_a_zoom_by_hand_is_not_overridden_by_a_resize(self, phone_page):
        phone_page.locator("#zoom").select_option("1")
        phone_page.wait_for_timeout(1500)
        width_before = phone_page.locator(".page").first.bounding_box()["width"]
        phone_page.set_viewport_size({"width": 500, "height": 915})
        phone_page.evaluate("window.dispatchEvent(new Event('resize'))")
        phone_page.wait_for_timeout(600)
        width_after = phone_page.locator(".page").first.bounding_box()["width"]
        assert width_after == pytest.approx(width_before, abs=1), "un zoom elegido a mano se pisó solo"

    def test_no_console_errors_on_a_phone(self, phone_page):
        assert phone_page.console_errors == []


@requires_fonts
class TestPlacingTheCaretWithAFinger:
    """Which two letters the caret goes between is the one thing a finger is
    worst at: the fingertip covers exactly what it is aiming at, and a PDF's
    body text on a phone is a few pixels tall."""

    @pytest.fixture
    def phone(self, browser, server, tmp_path):
        context = browser.new_context(viewport={"width": 412, "height": 915}, has_touch=True)
        tab = context.new_page()
        errors = []
        tab.on("console", lambda m: errors.append(m.text) if m.type == "error" else None)
        sample = tmp_path / "ejemplo.pdf"
        sample.write_bytes(build_pdf(pages=3))
        tab.goto(server, wait_until="networkidle")
        tab.set_input_files("#file-input", str(sample))
        tab.wait_for_selector(".page .span", timeout=30000)
        # Fitting the width re-renders every page, which takes the spans down
        # and puts fresh ones up: measuring one before that has settled gets a
        # box that belongs to nothing.
        tab.wait_for_function(
            "() => document.getElementById('zoom').value === 'fit'", timeout=15000
        )
        tab.wait_for_timeout(800)
        tab.wait_for_selector(".page .span", timeout=30000)
        tab.console_errors = errors  # type: ignore[attr-defined]
        tab.cdp = context.new_cdp_session(tab)  # type: ignore[attr-defined]
        yield tab
        context.close()

    def _touch(self, page, kind, x=0.0, y=0.0):
        page.cdp.send("Input.dispatchTouchEvent", {
            "type": kind,
            "touchPoints": [] if kind == "touchEnd" else [{"x": x, "y": y}],
        })

    def _caret(self, page):
        return page.evaluate("""() => {
            const selection = getSelection();
            const input = document.querySelector('.span__input');
            if (!input || !selection.rangeCount) return null;
            if (!input.contains(selection.anchorNode)) return null;
            return selection.getRangeAt(0).startOffset;
        }""")

    def _open_and_hold(self, page, text="Primera linea", from_left=4):
        """Touch a span and wait for the box and the dwell timer."""
        box = span_with(page, text).bounding_box()
        start = (box["x"] + from_left, box["y"] + box["height"] / 2)
        self._touch(page, "touchStart", *start)
        page.wait_for_timeout(900)
        return start

    def test_dragging_sideways_walks_the_caret(self, phone):
        start = self._open_and_hold(phone)
        before = self._caret(phone)
        for step in range(1, 9):
            self._touch(phone, "touchMove", start[0] + step * 8, start[1])
            phone.wait_for_timeout(50)
        after = self._caret(phone)
        self._touch(phone, "touchEnd")
        assert before is not None and after is not None
        assert after > before, f"el cursor no avanzó: {before} -> {after}"

    def test_dragging_back_walks_it_back(self, phone):
        start = self._open_and_hold(phone)
        for step in range(1, 9):
            self._touch(phone, "touchMove", start[0] + step * 8, start[1])
            phone.wait_for_timeout(40)
        far = self._caret(phone)
        for step in range(8, 0, -1):
            self._touch(phone, "touchMove", start[0] + step * 4, start[1])
            phone.wait_for_timeout(40)
        back = self._caret(phone)
        self._touch(phone, "touchEnd")
        assert back < far, f"no volvió: {far} -> {back}"

    def test_the_caret_stays_where_the_finger_lifted(self, phone):
        start = self._open_and_hold(phone)
        for step in range(1, 7):
            self._touch(phone, "touchMove", start[0] + step * 7, start[1])
            phone.wait_for_timeout(40)
        at_lift = self._caret(phone)
        self._touch(phone, "touchEnd")
        phone.wait_for_timeout(300)
        assert self._caret(phone) == at_lift

    def test_the_magnifier_shows_what_is_being_edited(self, phone):
        start = self._open_and_hold(phone)
        self._touch(phone, "touchMove", start[0] + 30, start[1])
        phone.wait_for_timeout(200)
        assert phone.locator(".loupe").count() == 1
        shown = phone.locator(".loupe__text").inner_text()
        self._touch(phone, "touchEnd")
        assert "Primera linea" in shown

    def test_the_magnifier_floats_above_the_finger(self, phone):
        """Under it, it would be hidden by the hand it exists to see past."""
        start = self._open_and_hold(phone)
        self._touch(phone, "touchMove", start[0] + 30, start[1])
        phone.wait_for_timeout(200)
        loupe = phone.locator(".loupe").bounding_box()
        self._touch(phone, "touchEnd")
        assert loupe["y"] + loupe["height"] < start[1], "la lupa tapa el dedo"

    def test_the_magnifier_keeps_the_caret_in_its_middle(self, phone):
        start = self._open_and_hold(phone)
        offsets = []
        for step in range(1, 7):
            self._touch(phone, "touchMove", start[0] + step * 9, start[1])
            phone.wait_for_timeout(60)
            offsets.append(phone.eval_on_selector(".loupe__text", "e => parseFloat(e.style.left)"))
        self._touch(phone, "touchEnd")
        assert offsets[-1] < offsets[0], f"el texto no se desplazó bajo la marca: {offsets}"

    def test_the_magnifier_goes_away_when_the_finger_lifts(self, phone):
        start = self._open_and_hold(phone)
        self._touch(phone, "touchMove", start[0] + 30, start[1])
        phone.wait_for_timeout(200)
        assert phone.locator(".loupe").count() == 1
        self._touch(phone, "touchEnd")
        phone.wait_for_timeout(300)
        assert phone.locator(".loupe").count() == 0

    def test_resting_a_finger_still_brings_it_up(self, phone):
        """A finger held still is asking as plainly as one already sliding."""
        self._open_and_hold(phone)
        assert phone.locator(".loupe").count() == 1
        self._touch(phone, "touchEnd")

    def test_the_page_still_scrolls_under_a_finger_on_text(self, phone):
        """The sideways part of the gesture was claimed, not the whole of it:
        a page of text is mostly text, and scrolling it has to keep working."""
        box = span_with(phone, "Primera linea").bounding_box()
        x, y = box["x"] + box["width"] / 2, box["y"] + box["height"] / 2
        before = phone.evaluate("() => document.getElementById('canvas-area').scrollTop")
        self._touch(phone, "touchStart", x, y)
        for step in range(1, 10):
            self._touch(phone, "touchMove", x, y - step * 25)
            phone.wait_for_timeout(25)
        self._touch(phone, "touchEnd")
        phone.wait_for_timeout(400)
        after = phone.evaluate("() => document.getElementById('canvas-area').scrollTop")
        assert after > before, f"la página dejó de desplazarse: {before} -> {after}"

    def test_a_mouse_never_summons_it(self, page):
        """A pointer is precise and is not hiding anything behind a hand."""
        open_editor(page, "Primera linea")
        box = span_with(page, "Primera linea").bounding_box()
        page.mouse.move(box["x"] + 4, box["y"] + box["height"] / 2)
        page.mouse.down()
        page.mouse.move(box["x"] + 60, box["y"] + box["height"] / 2, steps=6)
        page.wait_for_timeout(400)
        assert page.locator(".loupe").count() == 0
        page.mouse.up()

    def test_no_console_errors(self, phone):
        start = self._open_and_hold(phone)
        self._touch(phone, "touchMove", start[0] + 40, start[1])
        phone.wait_for_timeout(200)
        self._touch(phone, "touchEnd")
        assert phone.console_errors == []

    def _tap(self, page, x, y, hold=80):
        """A quick tap, the length a real one lasts — not a deliberate hold."""
        self._touch(page, "touchStart", x, y)
        page.wait_for_timeout(hold)
        self._touch(page, "touchEnd")
        page.wait_for_timeout(400)

    def test_a_quick_tap_inside_an_open_box_moves_the_caret(self, phone):
        """Taking the gesture over means the browser stops placing a caret of
        its own, so a tap that places none leaves it wherever it was — and
        whatever is typed next lands in the wrong place."""
        box = span_with(phone, "Primera linea").bounding_box()
        middle = box["y"] + box["height"] / 2
        self._tap(phone, box["x"] + 4, middle)
        near_start = self._caret(phone)
        self._tap(phone, box["x"] + box["width"] - 6, middle)
        near_end = self._caret(phone)
        assert near_start is not None and near_end is not None
        assert near_end > near_start + 5, f"el toque no movió el cursor: {near_start} -> {near_end}"

    def test_what_is_typed_lands_where_the_tap_was(self, phone):
        box = span_with(phone, "Primera linea").bounding_box()
        middle = box["y"] + box["height"] / 2
        self._tap(phone, box["x"] + 4, middle)
        self._tap(phone, box["x"] + box["width"] / 2, middle)
        phone.keyboard.type("XY")
        phone.wait_for_timeout(300)
        written = phone.evaluate("() => document.querySelector('.span__input').textContent")
        assert not written.startswith("XY"), f"escribió al principio: {written!r}"
        assert "XY" in written and written.index("XY") > 4, written

    def test_a_quick_tap_does_not_flash_the_magnifier(self, phone):
        box = span_with(phone, "Primera linea").bounding_box()
        middle = box["y"] + box["height"] / 2
        self._tap(phone, box["x"] + 4, middle)
        seen = []
        self._touch(phone, "touchStart", box["x"] + 30, middle)
        phone.wait_for_timeout(120)
        seen.append(phone.locator(".loupe").count())
        self._touch(phone, "touchEnd")
        assert seen == [0], "la lupa salió en un toque normal"

    def test_tapping_another_line_moves_the_editing_there(self, phone):
        box = span_with(phone, "Primera linea").bounding_box()
        self._tap(phone, box["x"] + 20, box["y"] + box["height"] / 2)
        other = span_with(phone, "Segunda linea").bounding_box()
        self._tap(phone, other["x"] + 20, other["y"] + other["height"] / 2)
        phone.wait_for_timeout(600)
        editing = phone.evaluate(
            "() => document.querySelector('.span.is-editing')?.dataset.text"
        )
        assert editing is not None and "Segunda linea" in editing, editing
