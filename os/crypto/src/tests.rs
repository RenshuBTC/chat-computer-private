//! The standards' own test vectors (FIPS 180, RFC 4231, RFC 5869, FIPS 197, RFC 8439, RFC 7748), and many more
//! made by an established implementation (vectors.rs, from make_vectors.py).

use super::*;
use crate::chacha20poly1305::{block, ChaCha20Poly1305, Poly1305};
use crate::ecdsa::Curve;
use crate::gcm::AesGcm;

mod vectors {
    include!("vectors.rs");
}

fn hex(s: &str) -> Vec<u8> {
    let s: String = s.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

#[test]
fn sha2_matches_the_standard_and_hashlib() {
    assert_eq!(sha256(b"abc").to_vec(), hex("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"));
    for (m, d) in vectors::SHA256 {
        assert_eq!(Sha256::digest(&hex(m)).to_vec(), hex(d));
    }
    for (m, d) in vectors::SHA384 {
        assert_eq!(Sha384::digest(&hex(m)).to_vec(), hex(d));
    }
    for (m, d) in vectors::SHA512 {
        assert_eq!(Sha512::digest(&hex(m)).to_vec(), hex(d));
    }
    // Fed a byte at a time, the same.
    let (m, d) = vectors::SHA384[2];
    let mut h = Sha384::new();
    for b in hex(m) {
        h.update(&[b]);
    }
    assert_eq!(h.finish().to_vec(), hex(d));
}

#[test]
fn hmac_and_hkdf_match_the_rfcs_and_python() {
    let mac = hmac::<Sha256>(b"Jefe", b"what do ya want for nothing?");
    assert_eq!(mac.to_vec(), hex("5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"));
    for (k, m, d256, d384) in vectors::HMAC {
        assert_eq!(hmac::<Sha256>(&hex(k), &hex(m)).to_vec(), hex(d256));
        assert_eq!(hmac::<Sha384>(&hex(k), &hex(m)).to_vec(), hex(d384));
    }
    // RFC 5869, test case 1.
    let prk = hkdf_extract::<Sha256>(&hex("000102030405060708090a0b0c"), &[0x0b; 22]);
    assert_eq!(prk.to_vec(), hex("077709362c2e32df0ddc3f0dc47bba6390b6c73bb50f9c3122ec844ad7c2b3e5"));
    let mut okm = [0u8; 42];
    hkdf_expand::<Sha256>(&prk, &hex("f0f1f2f3f4f5f6f7f8f9"), &mut okm);
    assert_eq!(okm.to_vec(), hex("3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf34007208d5b887185865"));
    for (ikm, salt, info, want) in vectors::HKDF384 {
        let prk = hkdf_extract::<Sha384>(&hex(salt), &hex(ikm));
        let mut out = vec![0u8; 100];
        hkdf_expand::<Sha384>(&prk, &hex(info), &mut out);
        assert_eq!(out, hex(want));
    }
}

#[test]
fn aes_matches_fips_197() {
    let mut b = [0u8; 16];
    b.copy_from_slice(&hex("00112233445566778899aabbccddeeff"));
    let mut c = b;
    aes::Aes::new(&hex("000102030405060708090a0b0c0d0e0f")).encrypt_block(&mut c);
    assert_eq!(c.to_vec(), hex("69c4e0d86a7b0430d8cdb78070b4c55a"));
    let mut c = b;
    aes::Aes::new(&hex("000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f")).encrypt_block(&mut c);
    assert_eq!(c.to_vec(), hex("8ea2b7ca516745bfeafc49904b496089"));
}

#[test]
fn table_aes_matches_fips_197_and_the_constant_time_one() {
    // FIPS 197's examples (C.1, C.3).
    let mut b: [u8; 16] = core::array::from_fn(|i| (i as u8) * 0x11);
    let k128: Vec<u8> = (0..16).collect();
    let k256: Vec<u8> = (0..32).collect();
    let aes = aes_tables::Aes::new(&k128).unwrap();
    aes.encrypt(&mut b);
    assert_eq!(b, [0x69, 0xc4, 0xe0, 0xd8, 0x6a, 0x7b, 0x04, 0x30, 0xd8, 0xcd, 0xb7, 0x80, 0x70, 0xb4, 0xc5, 0x5a]);
    aes.decrypt(&mut b);
    assert_eq!(b, core::array::from_fn(|i| (i as u8) * 0x11));
    let aes = aes_tables::Aes::new(&k256).unwrap();
    aes.encrypt(&mut b);
    assert_eq!(b, [0x8e, 0xa2, 0xb7, 0xca, 0x51, 0x67, 0x45, 0xbf, 0xea, 0xfc, 0x49, 0x90, 0x4b, 0x49, 0x60, 0x89]);
    aes.decrypt(&mut b);
    assert_eq!(b, core::array::from_fn(|i| (i as u8) * 0x11));
    // The same as the crypto crate's (constant-time) AES, for many keys and blocks.
    let mut x = 1u64;
    let mut next = || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x as u8
    };
    for size in [16, 32] {
        for _ in 0..50 {
            let key: Vec<u8> = (0..size).map(|_| next()).collect();
            let mut a: [u8; 16] = core::array::from_fn(|_| next());
            let mut c = a;
            aes_tables::Aes::new(&key).unwrap().encrypt(&mut a);
            aes::Aes::new(&key).encrypt_block(&mut c);
            assert_eq!(a, c);
        }
    }
}

