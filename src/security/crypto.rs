//! Cryptographie de vérification, sans dépendance : SHA-256, SHA-384,
//! signatures RSA (RS256) et ECDSA P-384 (ES384).
//!
//! Ce module ne fait que VÉRIFIER des signatures à partir de données publiques ;
//! il ne manipule aucun secret, donc le temps d'exécution variable n'est pas un
//! risque ici. Il ne doit pas être réutilisé pour signer ou chiffrer.
//! Les constantes sont générées par programme et les fonctions sont comparées
//! à OpenSSL dans les tests.

// ---------------------------------------------------------------- SHA-2 ----

const K256: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];
const H256: [u32; 8] = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
const K512: [u64; 80] = [
    0x428a2f98d728ae22, 0x7137449123ef65cd, 0xb5c0fbcfec4d3b2f, 0xe9b5dba58189dbbc,
    0x3956c25bf348b538, 0x59f111f1b605d019, 0x923f82a4af194f9b, 0xab1c5ed5da6d8118,
    0xd807aa98a3030242, 0x12835b0145706fbe, 0x243185be4ee4b28c, 0x550c7dc3d5ffb4e2,
    0x72be5d74f27b896f, 0x80deb1fe3b1696b1, 0x9bdc06a725c71235, 0xc19bf174cf692694,
    0xe49b69c19ef14ad2, 0xefbe4786384f25e3, 0x0fc19dc68b8cd5b5, 0x240ca1cc77ac9c65,
    0x2de92c6f592b0275, 0x4a7484aa6ea6e483, 0x5cb0a9dcbd41fbd4, 0x76f988da831153b5,
    0x983e5152ee66dfab, 0xa831c66d2db43210, 0xb00327c898fb213f, 0xbf597fc7beef0ee4,
    0xc6e00bf33da88fc2, 0xd5a79147930aa725, 0x06ca6351e003826f, 0x142929670a0e6e70,
    0x27b70a8546d22ffc, 0x2e1b21385c26c926, 0x4d2c6dfc5ac42aed, 0x53380d139d95b3df,
    0x650a73548baf63de, 0x766a0abb3c77b2a8, 0x81c2c92e47edaee6, 0x92722c851482353b,
    0xa2bfe8a14cf10364, 0xa81a664bbc423001, 0xc24b8b70d0f89791, 0xc76c51a30654be30,
    0xd192e819d6ef5218, 0xd69906245565a910, 0xf40e35855771202a, 0x106aa07032bbd1b8,
    0x19a4c116b8d2d0c8, 0x1e376c085141ab53, 0x2748774cdf8eeb99, 0x34b0bcb5e19b48a8,
    0x391c0cb3c5c95a63, 0x4ed8aa4ae3418acb, 0x5b9cca4f7763e373, 0x682e6ff3d6b2b8a3,
    0x748f82ee5defb2fc, 0x78a5636f43172f60, 0x84c87814a1f0ab72, 0x8cc702081a6439ec,
    0x90befffa23631e28, 0xa4506cebde82bde9, 0xbef9a3f7b2c67915, 0xc67178f2e372532b,
    0xca273eceea26619c, 0xd186b8c721c0c207, 0xeada7dd6cde0eb1e, 0xf57d4f7fee6ed178,
    0x06f067aa72176fba, 0x0a637dc5a2c898a6, 0x113f9804bef90dae, 0x1b710b35131c471b,
    0x28db77f523047d84, 0x32caab7b40c72493, 0x3c9ebe0a15c9bebc, 0x431d67c49c100d4c,
    0x4cc5d4becb3e42b6, 0x597f299cfc657e2a, 0x5fcb6fab3ad6faec, 0x6c44198c4a475817,
];
const H384: [u64; 8] = [
    0xcbbb9d5dc1059ed8, 0x629a292a367cd507, 0x9159015a3070dd17, 0x152fecd8f70e5939,
    0x67332667ffc00b31, 0x8eb44a8768581511, 0xdb0c2e0d64f98fa7, 0x47b5481dbefa4fa4,
];

fn pad(data: &[u8], block: usize) -> Vec<u8> {
    let mut m = data.to_vec();
    m.push(0x80);
    let len_bytes = block / 8;
    while (m.len() + len_bytes) % block != 0 {
        m.push(0);
    }
    let bits = (data.len() as u128) * 8;
    m.extend_from_slice(&bits.to_be_bytes()[16 - len_bytes..]);
    m
}

pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut h = H256;
    for chunk in pad(data, 64).chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([chunk[4 * i], chunk[4 * i + 1], chunk[4 * i + 2], chunk[4 * i + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let mut v = h;
        for i in 0..64 {
            let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
            let ch = (v[4] & v[5]) ^ (!v[4] & v[6]);
            let t1 = v[7].wrapping_add(s1).wrapping_add(ch).wrapping_add(K256[i]).wrapping_add(w[i]);
            let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
            let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
            let t2 = s0.wrapping_add(maj);
            v = [t1.wrapping_add(t2), v[0], v[1], v[2], v[3].wrapping_add(t1), v[4], v[5], v[6]];
        }
        for i in 0..8 {
            h[i] = h[i].wrapping_add(v[i]);
        }
    }
    let mut out = [0u8; 32];
    for i in 0..8 {
        out[4 * i..4 * i + 4].copy_from_slice(&h[i].to_be_bytes());
    }
    out
}

pub fn sha384(data: &[u8]) -> [u8; 48] {
    let mut h = H384;
    for chunk in pad(data, 128).chunks(128) {
        let mut w = [0u64; 80];
        for i in 0..16 {
            let mut b = [0u8; 8];
            b.copy_from_slice(&chunk[8 * i..8 * i + 8]);
            w[i] = u64::from_be_bytes(b);
        }
        for i in 16..80 {
            let s0 = w[i - 15].rotate_right(1) ^ w[i - 15].rotate_right(8) ^ (w[i - 15] >> 7);
            let s1 = w[i - 2].rotate_right(19) ^ w[i - 2].rotate_right(61) ^ (w[i - 2] >> 6);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let mut v = h;
        for i in 0..80 {
            let s1 = v[4].rotate_right(14) ^ v[4].rotate_right(18) ^ v[4].rotate_right(41);
            let ch = (v[4] & v[5]) ^ (!v[4] & v[6]);
            let t1 = v[7].wrapping_add(s1).wrapping_add(ch).wrapping_add(K512[i]).wrapping_add(w[i]);
            let s0 = v[0].rotate_right(28) ^ v[0].rotate_right(34) ^ v[0].rotate_right(39);
            let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
            let t2 = s0.wrapping_add(maj);
            v = [t1.wrapping_add(t2), v[0], v[1], v[2], v[3].wrapping_add(t1), v[4], v[5], v[6]];
        }
        for i in 0..8 {
            h[i] = h[i].wrapping_add(v[i]);
        }
    }
    let mut out = [0u8; 48];
    for i in 0..6 {
        out[8 * i..8 * i + 8].copy_from_slice(&h[i].to_be_bytes());
    }
    out
}

// ------------------------------------------------------- Aléa du système ----

/// Remplit `buf` avec de l'aléa cryptographique fourni par le système.
/// Renvoie `false` si aucune source n'est disponible.
#[cfg(unix)]
pub fn os_random(buf: &mut [u8]) -> bool {
    use std::io::Read;
    std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(buf)).is_ok()
}

#[cfg(windows)]
pub fn os_random(buf: &mut [u8]) -> bool {
    #[link(name = "advapi32")]
    extern "system" {
        // RtlGenRandom
        fn SystemFunction036(buffer: *mut u8, length: u32) -> u8;
    }
    unsafe { SystemFunction036(buf.as_mut_ptr(), buf.len() as u32) != 0 }
}

#[cfg(not(any(unix, windows)))]
pub fn os_random(_: &mut [u8]) -> bool {
    false
}

// ------------------------------------------------- Grands entiers (mod m) ----

type Num = Vec<u64>;

/// Octets gros-boutistes -> mots de 64 bits petit-boutistes, sur `k` mots.
/// Renvoie `None` si la valeur ne tient pas.
fn from_be(bytes: &[u8], k: usize) -> Option<Num> {
    let mut out = vec![0u64; k];
    for (i, b) in bytes.iter().rev().enumerate() {
        if *b != 0 {
            if i / 8 >= k {
                return None;
            }
            out[i / 8] |= (*b as u64) << (8 * (i % 8));
        }
    }
    Some(out)
}

fn to_be(n: &[u64]) -> Vec<u8> {
    n.iter().rev().flat_map(|w| w.to_be_bytes().to_vec()).collect()
}

fn ge(a: &[u64], b: &[u64]) -> bool {
    for i in (0..a.len()).rev() {
        if a[i] != b[i] {
            return a[i] > b[i];
        }
    }
    true
}

fn is_zero(a: &[u64]) -> bool {
    a.iter().all(|w| *w == 0)
}

/// a -= b ; renvoie la retenue.
fn sub_assign(a: &mut [u64], b: &[u64]) -> bool {
    let mut borrow = false;
    for i in 0..a.len() {
        let (d, b1) = a[i].overflowing_sub(b[i]);
        let (d, b2) = d.overflowing_sub(borrow as u64);
        a[i] = d;
        borrow = b1 || b2;
    }
    borrow
}

/// a += b ; renvoie la retenue.
fn add_assign(a: &mut [u64], b: &[u64]) -> bool {
    let mut carry = false;
    for i in 0..a.len() {
        let (s, c1) = a[i].overflowing_add(b[i]);
        let (s, c2) = s.overflowing_add(carry as u64);
        a[i] = s;
        carry = c1 || c2;
    }
    carry
}

/// Arithmétique de Montgomery modulo un nombre impair.
struct Mont {
    m: Num,
    n0: u64,
    r2: Num,
    one: Num,
}

impl Mont {
    fn new(m: &[u64]) -> Mont {
        let k = m.len();
        // inverse de m[0] modulo 2^64 par itération de Newton
        let mut inv = m[0];
        for _ in 0..6 {
            inv = inv.wrapping_mul(2u64.wrapping_sub(m[0].wrapping_mul(inv)));
        }
        let mut mont = Mont { m: m.to_vec(), n0: inv.wrapping_neg(), r2: vec![0; k], one: vec![0; k] };
        // R^2 mod m par doublements successifs
        let mut t = vec![0u64; k];
        t[0] = 1;
        if ge(&t, m) {
            sub_assign(&mut t, m);
        }
        for _ in 0..(2 * 64 * k) {
            let copy = t.clone();
            let carry = add_assign(&mut t, &copy);
            if carry || ge(&t, m) {
                sub_assign(&mut t, m);
            }
        }
        mont.r2 = t;
        let mut raw_one = vec![0u64; k];
        raw_one[0] = 1;
        mont.one = mont.mul(&mont.r2, &raw_one);
        mont
    }

    fn mul(&self, a: &[u64], b: &[u64]) -> Num {
        let k = self.m.len();
        let mut t = vec![0u64; k + 2];
        for i in 0..k {
            let mut c = 0u128;
            for j in 0..k {
                let v = t[j] as u128 + (a[j] as u128) * (b[i] as u128) + c;
                t[j] = v as u64;
                c = v >> 64;
            }
            let v = t[k] as u128 + c;
            t[k] = v as u64;
            t[k + 1] = (v >> 64) as u64;

            let q = t[0].wrapping_mul(self.n0);
            let mut c = (t[0] as u128 + (q as u128) * (self.m[0] as u128)) >> 64;
            for j in 1..k {
                let v = t[j] as u128 + (q as u128) * (self.m[j] as u128) + c;
                t[j - 1] = v as u64;
                c = v >> 64;
            }
            let v = t[k] as u128 + c;
            t[k - 1] = v as u64;
            t[k] = t[k + 1].wrapping_add((v >> 64) as u64);
        }
        let overflow = t[k] != 0;
        t.truncate(k);
        if overflow || ge(&t, &self.m) {
            sub_assign(&mut t, &self.m);
        }
        t
    }

    fn enter(&self, a: &[u64]) -> Num {
        self.mul(a, &self.r2)
    }

    fn leave(&self, a: &[u64]) -> Num {
        let mut one = vec![0u64; self.m.len()];
        one[0] = 1;
        self.mul(a, &one)
    }

    fn add(&self, a: &[u64], b: &[u64]) -> Num {
        let mut t = a.to_vec();
        let carry = add_assign(&mut t, b);
        if carry || ge(&t, &self.m) {
            sub_assign(&mut t, &self.m);
        }
        t
    }

    fn sub(&self, a: &[u64], b: &[u64]) -> Num {
        let mut t = a.to_vec();
        if sub_assign(&mut t, b) {
            add_assign(&mut t, &self.m);
        }
        t
    }

    /// base^exp, base et résultat en forme de Montgomery.
    fn pow(&self, base: &[u64], exp: &[u64]) -> Num {
        let mut acc = self.one.clone();
        for i in (0..exp.len() * 64).rev() {
            acc = self.mul(&acc, &acc);
            if (exp[i / 64] >> (i % 64)) & 1 == 1 {
                acc = self.mul(&acc, base);
            }
        }
        acc
    }
}

// ------------------------------------------------------------------ RSA ----

/// Clé publique RSA (pour les jetons RS256).
pub struct RsaKey {
    mont: Mont,
    e: u64,
    bytes: usize,
}

const SHA256_DIGEST_INFO: [u8; 19] = [
    0x30, 0x31, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01, 0x05, 0x00, 0x04, 0x20,
];

impl RsaKey {
    /// `n` et `e` en octets gros-boutistes. Refuse les clés trop petites ou anormales.
    pub fn new(n: &[u8], e: &[u8]) -> Result<RsaKey, &'static str> {
        let n: Vec<u8> = n.iter().copied().skip_while(|b| *b == 0).collect();
        if n.len() < 256 || n.len() > 512 {
            return Err("clé RSA : taille non admise (2048 à 4096 bits)");
        }
        let k = (n.len() + 7) / 8;
        let m = from_be(&n, k).ok_or("clé RSA invalide")?;
        if m[0] & 1 == 0 {
            return Err("clé RSA : module pair");
        }
        let e = from_be(e, 1).ok_or("clé RSA : exposant trop grand")?[0];
        if e < 3 || e & 1 == 0 {
            return Err("clé RSA : exposant invalide");
        }
        Ok(RsaKey { mont: Mont::new(&m), e, bytes: n.len() })
    }

    /// Vérifie une signature RSASSA-PKCS1-v1_5 avec SHA-256.
    pub fn verify_sha256(&self, message: &[u8], signature: &[u8]) -> bool {
        if signature.len() != self.bytes {
            return false;
        }
        let k = self.mont.m.len();
        let s = match from_be(signature, k) {
            Some(s) if !ge(&s, &self.mont.m) => s,
            _ => return false,
        };
        let em = self.mont.leave(&self.mont.pow(&self.mont.enter(&s), &[self.e]));
        let em = to_be(&em);
        let em = &em[em.len() - self.bytes..];

        let mut expected = vec![0xffu8; self.bytes];
        expected[0] = 0x00;
        expected[1] = 0x01;
        let tail = self.bytes - 32 - SHA256_DIGEST_INFO.len();
        expected[tail - 1] = 0x00;
        expected[tail..tail + 19].copy_from_slice(&SHA256_DIGEST_INFO);
        expected[tail + 19..].copy_from_slice(&sha256(message));
        em == &expected[..]
    }
}

