#!/usr/bin/env python3
"""Génère des jetons de test (tests/auth/fixtures.txt) et le fichier de clés
associé (tests/auth/keys.json), signés par des clés créées pour l'occasion.

Ces jetons imitent ceux du service d'autorisation de Minecraft ; ils ne sont
acceptés que par un serveur configuré avec tests/auth/keys.json.
Usage : python3 tests/auth/generate_fixtures.py
"""
import base64, json, os
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import ec, rsa, padding, utils

HERE = os.path.dirname(os.path.abspath(__file__))
ISSUER = "https://authorization.franchise.minecraft-services.net"
AUDIENCE = "api://auth-minecraft-services/multiplayer"
FAR = 4102444800  # 1er janvier 2100

def b64u(data):
    if isinstance(data, (dict, list)):
        data = json.dumps(data, separators=(",", ":")).encode()
    return base64.urlsafe_b64encode(data).rstrip(b"=").decode()

def rs256(key, header, payload):
    signed = f"{b64u(header)}.{b64u(payload)}"
    return f"{signed}.{b64u(key.sign(signed.encode(), padding.PKCS1v15(), hashes.SHA256()))}"

def es384(key, header, payload):
    signed = f"{b64u(header)}.{b64u(payload)}"
    r, s = utils.decode_dss_signature(key.sign(signed.encode(), ec.ECDSA(hashes.SHA384())))
    return f"{signed}.{b64u(r.to_bytes(48, 'big') + s.to_bytes(48, 'big'))}"

def spki(key):
    der = key.public_key().public_bytes(serialization.Encoding.DER, serialization.PublicFormat.SubjectPublicKeyInfo)
    return base64.b64encode(der).decode()

mojang = rsa.generate_private_key(public_exponent=65537, key_size=2048)
other_rsa = rsa.generate_private_key(public_exponent=65537, key_size=2048)
player = ec.generate_private_key(ec.SECP384R1())
thief = ec.generate_private_key(ec.SECP384R1())

pub = mojang.public_key().public_numbers()
keys = {"issuer": ISSUER, "keys": [{
    "kty": "RSA", "kid": "test-key-1", "use": "sig", "alg": "RS256",
    "n": b64u(pub.n.to_bytes(256, "big")), "e": b64u(pub.e.to_bytes(3, "big")),
}]}
json.dump(keys, open(os.path.join(HERE, "keys.json"), "w"), indent=1)

head = {"alg": "RS256", "typ": "JWT", "kid": "test-key-1"}
claims = {"iss": ISSUER, "aud": AUDIENCE, "exp": FAR, "nbf": 1700000000, "iat": 1700000000,
          "xid": "2535412345678901", "xname": "Alex", "cpk": spki(player)}
client_data = {"GameVersion": "1.26.65", "DeviceOS": 7, "ThirdPartyName": "Imposteur"}
client_head = {"alg": "ES384", "x5u": spki(player)}

valid = rs256(mojang, head, claims)
h, p, s = valid.split(".")
tampered = f"{h}.{b64u({**claims, 'xname': 'Admin'})}.{s}"

# Ancienne chaîne auto-signée annonçant un autre pseudo : elle ne doit pas compter
self_signed = es384(thief, {"alg": "ES384", "x5u": spki(thief)},
                    {"extraData": {"displayName": "Imposteur", "XUID": "1"}, "identityPublicKey": spki(thief), "exp": FAR})

fixtures = {
    "token_valid": valid,
    "token_expired": rs256(mojang, head, {**claims, "exp": 1700000100}),
    "token_not_yet_valid": rs256(mojang, head, {**claims, "nbf": FAR - 10}),
    "token_wrong_audience": rs256(mojang, head, {**claims, "aud": "api://autre-service"}),
    "token_wrong_issuer": rs256(mojang, head, {**claims, "iss": "https://evil.example"}),
    "token_other_signer": rs256(other_rsa, head, claims),
    "token_unknown_kid": rs256(mojang, {**head, "kid": "inconnue"}, claims),
    "token_tampered": tampered,
    "token_alg_none": f"{b64u({'alg': 'none', 'kid': 'test-key-1'})}.{b64u(claims)}.",
    "token_alg_hs256": f"{b64u({'alg': 'HS256', 'kid': 'test-key-1'})}.{b64u(claims)}.{b64u(os.urandom(32))}",
    "token_no_xuid": rs256(mojang, head, {k: v for k, v in claims.items() if k != "xid"}),
    "token_bad_cpk": rs256(mojang, head, {**claims, "cpk": base64.b64encode(b"\x00" * 120).decode()}),
    "client_valid": es384(player, client_head, client_data),
    "client_other_key": es384(thief, client_head, client_data),
    "client_alg_none": f"{b64u({'alg': 'none'})}.{b64u(client_data)}.",
    "self_signed_chain": self_signed,
}
with open(os.path.join(HERE, "fixtures.txt"), "w") as f:
    for name, value in fixtures.items():
        f.write(f"{name} {value}\n")
print(len(fixtures), "jetons de test écrits")
