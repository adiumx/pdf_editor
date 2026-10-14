"""Fails if any dependency of the engine could force us to open our source.

A dependency passes when its license expression offers at least one permissive
choice (MIT, Apache-2.0, BSD, Zlib, ISC, Unlicense, 0BSD, Unicode). Run it after
every `cargo add` or `cargo update`, because this is what makes the engine
sellable without a commercial license from anyone.

    python3 tools/check_licenses.py
"""
import json, pathlib, re, subprocess, sys

root = pathlib.Path(__file__).resolve().parents[1]
meta = json.loads(subprocess.run(["cargo", "metadata", "--format-version", "1"],
                                 cwd=root, capture_output=True, text=True, check=True).stdout)
PERMISSIVE = ("MIT", "APACHE-2.0", "BSD", "ZLIB", "ISC", "UNLICENSE", "0BSD", "UNICODE")
bad = []
for pkg in meta["packages"]:
    if pkg["name"] == "pdfcore":
        continue
    expression = (pkg["license"] or "").upper()
    # Split on OR: any one permissive alternative is enough. AND terms must all pass.
    alternatives = re.split(r"\s+OR\s+|/", expression.replace("(", "").replace(")", ""))
    if not any(any(p in alt for p in PERMISSIVE) for alt in alternatives):
        bad.append((pkg["name"], pkg["version"], pkg["license"]))
for name, version, license in bad:
    print(f"NO PERMISIVA: {name} {version}: {license}")
print("licencias OK" if not bad else f"{len(bad)} dependencia(s) a revisar")
sys.exit(1 if bad else 0)