// ---------------------------------------------------------- ECDSA P-384 ----

const P: [u64; 6] = [0x00000000ffffffff, 0xffffffff00000000, 0xfffffffffffffffe, 0xffffffffffffffff, 0xffffffffffffffff, 0xffffffffffffffff];
const N: [u64; 6] = [0xecec196accc52973, 0x581a0db248b0a77a, 0xc7634d81f4372ddf, 0xffffffffffffffff, 0xffffffffffffffff, 0xffffffffffffffff];
const B: [u64; 6] = [0x2a85c8edd3ec2aef, 0xc656398d8a2ed19d, 0x0314088f5013875a, 0x181d9c6efe814112, 0x988e056be3f82d19, 0xb3312fa7e23ee7e4];
const GX: [u64; 6] = [0x3a545e3872760ab7, 0x5502f25dbf55296c, 0x59f741e082542a38, 0x6e1d3b628ba79b98, 0x8eb1c71ef320ad74, 0xaa87ca22be8b0537];
const GY: [u64; 6] = [0x7a431d7c90ea0e5f, 0x0a60b1ce1d7e819d, 0xe9da3113b5f0b8c0, 0xf8f41dbd289a147c, 0x5d9e98bf9292dc29, 0x3617de4a96262c6f];

/// En-tête DER d'une clé publique P-384 (SubjectPublicKeyInfo), suivi de X et Y.
const SPKI_PREFIX: [u8; 24] = [
    0x30, 0x76, 0x30, 0x10, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x05, 0x2b, 0x81, 0x04, 0x00,
    0x22, 0x03, 0x62, 0x00, 0x04,
];

