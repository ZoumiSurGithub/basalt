#!/usr/bin/env python3
"""Génère tests/auth/crypto_vectors.txt à l'aide d'OpenSSL (module `cryptography`).

Les signatures et condensés de référence servent à vérifier src/crypto.rs.
Usage : python3 tests/auth/generate_vectors.py [nombre]
"""
import hashlib, os, sys
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import ec, rsa, padding, utils

count = int(sys.argv[1]) if len(sys.argv) > 1 else 24
N = 0xffffffffffffffffffffffffffffffffffffffffffffffffc7634d81f4372ddf581a0db248b0a77aecec196accc52973
out = []

for size in [0, 1, 3, 55, 56, 63, 64, 65, 111, 112, 119, 127, 128, 129, 200, 1000]:
    m = os.urandom(size)
    out.append(f"sha256 {m.hex() or '-'} {hashlib.sha256(m).hexdigest()}")
    out.append(f"sha384 {m.hex() or '-'} {hashlib.sha384(m).hexdigest()}")

def spki(key):
    return key.public_key().public_bytes(serialization.Encoding.DER, serialization.PublicFormat.SubjectPublicKeyInfo).hex()

for i in range(count):
    key = ec.generate_private_key(ec.SECP384R1())
    msg = os.urandom(1 + i * 7)
    r, s = utils.decode_dss_signature(key.sign(msg, ec.ECDSA(hashes.SHA384())))
    sig = r.to_bytes(48, "big") + s.to_bytes(48, "big")
    out.append(f"ec {spki(key)} {msg.hex()} {sig.hex()} 1")
    kind = i % 6
    if kind == 0:   # message modifié
        bad = bytes([msg[0] ^ 1]) + msg[1:]
        out.append(f"ec {spki(key)} {bad.hex()} {sig.hex()} 0")
    elif kind == 1: # signature modifiée
        bad = bytearray(sig); bad[70] ^= 0x10
        out.append(f"ec {spki(key)} {msg.hex()} {bytes(bad).hex()} 0")
    elif kind == 2: # autre clé
        out.append(f"ec {spki(ec.generate_private_key(ec.SECP384R1()))} {msg.hex()} {sig.hex()} 0")
    elif kind == 3: # r = 0
        out.append(f"ec {spki(key)} {msg.hex()} {(bytes(48) + sig[48:]).hex()} 0")
    elif kind == 4: # s >= n
        out.append(f"ec {spki(key)} {msg.hex()} {(sig[:48] + N.to_bytes(48, 'big')).hex()} 0")
    else:           # s remplacé par n - s : toujours valide en ECDSA
        out.append(f"ec {spki(key)} {msg.hex()} {(sig[:48] + (N - s).to_bytes(48, 'big')).hex()} 1")

for i, bits in enumerate([2048, 2048, 3072, 4096] * max(1, count // 12)):
    key = rsa.generate_private_key(public_exponent=65537 if i % 2 == 0 else 3, key_size=bits)
    pub = key.public_key().public_numbers()
    n = pub.n.to_bytes(bits // 8, "big").hex()
    e = pub.e.to_bytes(4, "big").hex()
    msg = os.urandom(10 + i * 31)
    sig = key.sign(msg, padding.PKCS1v15(), hashes.SHA256())
    out.append(f"rsa {n} {e} {msg.hex()} {sig.hex()} 1")
    bad = bytearray(sig); bad[-1] ^= 1
    out.append(f"rsa {n} {e} {msg.hex()} {bytes(bad).hex()} 0")
    out.append(f"rsa {n} {e} {(msg + b'x').hex()} {sig.hex()} 0")
    # signature PSS ou SHA-384 : mauvais schéma, doit être refusée
    other = key.sign(msg, padding.PKCS1v15(), hashes.SHA384())
    out.append(f"rsa {n} {e} {msg.hex()} {other.hex()} 0")

path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "crypto_vectors.txt")
open(path, "w").write("\n".join(out) + "\n")
print(len(out), "vecteurs ->", path)
