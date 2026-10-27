//! Systematic Reed–Solomon erasure coding over GF(2⁸).
//!
//! The generator matrix is the identity on top of a Cauchy matrix. Every square submatrix of a
//! Cauchy matrix is invertible, so any `k` rows of the generator form an invertible matrix: any
//! `k` facets rebuild the data (an MDS code). The first `k` facets are the data itself.

use crate::{gf256, CHUNK, FACETS, REBUILD};
use thiserror::Error;

/// Why encoding or decoding failed.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CodecError {
    /// `k` and `n` must satisfy `0 < k < n <= 256`.
    #[error("invalid code parameters k={k} n={n}")]
    Params {
        /// data facets
        k: usize,
        /// total facets
        n: usize,
    },
    /// Fewer distinct facets than the code needs.
    #[error("need {need} distinct facets to rebuild, have {have}")]
    NotEnough {
        /// facets required
        need: usize,
        /// distinct facets supplied
        have: usize,
    },
    /// A facet index is outside `0..n`.
    #[error("facet index {0} is out of range")]
    BadIndex(usize),
    /// Facets differ in length, or the length does not match the manifest.
    #[error("facet {0} has the wrong length")]
    BadLength(usize),
}

/// A `(k, n)` systematic Reed–Solomon code.
#[derive(Clone, Debug)]
pub struct Codec {
    k: usize,
    n: usize,
    /// `(n − k) × k` Cauchy rows that produce the parity facets.
    parity: Vec<Vec<u8>>,
}

impl Codec {
    /// Build a code with `k` data facets out of `n`.
    pub fn new(k: usize, n: usize) -> Result<Self, CodecError> {
        if k == 0 || k >= n || n > 256 {
            return Err(CodecError::Params { k, n });
        }
        // x_r = k + r and y_c = c are disjoint, so x_r ⊕ y_c is never zero.
        let parity = (0..n - k)
            .map(|r| (0..k).map(|c| gf256::inv(((k + r) ^ c) as u8)).collect())
            .collect();
        Ok(Codec { k, n, parity })
    }

    /// The Refrax code: 16 facets, any 5 rebuild.
    pub fn standard() -> Self {
        Codec::new(REBUILD, FACETS).expect("valid parameters")
    }

    /// Data facets.
    pub fn k(&self) -> usize {
        self.k
    }

    /// Total facets.
    pub fn n(&self) -> usize {
        self.n
    }

    fn row(&self, i: usize) -> Vec<u8> {
        if i < self.k {
            let mut r = vec![0u8; self.k];
            r[i] = 1;
            r
        } else {
            self.parity[i - self.k].clone()
        }
    }

    /// Turn `k` equally sized data facets into `n` facets. The first `k` are the data unchanged.
    pub fn encode(&self, data: &[Vec<u8>]) -> Result<Vec<Vec<u8>>, CodecError> {
        if data.len() != self.k {
            return Err(CodecError::NotEnough {
                need: self.k,
                have: data.len(),
            });
        }
        let len = data[0].len();
        if let Some(i) = data.iter().position(|d| d.len() != len) {
            return Err(CodecError::BadLength(i));
        }
        let mut out: Vec<Vec<u8>> = data.to_vec();
        for row in &self.parity {
            let mut p = vec![0u8; len];
            for (c, coef) in row.iter().enumerate() {
                gf256::mul_add_slice(*coef, &data[c], &mut p);
            }
            out.push(p);
        }
        Ok(out)
    }

    /// Rebuild the `k` data facets from any `k` distinct facets given as `(index, bytes)`.
    /// Extra facets are ignored.
    pub fn reconstruct(&self, facets: &[(usize, &[u8])]) -> Result<Vec<Vec<u8>>, CodecError> {
        let mut picked: Vec<(usize, &[u8])> = Vec::with_capacity(self.k);
        for (i, bytes) in facets {
            if *i >= self.n {
                return Err(CodecError::BadIndex(*i));
            }
            if picked.iter().all(|(j, _)| j != i) {
                picked.push((*i, bytes));
            }
            if picked.len() == self.k {
                break;
            }
        }
        if picked.len() < self.k {
            return Err(CodecError::NotEnough {
                need: self.k,
                have: picked.len(),
            });
        }
        let len = picked[0].1.len();
        if let Some((i, _)) = picked.iter().find(|(_, b)| b.len() != len) {
            return Err(CodecError::BadLength(*i));
        }

        // Fast path: all data facets present.
        picked.sort_by_key(|(i, _)| *i);
        if picked.iter().enumerate().all(|(pos, (i, _))| pos == *i) {
            return Ok(picked.iter().map(|(_, b)| b.to_vec()).collect());
        }

        let matrix: Vec<Vec<u8>> = picked.iter().map(|(i, _)| self.row(*i)).collect();
        let inverse =
            invert(matrix).expect("any k rows of a Cauchy-systematic generator are invertible");
        let mut data = vec![vec![0u8; len]; self.k];
        for (r, out) in data.iter_mut().enumerate() {
            for (c, (_, bytes)) in picked.iter().enumerate() {
                gf256::mul_add_slice(inverse[r][c], bytes, out);
            }
        }
        Ok(data)
    }
}