/// Clé publique P-384 (coordonnées affines, vérifiées sur la courbe).
#[derive(Clone, PartialEq, Debug)]
pub struct EcKey {
    x: Num,
    y: Num,
}

/// Point en coordonnées jacobiennes, en forme de Montgomery ; z = 0 : point à l'infini.
#[derive(Clone)]
struct Point {
    x: Num,
    y: Num,
    z: Num,
}

struct Curve {
    f: Mont,
    b: Num,
}

impl Curve {
    fn new() -> Curve {
        let f = Mont::new(&P);
        let b = f.enter(&B);
        Curve { f, b }
    }

    fn on_curve(&self, x: &[u64], y: &[u64]) -> bool {
        let f = &self.f;
        let (xm, ym) = (f.enter(x), f.enter(y));
        let three_x = f.add(&f.add(&xm, &xm), &xm);
        let rhs = f.add(&f.sub(&f.mul(&f.mul(&xm, &xm), &xm), &three_x), &self.b);
        f.mul(&ym, &ym) == rhs
    }

    fn double(&self, p: &Point) -> Point {
        let f = &self.f;
        if is_zero(&p.z) || is_zero(&p.y) {
            return Point { x: f.one.clone(), y: f.one.clone(), z: vec![0; 6] };
        }
        let delta = f.mul(&p.z, &p.z);
        let gamma = f.mul(&p.y, &p.y);
        let beta = f.mul(&p.x, &gamma);
        let t = f.mul(&f.sub(&p.x, &delta), &f.add(&p.x, &delta));
        let alpha = f.add(&f.add(&t, &t), &t);
        let beta2 = f.add(&beta, &beta);
        let beta4 = f.add(&beta2, &beta2);
        let beta8 = f.add(&beta4, &beta4);
        let x3 = f.sub(&f.mul(&alpha, &alpha), &beta8);
        let yz = f.add(&p.y, &p.z);
        let z3 = f.sub(&f.sub(&f.mul(&yz, &yz), &gamma), &delta);
        let g2 = f.mul(&gamma, &gamma);
        let g4 = f.add(&g2, &g2);
        let g8 = f.add(&f.add(&g4, &g4), &f.add(&g4, &g4));
        let y3 = f.sub(&f.mul(&alpha, &f.sub(&beta4, &x3)), &g8);
        Point { x: x3, y: y3, z: z3 }
    }

