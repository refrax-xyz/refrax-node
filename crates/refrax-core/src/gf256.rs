//! Arithmetic in GF(2⁸) with the reducing polynomial x⁸ + x⁴ + x³ + x² + 1 (0x11d) and
//! generator 2. Addition is XOR; multiplication goes through log / antilog tables built at
//! compile time.

const POLY: u16 = 0x11d;

const fn build() -> ([u8; 512], [u8; 256]) {
    let mut exp = [0u8; 512];
    let mut log = [0u8; 256];
    let mut x: u16 = 1;
    let mut i = 0;
    while i < 255 {
        exp[i] = x as u8;
        log[x as usize] = i as u8;
        x <<= 1;
        if x & 0x100 != 0 {
            x ^= POLY;
        }
        i += 1;
    }
    while i < 512 {
        exp[i] = exp[i - 255];
        i += 1;
    }
    (exp, log)
}

const TABLES: ([u8; 512], [u8; 256]) = build();
static EXP: [u8; 512] = TABLES.0;
static LOG: [u8; 256] = TABLES.1;

/// a + b (and a − b): XOR.
#[inline]
pub fn add(a: u8, b: u8) -> u8 {
    a ^ b
}

/// a · b
#[inline]
pub fn mul(a: u8, b: u8) -> u8 {
    if a == 0 || b == 0 {
        0
    } else {
        EXP[LOG[a as usize] as usize + LOG[b as usize] as usize]
    }
}

/// a⁻¹. Panics on zero, which has no inverse.
#[inline]
pub fn inv(a: u8) -> u8 {
    assert!(a != 0, "zero has no inverse in GF(256)");
    EXP[255 - LOG[a as usize] as usize]
}

/// a / b
#[inline]
pub fn div(a: u8, b: u8) -> u8 {
    mul(a, inv(b))
}

/// `dst[i] ^= c · src[i]` for every byte: the inner loop of encoding and decoding.
pub fn mul_add_slice(c: u8, src: &[u8], dst: &mut [u8]) {
    debug_assert_eq!(src.len(), dst.len());
    if c == 0 {
        return;
    }
    if c == 1 {
        for (d, s) in dst.iter_mut().zip(src) {
            *d ^= *s;
        }
        return;
    }
    let lc = LOG[c as usize] as usize;
    for (d, s) in dst.iter_mut().zip(src) {
        if *s != 0 {
            *d ^= EXP[lc + LOG[*s as usize] as usize];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_nonzero_element_has_an_inverse() {
        for a in 1..=255u8 {
            assert_eq!(mul(a, inv(a)), 1, "a = {a}");
        }
    }

    #[test]
    fn multiplication_is_commutative_and_distributive() {
        for a in 0..=255u8 {
            for b in [0u8, 1, 2, 3, 29, 76, 142, 255] {
                assert_eq!(mul(a, b), mul(b, a));
                for c in [0u8, 7, 200] {
                    assert_eq!(mul(a, add(b, c)), add(mul(a, b), mul(a, c)));
                }
            }
        }
    }

    #[test]
    fn mul_add_slice_matches_scalar() {
        let src: Vec<u8> = (0..=255).collect();
        let mut dst = vec![0x5au8; 256];
        mul_add_slice(37, &src, &mut dst);
        for (i, d) in dst.iter().enumerate() {
            assert_eq!(*d, 0x5a ^ mul(37, i as u8));
        }
    }
}
