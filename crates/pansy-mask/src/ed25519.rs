//! Just enough of Ed25519 to tell a Solana secret key from a signature of
//! the same length: the public key a 32-byte seed derives (RFC 8032, 5.1.5).
//! Nothing here signs, and nothing needs to run in constant time: the
//! check reads text a request already holds.

use sha2::{Digest, Sha512};

const MASK: u64 = (1 << 51) - 1;

/// An element of the field of 2^255 - 19, in five limbs of 51 bits.
#[derive(Clone, Copy)]
struct Fe([u64; 5]);

impl Fe {
    const ZERO: Fe = Fe([0; 5]);
    const ONE: Fe = Fe([1, 0, 0, 0, 0]);

    fn small(n: u64) -> Fe {
        Fe([n, 0, 0, 0, 0])
    }

    fn from_bytes(b: &[u8; 32]) -> Fe {
        let load = |i: usize| u64::from_le_bytes(b[i..i + 8].try_into().unwrap());
        Fe([
            load(0) & MASK,
            (load(6) >> 3) & MASK,
            (load(12) >> 6) & MASK,
            (load(19) >> 1) & MASK,
            (load(24) >> 12) & MASK,
        ])
    }

    fn carry(mut l: [u64; 5]) -> Fe {
        for _ in 0..2 {
            for i in 0..4 {
                l[i + 1] += l[i] >> 51;
                l[i] &= MASK;
            }
            l[0] += (l[4] >> 51) * 19;
            l[4] &= MASK;
        }
        Fe(l)
    }

    fn add(self, o: Fe) -> Fe {
        let mut l = self.0;
        for (x, y) in l.iter_mut().zip(o.0) {
            *x += y;
        }
        Fe::carry(l)
    }

    fn sub(self, o: Fe) -> Fe {
        // 4p added first, so no limb goes below zero
        let four_p = [
            (1 << 53) - 76,
            (1 << 53) - 4,
            (1 << 53) - 4,
            (1 << 53) - 4,
            (1 << 53) - 4,
        ];
        let mut l = self.0;
        for i in 0..5 {
            l[i] = l[i] + four_p[i] - o.0[i];
        }
        Fe::carry(l)
    }

    fn mul(self, o: Fe) -> Fe {
        let a = self.0.map(|x| x as u128);
        let b = o.0.map(|x| x as u128);
        let b19 = b.map(|x| x * 19);
        let mut r = [
            a[0] * b[0] + a[1] * b19[4] + a[2] * b19[3] + a[3] * b19[2] + a[4] * b19[1],
            a[0] * b[1] + a[1] * b[0] + a[2] * b19[4] + a[3] * b19[3] + a[4] * b19[2],
            a[0] * b[2] + a[1] * b[1] + a[2] * b[0] + a[3] * b19[4] + a[4] * b19[3],
            a[0] * b[3] + a[1] * b[2] + a[2] * b[1] + a[3] * b[0] + a[4] * b19[4],
            a[0] * b[4] + a[1] * b[3] + a[2] * b[2] + a[3] * b[1] + a[4] * b[0],
        ];
        for i in 0..4 {
            r[i + 1] += r[i] >> 51;
            r[i] &= MASK as u128;
        }
        r[0] += (r[4] >> 51) * 19;
        r[4] &= MASK as u128;
        r[1] += r[0] >> 51;
        r[0] &= MASK as u128;
        Fe::carry(r.map(|x| x as u64))
    }

    /// self^(p-2), the inverse.
    fn invert(self) -> Fe {
        // p - 2 = 2^255 - 21, little-endian
        let mut e = [0xffu8; 32];
        e[0] = 0xeb;
        e[31] = 0x7f;
        let mut r = Fe::ONE;
        for bit in (0..255).rev() {
            r = r.mul(r);
            if e[bit / 8] >> (bit % 8) & 1 == 1 {
                r = r.mul(self);
            }
        }
        r
    }

    fn to_bytes(self) -> [u8; 32] {
        let mut h = Fe::carry(self.0).0;
        // h is now below 2^255 + a little: take p off once if it is p or more
        let mut q = (h[0] + 19) >> 51;
        for x in &h[1..] {
            q = (x + q) >> 51;
        }
        h[0] += 19 * q;
        for i in 0..4 {
            h[i + 1] += h[i] >> 51;
            h[i] &= MASK;
        }
        h[4] &= MASK;
        let mut out = [0u8; 32];
        let (mut acc, mut bits, mut at) = (0u128, 0, 0);
        for limb in h {
            acc |= (limb as u128) << bits;
            bits += 51;
            while bits >= 8 {
                out[at] = acc as u8;
                acc >>= 8;
                bits -= 8;
                at += 1;
            }
        }
        out[at] = acc as u8;
        out
    }
}

/// A point in extended coordinates: x = X/Z, y = Y/Z, xy = T/Z.
#[derive(Clone, Copy)]
struct Point {
    x: Fe,
    y: Fe,
    z: Fe,
    t: Fe,
}

impl Point {
    /// The sum of two points, for a twisted Edwards curve with a = -1
    /// (add-2008-hwcd-3), which doubles a point too.
    fn add(self, o: Point, d2: Fe) -> Point {
        let a = self.y.sub(self.x).mul(o.y.sub(o.x));
        let b = self.y.add(self.x).mul(o.y.add(o.x));
        let c = self.t.mul(d2).mul(o.t);
        let d = self.z.add(self.z).mul(o.z);
        let (e, f, g, h) = (b.sub(a), d.sub(c), d.add(c), b.add(a));
        Point {
            x: e.mul(f),
            y: g.mul(h),
            z: f.mul(g),
            t: e.mul(h),
        }
    }
}

/// The public key the 32-byte `seed` derives.
pub(crate) fn public_key(seed: &[u8]) -> [u8; 32] {
    let h = Sha512::digest(seed);
    let mut a = [0u8; 32];
    a.copy_from_slice(&h[..32]);
    a[0] &= 248;
    a[31] &= 127;
    a[31] |= 64;

    // d = -121665/121666
    let d = Fe::ZERO
        .sub(Fe::small(121_665))
        .mul(Fe::small(121_666).invert());
    let d2 = d.add(d);
    let mut bx = hex32("216936d3cd6e53fec0a4e231fdd6dc5c692cc7609525a7b2c9562d608f25d51a");
    bx.reverse();
    let bx = Fe::from_bytes(&bx);
    let by = Fe::small(4).mul(Fe::small(5).invert());
    let base = Point {
        x: bx,
        y: by,
        z: Fe::ONE,
        t: bx.mul(by),
    };

    let mut p = Point {
        x: Fe::ZERO,
        y: Fe::ONE,
        z: Fe::ONE,
        t: Fe::ZERO,
    };
    for bit in (0..255).rev() {
        p = p.add(p, d2);
        if a[bit / 8] >> (bit % 8) & 1 == 1 {
            p = p.add(base, d2);
        }
    }
    let zi = p.z.invert();
    let mut out = p.y.mul(zi).to_bytes();
    out[31] |= (p.x.mul(zi).to_bytes()[0] & 1) << 7;
    out
}

fn hex32(h: &str) -> [u8; 32] {
    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&h[2 * i..2 * i + 2], 16).unwrap();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc8032_vectors() {
        let cases = [
            (
                "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60",
                "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a",
            ),
            (
                "4ccd089b28ff96da9db6c346ec114e0f5b8a319f35aba624da8cf6ed4fb8a6fb",
                "3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c",
            ),
        ];
        for (seed, public) in cases {
            assert_eq!(public_key(&hex32(seed)), hex32(public));
        }
    }
}
