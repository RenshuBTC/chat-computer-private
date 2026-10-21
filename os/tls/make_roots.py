#!/usr/bin/env python3
"""Makes src/roots.der and roots.txt: the certificate authorities Chat Computer trusts, which are Mozilla's (as
the ca-certificates package carries them: /usr/share/ca-certificates/mozilla, not the system's bundle, which can
hold local additions). roots.der is the certificates one after another; roots.txt names each, for review.

    python3 make_roots.py [folder]
"""
from __future__ import annotations

import base64
import hashlib
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent


def main() -> None:
    folder = Path(sys.argv[1] if len(sys.argv) > 1 else "/usr/share/ca-certificates/mozilla")
    version = subprocess.run(["dpkg-query", "-W", "-f", "${Version}", "ca-certificates"], capture_output=True, text=True).stdout.strip()
    certs = []
    for f in sorted(folder.glob("*.crt")):
        text = f.read_text()
        body = text.split("-----BEGIN CERTIFICATE-----")[1].split("-----END CERTIFICATE-----")[0]
        certs.append((f.stem, base64.b64decode("".join(body.split()))))
    (HERE / "src" / "roots.der").write_bytes(b"".join(der for _, der in certs))
    lines = [f"# Mozilla's certificate authorities, from the ca-certificates package {version} ({len(certs)}): name, SHA-256",
             "# Made by make_roots.py; src/roots.der has the certificates themselves."]
    lines += [f"{name}  {hashlib.sha256(der).hexdigest()}" for name, der in certs]
    (HERE / "roots.txt").write_text("\n".join(lines) + "\n")
    print(f"wrote {len(certs)} roots ({sum(len(d) for _, d in certs)} bytes)")


if __name__ == "__main__":
    main()