    fn add(&self, p: &Point, q: &Point) -> Point {
        let f = &self.f;
        if is_zero(&p.z) {
            return q.clone();
        }
        if is_zero(&q.z) {
            return p.clone();
        }
        let z1z1 = f.mul(&p.z, &p.z);
        let z2z2 = f.mul(&q.z, &q.z);
        let u1 = f.mul(&p.x, &z2z2);
        let u2 = f.mul(&q.x, &z1z1);
        let s1 = f.mul(&p.y, &f.mul(&q.z, &z2z2));
        let s2 = f.mul(&q.y, &f.mul(&p.z, &z1z1));
        if u1 == u2 {
            if s1 == s2 {
                return self.double(p);
            }
            return Point { x: f.one.clone(), y: f.one.clone(), z: vec![0; 6] };
        }
        let h = f.sub(&u2, &u1);
        let r = f.sub(&s2, &s1);
        let h2 = f.mul(&h, &h);
        let h3 = f.mul(&h, &h2);
        let v = f.mul(&u1, &h2);
        let x3 = f.sub(&f.sub(&f.mul(&r, &r), &h3), &f.add(&v, &v));
        let y3 = f.sub(&f.mul(&r, &f.sub(&v, &x3)), &f.mul(&s1, &h3));
        let z3 = f.mul(&h, &f.mul(&p.z, &q.z));
        Point { x: x3, y: y3, z: z3 }
    }

    fn scalar_mul(&self, k: &[u64], p: &Point) -> Point {
        let mut acc = Point { x: self.f.one.clone(), y: self.f.one.clone(), z: vec![0; 6] };
        for i in (0..384).rev() {
            acc = self.double(&acc);
            if (k[i / 64] >> (i % 64)) & 1 == 1 {
                acc = self.add(&acc, p);
            }
        }
        acc
    }
}

