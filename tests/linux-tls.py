#!/usr/bin/env python3
"""Qualify the actual HTTPS/WSS client transports against isolated TLS fixtures.

Run on the host with --client pointing at the Fedora-built tls-check example and
--podman-image pointing at its Fedora builder. No real credentials or trust-store
changes are used. Private keys live only in the temporary fixture directory.
"""
import argparse
import base64
import hashlib
import http.server
import json
from pathlib import Path
import ssl
import subprocess
import tempfile
import threading


class Fixture(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *_):
        pass

    def do_GET(self):
        self.server.negotiated.append({"path": self.path, "version": self.connection.version(),
                                       "cipher": self.connection.cipher()[0]})
        if self.path == "/engine/workspaces":
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", "2")
            self.end_headers()
            self.wfile.write(b"[]")
        elif self.path == "/engine/ws":
            key = self.headers["Sec-WebSocket-Key"]
            accept = base64.b64encode(hashlib.sha1((key + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11").encode()).digest()).decode()
            self.send_response(101)
            self.send_header("Upgrade", "websocket")
            self.send_header("Connection", "Upgrade")
            self.send_header("Sec-WebSocket-Accept", accept)
            self.end_headers()
            self.close_connection = True
        else:
            self.send_error(404)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--client", type=Path, required=True)
    parser.add_argument("--podman-image", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    client = args.client.resolve()
    root = Path(__file__).resolve().parents[1]
    relative_client = client.relative_to(root)
    results = []
    with tempfile.TemporaryDirectory(prefix="optchat-tls-") as temporary:
        fixture = Path(temporary)
        (fixture / "empty").mkdir()

        def openssl(*arguments):
            subprocess.run(["openssl", *arguments], cwd=fixture, check=True,
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

        openssl("req", "-x509", "-newkey", "rsa:4096", "-nodes", "-sha256", "-days", "2",
                "-subj", "/CN=OptChat temporary qualification CA", "-keyout", "ca.key", "-out", "ca.crt")
        openssl("req", "-new", "-newkey", "rsa:4096", "-nodes", "-subj", "/CN=localhost",
                "-keyout", "server.key", "-out", "server.csr")
        for name, hostname, days in [("valid", "localhost", "2"), ("wrong-host", "wrong.invalid", "2"),
                                     ("expired", "localhost", "0")]:
            (fixture / "extensions").write_text(f"subjectAltName=DNS:{hostname}\nextendedKeyUsage=serverAuth\n")
            openssl("x509", "-req", "-in", "server.csr", "-CA", "ca.crt", "-CAkey", "ca.key",
                    "-CAcreateserial", "-sha256", "-days", days, "-extfile", "extensions", "-out", f"{name}.crt")
        openssl("req", "-x509", "-newkey", "rsa:4096", "-nodes", "-sha256", "-days", "2",
                "-subj", "/CN=Untrusted temporary CA", "-keyout", "untrusted.key", "-out", "untrusted.crt")
        for policy, include, extra in [
            ("strict", "/etc/crypto-policies/back-ends/opensslcnf.config", "MinProtocol=TLSv1.3\nVerifyMode=-Peer\n"),
            ("future", "/usr/share/crypto-policies/FUTURE/opensslcnf.txt", ""),
        ]:
            (fixture / f"{policy}.cnf").write_text(
                "openssl_conf=qualification_init\n[qualification_init]\nssl_conf=qualification_ssl\n"
                "[qualification_ssl]\nsystem_default=qualification_policy\n[qualification_policy]\n"
                f".include {include}\n[qualification_policy]\n{extra}")

        def check(name, cert="valid", version=ssl.TLSVersion.TLSv1_3, cipher=None,
                  trust="ca.crt", policy=None, allowed=True):
            context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
            context.minimum_version = version
            context.maximum_version = version
            if cipher:
                context.set_ciphers(cipher)
            context.load_cert_chain(fixture / f"{cert}.crt", fixture / "server.key")
            server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Fixture)
            server.negotiated = []
            server.socket = context.wrap_socket(server.socket, server_side=True)
            thread = threading.Thread(target=server.serve_forever, daemon=True)
            thread.start()
            command = ["podman", "run", "--rm", "--network=host", "--cap-drop=all",
                       "--security-opt=no-new-privileges", "--volume", f"{root}:/workspace:ro",
                       "--volume", f"{fixture}:/fixtures:ro", "--env", f"SSL_CERT_FILE=/fixtures/{trust}",
                       "--env", "SSL_CERT_DIR=/fixtures/empty"]
            if policy:
                command += ["--env", f"OPENSSL_CONF=/fixtures/{policy}.cnf"]
            command += [args.podman_image, f"/workspace/{relative_client}",
                        f"https://localhost:{server.server_port}"]
            try:
                completed = subprocess.run(command, text=True, capture_output=True, timeout=40)
                assert completed.returncode == 0, (name, completed.stderr)
                transports = json.loads(completed.stdout)
                assert transports == {"https": allowed, "wss": allowed}, (name, transports, completed.stderr)
                result = {"case": name, "expected_allowed": allowed, **transports,
                          "negotiated": server.negotiated}
                results.append(result)
                print(f"PASS {name}: HTTPS/WSS {'accepted' if allowed else 'rejected'}", flush=True)
            finally:
                server.shutdown()
                server.server_close()
                thread.join(timeout=2)

        check("DEFAULT TLS 1.3")
        check("DEFAULT allowed TLS 1.2", version=ssl.TLSVersion.TLSv1_2,
              cipher="ECDHE-RSA-AES128-GCM-SHA256")
        check("untrusted certificate", trust="untrusted.crt", allowed=False)
        check("wrong hostname", cert="wrong-host", allowed=False)
        check("expired certificate", cert="expired", allowed=False)
        check("legacy TLS 1.1", version=ssl.TLSVersion.TLSv1_1,
              cipher="ECDHE-RSA-AES128-SHA:@SECLEVEL=0", allowed=False)
        check("DEFAULT excluded TLS 1.2 cipher", version=ssl.TLSVersion.TLSv1_2,
              cipher="ECDHE-RSA-AES256-SHA384", allowed=False)
        check("stricter system minimum rejects TLS 1.2", version=ssl.TLSVersion.TLSv1_2,
              cipher="ECDHE-RSA-AES128-GCM-SHA256", policy="strict", allowed=False)
        check("stricter system minimum allows TLS 1.3", policy="strict")
        check("system configuration cannot disable certificate verification", policy="strict",
              trust="untrusted.crt", allowed=False)
        check("FUTURE TLS 1.3", policy="future")
        check("FUTURE rejects 128-bit TLS 1.2", version=ssl.TLSVersion.TLSv1_2,
              cipher="ECDHE-RSA-AES128-GCM-SHA256", policy="future", allowed=False)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps({"builder": args.podman_image, "client": str(client),
                                      "client_sha256": hashlib.sha256(client.read_bytes()).hexdigest(),
                                      "cases": results, "checks": 2 * len(results), "result": "passed"}, indent=2) + "\n")
    print(f"{2 * len(results)} HTTPS/WSS transport checks passed")


if __name__ == "__main__":
    main()
