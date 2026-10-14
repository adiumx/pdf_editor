"""Dev-only oracle: compare pdfcore's text runs with PyMuPDF's spans.

PyMuPDF is never part of the shipped engine (AGPL); it only serves here as an
independent opinion on what a page says and where.

    python3 tools/compare_with_pymupdf.py file.pdf [...]
"""
import re, subprocess, sys, pathlib
import pymupdf

DUMP = pathlib.Path(__file__).resolve().parents[1] / "target/debug/examples/dump"

def norm(s): return re.sub(r"\s+", "", s)

def rust_runs(path):
    out = subprocess.run([str(DUMP), path], capture_output=True, text=True, check=True).stdout
    runs = {}
    for line in out.splitlines():
        p = line.split("\t")
        runs.setdefault(int(p[0]), []).append((float(p[1]), float(p[2]), float(p[3]), float(p[4]), p[7] if len(p) > 7 else ""))
    return runs

def compare(path):
    doc = pymupdf.open(path)
    runs = rust_runs(path)
    worst_total = 0
    for pno in range(doc.page_count):
        theirs = norm(doc[pno].get_text())
        ours = norm("".join(r[4] for r in runs.get(pno, [])))
        spans = [s for b in doc[pno].get_text("dict")["blocks"] if b["type"] == 0
                 for l in b["lines"] for s in l["spans"]]
        matched = deltas = 0
        worst = 0.0
        for x0, y0, x1, y1, text in runs.get(pno, []):
            key = norm(text)
            if not key: continue
            cands = [s for s in spans if norm(s["text"]) == key]
            if not cands: continue
            matched += 1
            best = min(cands, key=lambda s: abs(s["bbox"][0] - x0) + abs(s["bbox"][1] - y0))
            d = max(abs(best["bbox"][0] - x0), abs(best["bbox"][2] - x1))
            worst = max(worst, d)
        same = ours == theirs
        # text may differ only in order; compare as sorted character counts
        same_chars = sorted(ours) == sorted(theirs)
        print(f"  page {pno}: texto igual={same} mismos caracteres={same_chars} "
              f"({len(ours)} vs {len(theirs)}) runs con equivalente={matched}/{len(runs.get(pno, []))} "
              f"peor desviación horizontal={worst:.1f}pt")
        if not same_chars:
            from collections import Counter
            diff = (Counter(theirs) - Counter(ours)), (Counter(ours) - Counter(theirs))
            print("     faltan:", dict(diff[0]), " sobran:", dict(diff[1]))

for p in sys.argv[1:]:
    print(p); compare(p)