impl EcKey {
    /// Lit une clé au format DER SubjectPublicKeyInfo (120 octets pour P-384).
    pub fn from_spki(der: &[u8]) -> Result<EcKey, &'static str> {
        if der.len() != 120 || der[..24] != SPKI_PREFIX {
            return Err("clé publique : format P-384 attendu");
        }
        let x = from_be(&der[24..72], 6).ok_or("clé publique invalide")?;
        let y = from_be(&der[72..120], 6).ok_or("clé publique invalide")?;
        if ge(&x, &P) || ge(&y, &P) || !Curve::new().on_curve(&x, &y) {
            return Err("clé publique : point hors de la courbe");
        }
        Ok(EcKey { x, y })
    }

    /// Vérifie une signature ES384 : `signature` = r || s (96 octets), SHA-384 du message.
    pub fn verify_sha384(&self, message: &[u8], signature: &[u8]) -> bool {
        if signature.len() != 96 {
            return false;
        }
        let (r, s) = match (from_be(&signature[..48], 6), from_be(&signature[48..], 6)) {
            (Some(r), Some(s)) => (r, s),
            _ => return false,
        };
        if is_zero(&r) || is_zero(&s) || ge(&r, &N) || ge(&s, &N) {
            return false;
        }
        let mut z = match from_be(&sha384(message), 6) {
            Some(z) => z,
            None => return false,
        };
        if ge(&z, &N) {
            sub_assign(&mut z, &N);
        }
        // w = s^-1 mod n (petit théorème de Fermat), u1 = z·w, u2 = r·w
        let fn_ = Mont::new(&N);
        let mut n_minus_2 = N.to_vec();
        sub_assign(&mut n_minus_2, &[2, 0, 0, 0, 0, 0]);
        let w = fn_.pow(&fn_.enter(&s), &n_minus_2);
        let u1 = fn_.leave(&fn_.mul(&fn_.enter(&z), &w));
        let u2 = fn_.leave(&fn_.mul(&fn_.enter(&r), &w));

        let c = Curve::new();
        let one = c.f.one.clone();
        let g = Point { x: c.f.enter(&GX), y: c.f.enter(&GY), z: one.clone() };
        let q = Point { x: c.f.enter(&self.x), y: c.f.enter(&self.y), z: one };
        let sum = c.add(&c.scalar_mul(&u1, &g), &c.scalar_mul(&u2, &q));
        if is_zero(&sum.z) {
            return false;
        }
        // x affine = X / Z^2
        let mut p_minus_2 = P.to_vec();
        sub_assign(&mut p_minus_2, &[2, 0, 0, 0, 0, 0]);
        let z2 = c.f.mul(&sum.z, &sum.z);
        let inv = c.f.pow(&z2, &p_minus_2);
        let mut x = c.f.leave(&c.f.mul(&sum.x, &inv));
        if ge(&x, &N) {
            sub_assign(&mut x, &N);
        }
        x == r
    }
}

/// Coordonnée x affine d'un point (non nul), réduite modulo p.
fn affine(c: &Curve, p: &Point) -> (Num, Num) {
    let mut p_minus_2 = P.to_vec();
    sub_assign(&mut p_minus_2, &[2, 0, 0, 0, 0, 0]);
    let zi = c.f.pow(&p.z, &p_minus_2);
    let zi2 = c.f.mul(&zi, &zi);
    let zi3 = c.f.mul(&zi2, &zi);
    (c.f.leave(&c.f.mul(&p.x, &zi2)), c.f.leave(&c.f.mul(&p.y, &zi3)))
}

fn fixed48(n: &[u64]) -> [u8; 48] {
    let v = to_be(n);
    let mut out = [0u8; 48];
    let take = v.len().min(48);
    out[48 - take..].copy_from_slice(&v[v.len() - take..]);
    out
}

fn random_scalar() -> Option<Num> {
    for _ in 0..32 {
        let mut bytes = [0u8; 48];
        if !os_random(&mut bytes) {
            return None;
        }
        let k = from_be(&bytes, 6)?;
        if !is_zero(&k) && !ge(&k, &N) {
            return Some(k);
        }
    }
    None
}

/// Clé privée P-384 jetable, pour l'outil de capture : elle sert le temps d'une
/// connexion à un serveur local. Les calculs ne sont pas à temps constant ; ne pas
/// l'employer pour une clé durable (l'identité du serveur utilise `p384`).
pub struct EcSecret {
    d: Num,
    public: EcKey,
}

impl EcSecret {
    pub fn generate() -> Option<EcSecret> {
        let d = random_scalar()?;
        let c = Curve::new();
        let g = Point { x: c.f.enter(&GX), y: c.f.enter(&GY), z: c.f.one.clone() };
        let (x, y) = affine(&c, &c.scalar_mul(&d, &g));
        Some(EcSecret { d, public: EcKey { x, y } })
    }

    pub fn public(&self) -> &EcKey {
        &self.public
    }

    /// Clé publique au format DER SubjectPublicKeyInfo (120 octets).
    pub fn spki(&self) -> Vec<u8> {
        let mut out = SPKI_PREFIX.to_vec();
        out.extend_from_slice(&fixed48(&self.public.x));
        out.extend_from_slice(&fixed48(&self.public.y));
        out
    }

    /// Secret partagé avec une autre clé (ECDH) : coordonnée x, 48 octets.
    pub fn shared(&self, peer: &EcKey) -> Option<[u8; 48]> {
        let c = Curve::new();
        let q = Point { x: c.f.enter(&peer.x), y: c.f.enter(&peer.y), z: c.f.one.clone() };
        let s = c.scalar_mul(&self.d, &q);
        if is_zero(&s.z) {
            return None;
        }
        Some(fixed48(&affine(&c, &s).0))
    }

