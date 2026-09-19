#!/usr/bin/env python3
"""Start the editor and open it in a browser.

    python run.py [--port 8000] [--host 127.0.0.1] [--no-browser]
"""

from __future__ import annotations

import argparse
import threading
import webbrowser


def main() -> None:
    parser = argparse.ArgumentParser(description="Editor de PDF local")
    parser.add_argument("--host", default="127.0.0.1", help="Interfaz de escucha")
    parser.add_argument("--port", type=int, default=8000, help="Puerto")
    parser.add_argument("--no-browser", action="store_true", help="No abrir el navegador")
    parser.add_argument("--reload", action="store_true", help="Recargar al cambiar el código")
    args = parser.parse_args()

    url = f"http://{'localhost' if args.host in {'127.0.0.1', '0.0.0.0'} else args.host}:{args.port}"
    if not args.no_browser and not args.reload:
        threading.Timer(1.0, lambda: webbrowser.open(url)).start()

    print(f"Editor de PDF disponible en {url}")
    import uvicorn

    uvicorn.run("app.main:app", host=args.host, port=args.port, reload=args.reload, log_level="info")


if __name__ == "__main__":
    main()