#[test]
fn aes_gcm_matches_openssl_and_refuses_anything_changed() {
    for (k, n, aad, pt, ct) in vectors::AES_GCM {
        let (k, aad, pt, ct) = (hex(k), hex(aad), hex(pt), hex(ct));
        let mut nonce = [0u8; 12];
        nonce.copy_from_slice(&hex(n));
        let g = AesGcm::new(&k);
        let mut data = pt.clone();
        let tag = g.seal(&nonce, &aad, &mut data);
        assert_eq!([data.clone(), tag.to_vec()].concat(), ct);
        assert!(g.open(&nonce, &aad, &mut data, &tag));
        assert_eq!(data, pt);
        // A bit changed anywhere: refused, and nothing decrypted.
        let mut bad = ct[..pt.len()].to_vec();
        if !bad.is_empty() {
            bad[0] ^= 1;
            let before = bad.clone();
            assert!(!g.open(&nonce, &aad, &mut bad, &tag));
            assert_eq!(bad, before);
        }
        let mut bad_tag = tag;
        bad_tag[15] ^= 0x80;
        let mut d = ct[..pt.len()].to_vec();
        assert!(!g.open(&nonce, &aad, &mut d, &bad_tag));
    }
}

#[test]
fn chacha20_and_poly1305_match_rfc_8439_and_openssl() {
    let mut key = [0u8; 32];
    key.copy_from_slice(&hex("000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f"));
    let mut nonce = [0u8; 12];
    nonce.copy_from_slice(&hex("000000090000004a00000000"));
    assert_eq!(
        block(&key, 1, &nonce).to_vec(),
        hex("10f1e7e4d13b5915500fdd1fa32071c4c7d1f4c733c068030422aa9ac3d46c4e\
             d2826446079faa0914c2d705d98b02a2b5129cd1de164eb9cbd083e8a2503c4e")
    );
    let mut pk = [0u8; 32];
    pk.copy_from_slice(&hex("85d6be7857556d337f4452fe42d506a80103808afb0db2fd4abff6af4149f51b"));
    let mut p = Poly1305::new(&pk);
    p.update(b"Cryptographic Forum Research Group");
    assert_eq!(p.finish().to_vec(), hex("a8061dc1305136c6c22b8baf0c0127a9"));
    for (k, n, aad, pt, ct) in vectors::CHACHA {
        let (k, aad, pt, ct) = (hex(k), hex(aad), hex(pt), hex(ct));
        let mut nonce = [0u8; 12];
        nonce.copy_from_slice(&hex(n));
        let c = ChaCha20Poly1305::new(&k);
        let mut data = pt.clone();
        let tag = c.seal(&nonce, &aad, &mut data);
        assert_eq!([data.clone(), tag.to_vec()].concat(), ct);
        assert!(c.open(&nonce, &aad, &mut data, &tag));
        assert_eq!(data, pt);
        let mut t = tag;
        t[0] ^= 1;
        assert!(!c.open(&nonce, &aad, &mut data.clone(), &t));
    }
}

fn arr32(s: &str) -> [u8; 32] {
    let mut a = [0u8; 32];
    a.copy_from_slice(&hex(s));
    a
}