    /// Signature ES384 : r || s (96 octets).
    pub fn sign_sha384(&self, message: &[u8]) -> Option<Vec<u8>> {
        let mut z = from_be(&sha384(message), 6)?;
        if ge(&z, &N) {
            sub_assign(&mut z, &N);
        }
        let c = Curve::new();
        let g = Point { x: c.f.enter(&GX), y: c.f.enter(&GY), z: c.f.one.clone() };
        let fn_ = Mont::new(&N);
        let mut n_minus_2 = N.to_vec();
        sub_assign(&mut n_minus_2, &[2, 0, 0, 0, 0, 0]);
        for _ in 0..16 {
            let k = random_scalar()?;
            let mut r = affine(&c, &c.scalar_mul(&k, &g)).0;
            if ge(&r, &N) {
                sub_assign(&mut r, &N);
            }
            if is_zero(&r) {
                continue;
            }
            let k_inv = fn_.pow(&fn_.enter(&k), &n_minus_2);
            let rd = fn_.mul(&fn_.enter(&r), &fn_.enter(&self.d));
            let s = fn_.leave(&fn_.mul(&k_inv, &fn_.add(&fn_.enter(&z), &rd)));
            if is_zero(&s) {
                continue;
            }
            let mut out = fixed48(&r).to_vec();
            out.extend_from_slice(&fixed48(&s));
            return Some(out);
        }
        None
    }
}

/// AES-256 (chiffrement d'un bloc), pour le mode compteur du protocole Bedrock.
pub struct Aes256 {
    round_keys: [[u8; 16]; 15],
    sbox: [u8; 256],
}

fn xtime(b: u8) -> u8 {
    (b << 1) ^ if b & 0x80 != 0 { 0x1b } else { 0 }
}

impl Aes256 {
    pub fn new(key: &[u8; 32]) -> Aes256 {
        // Table de substitution calculée (inverse dans GF(2^8) puis transformation affine)
        let mut sbox = [0u8; 256];
        let (mut p, mut q) = (1u8, 1u8);
        loop {
            p = p ^ xtime(p);
            q ^= q << 1;
            q ^= q << 2;
            q ^= q << 4;
            if q & 0x80 != 0 {
                q ^= 0x09;
            }
            sbox[p as usize] = q ^ q.rotate_left(1) ^ q.rotate_left(2) ^ q.rotate_left(3) ^ q.rotate_left(4) ^ 0x63;
            if p == 1 {
                break;
            }
        }
        sbox[0] = 0x63;
        let mut w = [[0u8; 4]; 60];
        for i in 0..8 {
            w[i].copy_from_slice(&key[4 * i..4 * i + 4]);
        }
        let mut rcon = 1u8;
        for i in 8..60 {
            let mut t = w[i - 1];
            if i % 8 == 0 {
                t = [sbox[t[1] as usize] ^ rcon, sbox[t[2] as usize], sbox[t[3] as usize], sbox[t[0] as usize]];
                rcon = xtime(rcon);
            } else if i % 8 == 4 {
                t = [sbox[t[0] as usize], sbox[t[1] as usize], sbox[t[2] as usize], sbox[t[3] as usize]];
            }
            for j in 0..4 {
                w[i][j] = w[i - 8][j] ^ t[j];
            }
        }
        let mut round_keys = [[0u8; 16]; 15];
        for r in 0..15 {
            for c in 0..4 {
                round_keys[r][4 * c..4 * c + 4].copy_from_slice(&w[4 * r + c]);
            }
        }
        Aes256 { round_keys, sbox }
    }

    pub fn encrypt_block(&self, block: &[u8; 16]) -> [u8; 16] {
        let mut s = *block;
        for i in 0..16 {
            s[i] ^= self.round_keys[0][i];
        }
        for round in 1..15 {
            let mut t = [0u8; 16];
            // substitution + décalage des lignes
            for c in 0..4 {
                for r in 0..4 {
                    t[4 * c + r] = self.sbox[s[4 * ((c + r) % 4) + r] as usize];
                }
            }
            if round != 14 {
                for c in 0..4 {
                    let col = [t[4 * c], t[4 * c + 1], t[4 * c + 2], t[4 * c + 3]];
                    let all = col[0] ^ col[1] ^ col[2] ^ col[3];
                    for r in 0..4 {
                        t[4 * c + r] = col[r] ^ all ^ xtime(col[r] ^ col[(r + 1) % 4]);
                    }
                }
            }
            for i in 0..16 {
                s[i] = t[i] ^ self.round_keys[round][i];
            }
        }
        s
    }
}

