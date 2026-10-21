#!/usr/bin/env python3
"""Makes src/vectors.rs: test vectors from Python's own hashlib and from the cryptography package (OpenSSL), so
Chat Computer's own cryptography is checked against an established implementation as well as the standards'
published vectors (those are in src/tests.rs). Keys are made fresh each time this runs; the file is kept, so the
tests don't change unless it's run again.

    pip install cryptography; python3 make_vectors.py
"""
from __future__ import annotations

import hashlib
import hmac
import os
from pathlib import Path

from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import ec, padding, rsa, x25519
from cryptography.hazmat.primitives.asymmetric.utils import decode_dss_signature
from cryptography.hazmat.primitives.ciphers.aead import AESGCM, ChaCha20Poly1305
from cryptography.hazmat.primitives.kdf.hkdf import HKDF

OUT = Path(__file__).resolve().parent / "src" / "vectors.rs"


def h(b: bytes) -> str:
    return '"' + b.hex() + '"'


def main() -> None:
    lines = ["// Made by make_vectors.py from Python's hashlib and the cryptography package (OpenSSL). Don't edit.", ""]
    msgs = [b"", b"abc", bytes(range(256)) * 4 + b"tail", b"a" * 111, b"b" * 112, b"c" * 127, b"d" * 128]
    for name, fn in [("SHA256", hashlib.sha256), ("SHA384", hashlib.sha384), ("SHA512", hashlib.sha512)]:
        lines.append(f"pub const {name}: &[(&str, &str)] = &[")
        for m in msgs:
            lines.append(f"    ({h(m)}, {h(fn(m).digest())}),")
        lines.append("];")
    lines.append("pub const HMAC: &[(&str, &str, &str, &str)] = &[")
    for key in [b"k", os.urandom(32), os.urandom(200)]:
        for m in [b"", b"hello", os.urandom(300)]:
            lines.append(f"    ({h(key)}, {h(m)}, {h(hmac.new(key, m, hashlib.sha256).digest())}, "
                         f"{h(hmac.new(key, m, hashlib.sha384).digest())}),")
    lines.append("];")
    lines.append("pub const HKDF384: &[(&str, &str, &str, &str)] = &[")
    for _ in range(3):
        ikm, salt, info = os.urandom(48), os.urandom(48), os.urandom(20)
        okm = HKDF(algorithm=hashes.SHA384(), length=100, salt=salt, info=info).derive(ikm)
        lines.append(f"    ({h(ikm)}, {h(salt)}, {h(info)}, {h(okm)}),")
    lines.append("];")
    lines.append("pub const AES_GCM: &[(&str, &str, &str, &str, &str)] = &[")
    for klen in (16, 32):
        for n in (0, 1, 15, 16, 17, 64, 100, 1000):
            key, nonce, aad, pt = os.urandom(klen), os.urandom(12), os.urandom(n % 23), os.urandom(n)
            ct = AESGCM(key).encrypt(nonce, pt, aad)
            lines.append(f"    ({h(key)}, {h(nonce)}, {h(aad)}, {h(pt)}, {h(ct)}),")
    lines.append("];")
    lines.append("pub const CHACHA: &[(&str, &str, &str, &str, &str)] = &[")
    for n in (0, 1, 15, 16, 17, 63, 64, 65, 300):
        key, nonce, aad, pt = os.urandom(32), os.urandom(12), os.urandom(n % 19), os.urandom(n)
        ct = ChaCha20Poly1305(key).encrypt(nonce, pt, aad)
        lines.append(f"    ({h(key)}, {h(nonce)}, {h(aad)}, {h(pt)}, {h(ct)}),")
    lines.append("];")
    lines.append("pub const X25519: &[(&str, &str, &str, &str)] = &[")
    for _ in range(4):
        a, b = x25519.X25519PrivateKey.generate(), x25519.X25519PrivateKey.generate()
        raw = lambda k: k.private_bytes(serialization.Encoding.Raw, serialization.PrivateFormat.Raw, serialization.NoEncryption())
        pub = lambda k: k.public_key().public_bytes(serialization.Encoding.Raw, serialization.PublicFormat.Raw)
        lines.append(f"    ({h(raw(a))}, {h(pub(a))}, {h(pub(b))}, {h(a.exchange(b.public_key()))}),")
    lines.append("];")
    lines.append("pub const ECDSA: &[(&str, &str, &str, &str, &str, &str)] = &[")
    for curve, name, hashname, hashfn in [(ec.SECP256R1(), "P256", "SHA256", hashes.SHA256()),
                                          (ec.SECP384R1(), "P384", "SHA384", hashes.SHA384()),
                                          (ec.SECP256R1(), "P256", "SHA384", hashes.SHA384()),
                                          (ec.SECP384R1(), "P384", "SHA256", hashes.SHA256())]:
        for _ in range(2):
            key = ec.generate_private_key(curve)
            msg = os.urandom(50)
            r, s = decode_dss_signature(key.sign(msg, ec.ECDSA(hashfn)))
            size = curve.key_size // 8
            point = key.public_key().public_bytes(serialization.Encoding.X962, serialization.PublicFormat.UncompressedPoint)
            lines.append(f'    ("{name}", "{hashname}", {h(point)}, {h(msg)}, {h(r.to_bytes(size, "big"))}, '
                         f'{h(s.to_bytes(size, "big"))}),')
    lines.append("];")
    lines.append("pub const RSA: &[(&str, &str, &str, &str, &str, &str)] = &[")
    for bits, hashname, hashfn in [(2048, "SHA256", hashes.SHA256()), (3072, "SHA384", hashes.SHA384()),
                                   (4096, "SHA512", hashes.SHA512()), (2049, "SHA256", hashes.SHA256())]:
        key = rsa.generate_private_key(public_exponent=65537, key_size=bits)
        nums = key.public_key().public_numbers()
        n = nums.n.to_bytes((nums.n.bit_length() + 7) // 8, "big")
        e = nums.e.to_bytes(3, "big")
        msg = os.urandom(64)
        pkcs1 = key.sign(msg, padding.PKCS1v15(), hashfn)
        pss = key.sign(msg, padding.PSS(mgf=padding.MGF1(hashfn), salt_length=hashfn.digest_size), hashfn)
        lines.append(f'    ("{hashname}", {h(n)}, {h(e)}, {h(msg)}, {h(pkcs1)}, {h(pss)}),')
    lines.append("];")
    OUT.write_text("\n".join(lines) + "\n")
    print("wrote", OUT)


if __name__ == "__main__":
    main()
