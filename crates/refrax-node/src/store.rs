//! Facets on disk: `<data-dir>/facets/<object-id>/<facet>.facet`, written atomically.

use refrax_core::ObjectId;
use std::io;
use std::path::{Path, PathBuf};
use tokio::fs;

/// The node's facet store.
#[derive(Clone, Debug)]
pub struct FacetStore {
    root: PathBuf,
}

impl FacetStore {
    /// Open (and create) the store under `data_dir`.
    pub async fn open(data_dir: &Path) -> io::Result<Self> {
        let root = data_dir.join("facets");
        fs::create_dir_all(&root).await?;
        Ok(FacetStore { root })
    }

    fn path(&self, object: &ObjectId, facet: u8) -> PathBuf {
        self.root
            .join(object.to_string())
            .join(format!("{facet:02}.facet"))
    }

    /// Store a facet. Written to a temporary file first, then renamed into place.
    pub async fn put(&self, object: &ObjectId, facet: u8, bytes: &[u8]) -> io::Result<()> {
        let path = self.path(object, facet);
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).await?;
        }
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, bytes).await?;
        fs::rename(&tmp, &path).await
    }

    /// Read a facet, or `None` if it is not here.
    pub async fn get(&self, object: &ObjectId, facet: u8) -> io::Result<Option<Vec<u8>>> {
        match fs::read(self.path(object, facet)).await {
            Ok(b) => Ok(Some(b)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Number of facets and total bytes stored.
    pub async fn stats(&self) -> io::Result<(usize, u64)> {
        let (mut count, mut bytes) = (0usize, 0u64);
        let mut objects = fs::read_dir(&self.root).await?;
        while let Some(obj) = objects.next_entry().await? {
            if !obj.file_type().await?.is_dir() {
                continue;
            }
            let mut files = fs::read_dir(obj.path()).await?;
            while let Some(f) = files.next_entry().await? {
                if f.path().extension().is_some_and(|e| e == "facet") {
                    count += 1;
                    bytes += f.metadata().await?.len();
                }
            }
        }
        Ok((count, bytes))
    }

    /// Delete every facet. Devnet only: this is how you watch a node get slashed.
    pub async fn wipe(&self) -> io::Result<usize> {
        let (count, _) = self.stats().await?;
        fs::remove_dir_all(&self.root).await?;
        fs::create_dir_all(&self.root).await?;
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn put_get_stats_wipe() {
        let dir = tempfile::tempdir().unwrap();
        let store = FacetStore::open(dir.path()).await.unwrap();
        let id = ObjectId([7; 32]);
        assert!(store.get(&id, 3).await.unwrap().is_none());
        store.put(&id, 3, b"facet bytes").await.unwrap();
        assert_eq!(store.get(&id, 3).await.unwrap().unwrap(), b"facet bytes");
        assert_eq!(store.stats().await.unwrap(), (1, 11));
        assert_eq!(store.wipe().await.unwrap(), 1);
        assert!(store.get(&id, 3).await.unwrap().is_none());
    }
}