/// Flux AES-256 en mode compteur, tel que l'emploie Bedock : bloc initial = 12 premiers
/// octets de la clé puis le compteur 2 ; le flux se poursuit d'un paquet au suivant.
pub struct BedrockCipher {
    aes: Aes256,
    counter: [u8; 16],
    stream: [u8; 16],
    used: usize,
}

impl BedrockCipher {
    pub fn new(key: &[u8; 32]) -> BedrockCipher {
        let mut counter = [0u8; 16];
        counter[..12].copy_from_slice(&key[..12]);
        counter[15] = 2;
        BedrockCipher { aes: Aes256::new(key), counter, stream: [0; 16], used: 16 }
    }

    /// Chiffre ou déchiffre sur place (la même opération).
    pub fn apply(&mut self, data: &mut [u8]) {
        for b in data.iter_mut() {
            if self.used == 16 {
                self.stream = self.aes.encrypt_block(&self.counter);
                self.used = 0;
                for i in (12..16).rev() {
                    self.counter[i] = self.counter[i].wrapping_add(1);
                    if self.counter[i] != 0 {
                        break;
                    }
                }
            }
            *b ^= self.stream[self.used];
            self.used += 1;
        }
    }
}

#[cfg(test)]
mod capture_crypto_tests {
    use super::*;

    #[test]
    fn aes256_matches_the_standard_vector() {
        // FIPS-197, annexe C.3
        let key: Vec<u8> = (0u8..32).collect();
        let plain: Vec<u8> = (0u8..16).map(|i| i * 0x11).collect();
        let aes = Aes256::new(key.as_slice().try_into().unwrap());
        let out = aes.encrypt_block(plain.as_slice().try_into().unwrap());
        assert_eq!(
            out,
            [0x8e, 0xa2, 0xb7, 0xca, 0x51, 0x67, 0x45, 0xbf, 0xea, 0xfc, 0x49, 0x90, 0x4b, 0x49, 0x60, 0x89]
        );
    }

    #[test]
    fn counter_mode_matches_the_standard_vector() {
        // NIST SP 800-38A, F.5.5 (CTR-AES256), premier bloc, en partant du même bloc compteur
        let key = [
            0x60, 0x3d, 0xeb, 0x10, 0x15, 0xca, 0x71, 0xbe, 0x2b, 0x73, 0xae, 0xf0, 0x85, 0x7d, 0x77, 0x81, 0x1f, 0x35, 0x2c,
            0x07, 0x3b, 0x61, 0x08, 0xd7, 0x2d, 0x98, 0x10, 0xa3, 0x09, 0x14, 0xdf, 0xf4,
        ];
        let aes = Aes256::new(&key);
        let counter = [0xf0, 0xf1, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8, 0xf9, 0xfa, 0xfb, 0xfc, 0xfd, 0xfe, 0xff];
        let plain = [0x6b, 0xc1, 0xbe, 0xe2, 0x2e, 0x40, 0x9f, 0x96, 0xe9, 0x3d, 0x7e, 0x11, 0x73, 0x93, 0x17, 0x2a];
        let ks = aes.encrypt_block(&counter);
        let out: Vec<u8> = plain.iter().zip(ks.iter()).map(|(a, b)| a ^ b).collect();
        assert_eq!(out, [0x60, 0x1e, 0xc3, 0x13, 0x77, 0x57, 0x89, 0xa5, 0xb7, 0xa7, 0xf5, 0x04, 0xbb, 0xf3, 0xd2, 0x28]);
        // Le flux se poursuit d'un appel à l'autre
        let mut a = BedrockCipher::new(&key);
        let mut b = BedrockCipher::new(&key);
        let mut one = [7u8; 40];
        a.apply(&mut one);
        let mut two = [7u8; 40];
        b.apply(&mut two[..5]);
        b.apply(&mut two[5..]);
        assert_eq!(one, two);
        assert_ne!(one, [7u8; 40]);
    }

    #[test]
    fn disposable_key_signs_and_agrees() {
        let a = EcSecret::generate().unwrap();
        let b = EcSecret::generate().unwrap();
        assert_eq!(EcKey::from_spki(&a.spki()).unwrap(), *a.public());
        let sig = a.sign_sha384(b"bonjour").unwrap();
        assert!(a.public().verify_sha384(b"bonjour", &sig));
        assert!(!a.public().verify_sha384(b"bonsoir", &sig));
        assert!(!b.public().verify_sha384(b"bonjour", &sig));
        assert_eq!(a.shared(b.public()).unwrap(), b.shared(a.public()).unwrap());
    }
}