#[test]
fn x25519_matches_rfc_7748_and_openssl() {
    assert_eq!(
        x25519::x25519(
            &arr32("a546e36bf0527c9d3b16154b82465edd62144c0ac1fc5a18506a2244ba449ac4"),
            &arr32("e6db6867583030db3594c1a424b15f7c726624ec26b3353b10a903a6d0ab1c4c")
        )
        .to_vec(),
        hex("c3da55379de9c6908e94ea4df28d084f32eccf03491c71f754b4075577a28552")
    );
    let alice = arr32("77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a");
    let bob = arr32("5dab087e624a8a4b79e17f8b83800ee66f3bb1292618b6fd1c2f8b27ff88e0eb");
    assert_eq!(x25519::public(&alice).to_vec(), hex("8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a"));
    assert_eq!(x25519::public(&bob).to_vec(), hex("de9edb7d7b7dc1b4d35b61c2ece435373f8343c85b78674dadfc7e146f882b4f"));
    let shared = hex("4a5d9d5ba4ce2de1728e3bf480350f25e07e21c947d19e3376f09b3c1e161742");
    assert_eq!(x25519::shared(&alice, &x25519::public(&bob)).unwrap().to_vec(), shared);
    assert_eq!(x25519::shared(&bob, &x25519::public(&alice)).unwrap().to_vec(), shared);
    for (private, public, theirs, secret) in vectors::X25519 {
        assert_eq!(x25519::public(&arr32(private)).to_vec(), hex(public));
        assert_eq!(x25519::shared(&arr32(private), &arr32(theirs)).unwrap().to_vec(), hex(secret));
    }
    // A key that makes the secret zero (a point of small order): refused.
    assert_eq!(x25519::shared(&alice, &[0u8; 32]), None);
}

#[test]
fn ecdsa_signatures_from_openssl_check_and_changed_ones_dont() {
    for (curve, hash, point, msg, r, s) in vectors::ECDSA {
        let curve = if *curve == "P256" { Curve::P256 } else { Curve::P384 };
        let digest = if *hash == "SHA256" { sha256(&hex(msg)).to_vec() } else { Sha384::digest(&hex(msg)).to_vec() };
        let (point, r, s) = (hex(point), hex(r), hex(s));
        assert!(ecdsa::verify(curve, &point, &digest, &r, &s), "{curve:?} {hash}");
        let mut wrong = digest.clone();
        wrong[3] ^= 1;
        assert!(!ecdsa::verify(curve, &point, &wrong, &r, &s));
        let mut r2 = r.clone();
        r2[5] ^= 0x10;
        assert!(!ecdsa::verify(curve, &point, &digest, &r2, &s));
        let mut p2 = point.clone();
        p2[10] ^= 1; // no longer on the curve
        assert!(!ecdsa::verify(curve, &p2, &digest, &r, &s));
        assert!(!ecdsa::verify(curve, &point, &digest, &vec![0; r.len()], &s));
    }
}

#[test]
fn rsa_signatures_from_openssl_check_and_changed_ones_dont() {
    for (hash, n, e, msg, pkcs1, pss) in vectors::RSA {
        let d = match *hash {
            "SHA256" => rsa::Digest::Sha256,
            "SHA384" => rsa::Digest::Sha384,
            _ => rsa::Digest::Sha512,
        };
        let (n, e, msg, pkcs1, pss) = (hex(n), hex(e), hex(msg), hex(pkcs1), hex(pss));
        assert!(rsa::verify_pkcs1(&n, &e, d, &msg, &pkcs1), "{hash} {}", n.len());
        assert!(rsa::verify_pss(&n, &e, d, &msg, &pss), "{hash} {}", n.len());
        // Each kind isn't taken for the other; a changed message or signature fails.
        assert!(!rsa::verify_pkcs1(&n, &e, d, &msg, &pss));
        assert!(!rsa::verify_pss(&n, &e, d, &msg, &pkcs1));
        let mut m2 = msg.clone();
        m2[0] ^= 1;
        assert!(!rsa::verify_pkcs1(&n, &e, d, &m2, &pkcs1) && !rsa::verify_pss(&n, &e, d, &m2, &pss));
        let mut s2 = pkcs1.clone();
        s2[100] ^= 4;
        assert!(!rsa::verify_pkcs1(&n, &e, d, &msg, &s2));
    }
}

#[test]
fn the_generator_gives_different_bytes_each_time_and_forgets_its_key() {
    let mut d = drbg::Drbg::new(b"seed");
    let (a, b): ([u8; 48], [u8; 48]) = (d.bytes(), d.bytes());
    assert_ne!(a, b);
    // The same seed, the same bytes (it's a function of its seed: the kernel's seed is what's hard to guess).
    let mut e = drbg::Drbg::new(b"seed");
    assert_eq!(e.bytes::<48>(), a);
    // Reseeded: different from then on.
    e.reseed(b"more");
    assert_ne!(e.bytes::<48>(), b);
    // Long requests are whole and not repetitive.
    let mut long = vec![0u8; 1000];
    d.fill(&mut long);
    assert!(long.chunks(64).collect::<std::collections::HashSet<_>>().len() == 16);
}
