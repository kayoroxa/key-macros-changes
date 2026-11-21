"""Servidor local do Tap/Hold Lab.

Serve a página em http://localhost:8765 e guarda os dados da aba "Caçador de combos"
num arquivo em disco (data/combo-data.json). Assim os dados valem em qualquer navegador.

Proteções contra perda de dados:
  * recusa (409) gravações que tenham MENOS teclas que o arquivo atual (página desatualizada),
    a não ser que a página peça explicitamente ("Zerar dados");
  * antes de sobrescrever, guarda cópia em data/backups/ (no máx. 1 a cada 10 min, últimas 40;
    sempre que for uma gravação forçada);
  * escrita atômica (arquivo temporário + replace).

Uso:  python serve.py [--port 8765] [--data-dir PASTA] [--no-browser]
"""
import argparse
import glob
import json
import os
import shutil
import socket
import threading
import time
import webbrowser
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

ROOT = os.path.dirname(os.path.abspath(__file__))
HTML = os.path.join(ROOT, "kanata_tap_hold_simulator.html")
EMPTY = {"keys": {}, "pairs": {}, "labels": {}}
LOCK = threading.Lock()
MAX_BODY = 8 * 1024 * 1024


def total(d):
    try:
        return sum(int(v) for v in d["keys"].values())
    except Exception:
        return 0


def make_handler(port, data_file, bak_dir):
    allowed_hosts = {f"localhost:{port}", f"127.0.0.1:{port}"}

    def backup(force):
        if not os.path.exists(data_file):
            return
        os.makedirs(bak_dir, exist_ok=True)
        files = sorted(glob.glob(os.path.join(bak_dir, "combo-data-*.json")))
        if not force and files and time.time() - os.path.getmtime(files[-1]) < 600:
            return
        name = time.strftime("combo-data-%Y%m%d-%H%M%S.json")
        shutil.copy2(data_file, os.path.join(bak_dir, name))
        files = sorted(glob.glob(os.path.join(bak_dir, "combo-data-*.json")))
        for old in files[:-40]:
            try:
                os.remove(old)
            except OSError:
                pass

    class Handler(BaseHTTPRequestHandler):
        server_version = "TapHoldLab/1"

        def log_message(self, *a):
            pass

        def _send(self, code, body, ctype="application/json; charset=utf-8"):
            if isinstance(body, (dict, list)):
                body = json.dumps(body).encode("utf-8")
            elif isinstance(body, str):
                body = body.encode("utf-8")
            self.send_response(code)
            self.send_header("Content-Type", ctype)
            self.send_header("Content-Length", str(len(body)))
            self.send_header("Cache-Control", "no-store")
            self.end_headers()
            self.wfile.write(body)

        def _host_ok(self):
            if self.headers.get("Host", "") in allowed_hosts:
                return True
            self._send(403, {"error": "host"})
            return False

        def do_GET(self):
            if not self._host_ok():
                return
            path = self.path.split("?")[0]
            if path in ("/", "/index.html"):
                with open(HTML, "rb") as f:
                    self._send(200, f.read(), "text/html; charset=utf-8")
            elif path == "/api/data":
                with LOCK:
                    if os.path.exists(data_file):
                        with open(data_file, "rb") as f:
                            self._send(200, f.read())
                    else:
                        self._send(200, EMPTY)
            else:
                self._send(404, {"error": "not found"})

        def do_PUT(self):
            if not self._host_ok():
                return
            if self.path.split("?")[0] != "/api/data":
                return self._send(404, {"error": "not found"})
            # Cabeçalho custom: força preflight CORS, então outros sites não conseguem gravar aqui.
            if self.headers.get("X-Requested-With") != "tap-hold-lab":
                return self._send(403, {"error": "header"})
            try:
                n = int(self.headers.get("Content-Length", "0"))
                if n <= 0 or n > MAX_BODY:
                    raise ValueError
                new = json.loads(self.rfile.read(n).decode("utf-8"))
                if not isinstance(new.get("keys"), dict) or not isinstance(new.get("pairs"), dict):
                    raise ValueError
            except Exception:
                return self._send(400, {"error": "payload inválido"})
            force = self.headers.get("X-Force") == "1"
            with LOCK:
                cur = EMPTY
                if os.path.exists(data_file):
                    try:
                        with open(data_file, "r", encoding="utf-8") as f:
                            cur = json.load(f)
                    except Exception:
                        cur = EMPTY
                        shutil.copy2(data_file, data_file + ".corrompido-" + time.strftime("%Y%m%d-%H%M%S"))
                if not force and total(new) < total(cur):
                    return self._send(409, {"error": "disco tem mais dados", "disk_total": total(cur)})
                backup(force)
                os.makedirs(os.path.dirname(data_file), exist_ok=True)
                tmp = data_file + ".tmp"
                with open(tmp, "w", encoding="utf-8") as f:
                    json.dump(new, f)
                os.replace(tmp, data_file)
            self._send(200, {"ok": True, "total": total(new)})

    return Handler


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", type=int, default=8765)
    ap.add_argument("--data-dir", default=os.environ.get("TAPHOLD_DATA_DIR") or os.path.join(ROOT, "data"))
    ap.add_argument("--no-browser", action="store_true")
    a = ap.parse_args()
    url = f"http://localhost:{a.port}/"
    data_file = os.path.join(a.data_dir, "combo-data.json")
    bak_dir = os.path.join(a.data_dir, "backups")

    with socket.socket() as s:
        if s.connect_ex(("127.0.0.1", a.port)) == 0:
            print(f"Já existe algo rodando em {url} — abrindo o navegador.")
            if not a.no_browser:
                webbrowser.open(url)
            return

    srv = ThreadingHTTPServer(("127.0.0.1", a.port), make_handler(a.port, data_file, bak_dir))
    print(f"Tap/Hold Lab rodando em {url}")
    print(f"Dados salvos em: {data_file}")
    print("Deixe esta janela aberta enquanto usa o site. Ctrl+C para parar.")
    if not a.no_browser:
        threading.Timer(0.6, lambda: webbrowser.open(url)).start()
    try:
        srv.serve_forever()
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