/// Gauss–Jordan inversion over GF(2⁸). `None` if the matrix is singular.
fn invert(mut m: Vec<Vec<u8>>) -> Option<Vec<Vec<u8>>> {
    let n = m.len();
    let mut inv: Vec<Vec<u8>> = (0..n)
        .map(|i| {
            let mut r = vec![0u8; n];
            r[i] = 1;
            r
        })
        .collect();
    for col in 0..n {
        let pivot = (col..n).find(|&r| m[r][col] != 0)?;
        m.swap(col, pivot);
        inv.swap(col, pivot);
        let scale = gf256::inv(m[col][col]);
        for j in 0..n {
            m[col][j] = gf256::mul(m[col][j], scale);
            inv[col][j] = gf256::mul(inv[col][j], scale);
        }
        for r in 0..n {
            if r != col && m[r][col] != 0 {
                let f = m[r][col];
                for j in 0..n {
                    m[r][j] ^= gf256::mul(f, m[col][j]);
                    inv[r][j] ^= gf256::mul(f, inv[col][j]);
                }
            }
        }
    }
    Some(inv)
}

/// An object cut into facets, ready to scatter.
#[derive(Clone, Debug)]
pub struct Encoded {
    /// Original size in bytes.
    pub size: u64,
    /// Length of every facet: a whole number of [`CHUNK`]s.
    pub shard_len: usize,
    /// `FACETS` facets; index = facet number.
    pub facets: Vec<Vec<u8>>,
}

/// Facet length for an object of `size` bytes: `size / REBUILD`, rounded up to whole chunks.
pub fn shard_len_for(size: u64) -> usize {
    let per = (size as usize).div_ceil(REBUILD).max(1);
    per.div_ceil(CHUNK) * CHUNK
}

/// Cut `data` into the standard 16 facets.
pub fn encode_object(data: &[u8]) -> Encoded {
    let shard_len = shard_len_for(data.len() as u64);
    let mut padded = data.to_vec();
    padded.resize(shard_len * REBUILD, 0);
    let shards: Vec<Vec<u8>> = padded.chunks(shard_len).map(|c| c.to_vec()).collect();
    let facets = Codec::standard().encode(&shards).expect("equal shards");
    Encoded {
        size: data.len() as u64,
        shard_len,
        facets,
    }
}

/// Rebuild an object of `size` bytes from any 5 of its facets.
pub fn decode_object(
    facets: &[(usize, &[u8])],
    size: u64,
    shard_len: usize,
) -> Result<Vec<u8>, CodecError> {
    if let Some((i, _)) = facets.iter().find(|(_, b)| b.len() != shard_len) {
        return Err(CodecError::BadLength(*i));
    }
    let data = Codec::standard().reconstruct(facets)?;
    let mut out: Vec<u8> = data.concat();
    out.truncate(size as usize);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{seq::SliceRandom, Rng, SeedableRng};

    fn sample(len: usize, seed: u64) -> Vec<u8> {
        let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
        (0..len).map(|_| rng.gen()).collect()
    }

    /// All C(16, 5) = 4368 subsets of five facets rebuild the object.
    #[test]
    fn every_five_facet_subset_rebuilds() {
        let data = sample(3 * CHUNK + 17, 1);
        let enc = encode_object(&data);
        assert_eq!(enc.facets.len(), FACETS);
        let mut count = 0;
        for a in 0..16 {
            for b in a + 1..16 {
                for c in b + 1..16 {
                    for d in c + 1..16 {
                        for e in d + 1..16 {
                            let pick: Vec<(usize, &[u8])> = [a, b, c, d, e]
                                .iter()
                                .map(|&i| (i, enc.facets[i].as_slice()))
                                .collect();
                            assert_eq!(
                                decode_object(&pick, enc.size, enc.shard_len).unwrap(),
                                data
                            );
                            count += 1;
                        }
                    }
                }
            }
        }
        assert_eq!(count, 4368);
    }

    #[test]
    fn four_facets_are_not_enough() {
        let enc = encode_object(b"hello refrax");
        let pick: Vec<(usize, &[u8])> = (0..4)
            .map(|i| (i * 3, enc.facets[i * 3].as_slice()))
            .collect();
        assert_eq!(
            decode_object(&pick, enc.size, enc.shard_len),
            Err(CodecError::NotEnough { need: 5, have: 4 })
        );
    }

    #[test]
    fn duplicates_do_not_count_twice() {
        let enc = encode_object(b"duplicate");
        let f = enc.facets[7].as_slice();
        let pick = vec![(7, f), (7, f), (7, f), (7, f), (7, f)];
        assert!(matches!(
            decode_object(&pick, enc.size, enc.shard_len),
            Err(CodecError::NotEnough { .. })
        ));
    }

    #[test]
    fn data_facets_are_the_data() {
        let data = sample(5 * CHUNK, 2);
        let enc = encode_object(&data);
        assert_eq!(enc.shard_len, CHUNK);
        assert_eq!(enc.facets[..5].concat(), data);
    }

    #[test]
    fn random_sizes_and_subsets() {
        let mut rng = rand::rngs::StdRng::seed_from_u64(9);
        for round in 0..40 {
            let data = sample(rng.gen_range(0..40_000), round);
            let enc = encode_object(&data);
            let mut idx: Vec<usize> = (0..FACETS).collect();
            idx.shuffle(&mut rng);
            let pick: Vec<(usize, &[u8])> = idx[..5]
                .iter()
                .map(|&i| (i, enc.facets[i].as_slice()))
                .collect();
            assert_eq!(decode_object(&pick, enc.size, enc.shard_len).unwrap(), data);
        }
    }

    #[test]
    fn shard_length_is_whole_chunks() {
        assert_eq!(shard_len_for(0), CHUNK);
        assert_eq!(shard_len_for(5 * 1024), 1024);
        assert_eq!(shard_len_for(5 * 1024 + 1), 2048);
    }

    #[test]
    fn rejects_bad_parameters() {
        assert!(Codec::new(0, 4).is_err());
        assert!(Codec::new(4, 4).is_err());
        assert!(Codec::new(10, 300).is_err());
    }
}
