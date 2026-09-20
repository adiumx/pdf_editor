#!/usr/bin/env python3
"""Start the editor and open it in a browser.

    python run.py [--port 8000] [--host 127.0.0.1] [--no-browser]
"""

from __future__ import annotations

import argparse
import socket
import threading
import webbrowser


def lan_address() -> str | None:
    """This machine's address on the local network, as others would reach it.

    Found by asking the routing table which interface a packet would leave by;
    nothing is sent, and the address is not looked up anywhere. Returns None
    when there is no route out, which is the case on a machine with no network.
    """
    try:
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as probe:
            probe.connect(("192.0.2.1", 9))  # reserved for documentation, routed nowhere
            return probe.getsockname()[0]
    except OSError:
        return None


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

    # Listening on every interface is how a tablet on the same network reaches
    # the editor, so the address it should be opened at is worth printing —
    # along with what else that opens, since there is no password on any of it.
    if args.host == "0.0.0.0":
        address = lan_address()
        if address:
            print(f"Desde otro equipo de tu red: http://{address}:{args.port}")
        print(
            "Atención: cualquiera que alcance esta red puede abrir el editor,\n"
            "          subir archivos y leer los documentos que tengas abiertos.\n"
            "          No hay contraseña. Úsalo solo en una red de confianza."
        )
    import uvicorn

    uvicorn.run("app.main:app", host=args.host, port=args.port, reload=args.reload, log_level="info")


if __name__ == "__main__":
    main()
