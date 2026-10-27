//! Binary SHA-256 merkle trees with domain-separated leaves and nodes.
//!
//! * leaf  = H(0x00 ‖ data)
//! * node  = H(0x01 ‖ left ‖ right)
//! * an odd node at the end of a level is promoted unchanged (no duplication, so two different
//!   leaf lists can never share a root).
//!
//! Proofs carry only sibling hashes. The verifier rebuilds the path from the leaf index and the
//! leaf count, so a proof is bound to one position in the tree.

use crate::Hash;

/// Hash of a leaf.
pub fn leaf(data: &[u8]) -> Hash {
    Hash::digest(&[&[0x00], data])
}

/// Hash of an inner node.
pub fn node(left: &Hash, right: &Hash) -> Hash {
    Hash::digest(&[&[0x01], &left.0, &right.0])
}

/// A tree kept level by level, leaves first.
#[derive(Clone, Debug)]
pub struct MerkleTree {
    levels: Vec<Vec<Hash>>,
}

impl MerkleTree {
    /// Build from leaf hashes. Panics on an empty list.
    pub fn from_leaves(leaves: Vec<Hash>) -> Self {
        assert!(!leaves.is_empty(), "a merkle tree needs at least one leaf");
        let mut levels = vec![leaves];
        while levels.last().map_or(0, Vec::len) > 1 {
            let prev = levels.last().expect("non-empty");
            let next = prev
                .chunks(2)
                .map(|p| {
                    if p.len() == 2 {
                        node(&p[0], &p[1])
                    } else {
                        p[0]
                    }
                })
                .collect();
            levels.push(next);
        }
        MerkleTree { levels }
    }

    /// Build over `data` cut into `chunk`-sized leaves.
    pub fn from_chunks(data: &[u8], chunk: usize) -> Self {
        let leaves = if data.is_empty() {
            vec![leaf(&[])]
        } else {
            data.chunks(chunk).map(leaf).collect()
        };
        Self::from_leaves(leaves)
    }

    /// The root hash.
    pub fn root(&self) -> Hash {
        self.levels.last().expect("non-empty")[0]
    }

    /// Number of leaves.
    pub fn leaf_count(&self) -> usize {
        self.levels[0].len()
    }

    /// Sibling hashes from leaf `index` up to the root.
    pub fn proof(&self, index: usize) -> Vec<Hash> {
        assert!(index < self.leaf_count(), "leaf index out of range");
        let mut out = Vec::new();
        let mut idx = index;
        for level in &self.levels[..self.levels.len() - 1] {
            if idx % 2 == 1 {
                out.push(level[idx - 1]);
            } else if idx + 1 < level.len() {
                out.push(level[idx + 1]);
            }
            idx /= 2;
        }
        out
    }
}

/// Check that `leaf` sits at `index` in a tree of `leaf_count` leaves with this `root`.
pub fn verify(root: &Hash, index: usize, leaf_count: usize, leaf: Hash, siblings: &[Hash]) -> bool {
    if index >= leaf_count {
        return false;
    }
    let mut h = leaf;
    let mut idx = index;
    let mut n = leaf_count;
    let mut it = siblings.iter();
    while n > 1 {
        if idx % 2 == 1 {
            match it.next() {
                Some(s) => h = node(s, &h),
                None => return false,
            }
        } else if idx + 1 < n {
            match it.next() {
                Some(s) => h = node(&h, s),
                None => return false,
            }
        }
        idx /= 2;
        n = n.div_ceil(2);
    }
    it.next().is_none() && h == *root
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proofs_verify_for_every_leaf_and_size() {
        for count in 1..=33 {
            let leaves: Vec<Hash> = (0..count).map(|i: u32| leaf(&i.to_le_bytes())).collect();
            let tree = MerkleTree::from_leaves(leaves.clone());
            for (i, l) in leaves.iter().enumerate() {
                let p = tree.proof(i);
                assert!(
                    verify(&tree.root(), i, count as usize, *l, &p),
                    "count {count} index {i}"
                );
                // the same proof does not work for a neighbour position
                if count > 1 {
                    let other = (i + 1) % count as usize;
                    assert!(!verify(&tree.root(), other, count as usize, *l, &p));
                }
            }
        }
    }

    #[test]
    fn tampered_leaf_fails() {
        let tree = MerkleTree::from_chunks(&[7u8; 4096], 1024);
        let p = tree.proof(2);
        assert!(!verify(&tree.root(), 2, 4, leaf(&[8u8; 1024]), &p));
    }

    #[test]
    fn leaves_and_nodes_are_separated() {
        let a = leaf(b"a");
        let b = leaf(b"b");
        let two = MerkleTree::from_leaves(vec![a, b]).root();
        // A single leaf whose bytes happen to be the concatenation of two child hashes
        // must not collide with the two-leaf tree.
        let fake = leaf(&[a.0, b.0].concat());
        assert_ne!(two, fake);
    }

    #[test]
    fn promotion_does_not_duplicate() {
        let l: Vec<Hash> = (0..3u8).map(|i| leaf(&[i])).collect();
        let mut dup = l.clone();
        dup.push(l[2]);
        assert_ne!(
            MerkleTree::from_leaves(l).root(),
            MerkleTree::from_leaves(dup).root()
        );
    }
}
