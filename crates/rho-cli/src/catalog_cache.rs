//! Disk cache for the model catalogue.
//!
//! `rho-cli` owns the cache because `rho-core` has no disk-config dependency. The
//! cache is keyed by `Provider::catalog_fingerprint` and expires after 24 hours. A
//! listing over the byte cap is refused rather than truncated, so a user never sees a
//! silently shortened list. See `SPEC-choose-a-model-and-configure-a-run` section 7.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use rho_core::{CancelToken, ModelCatalog, ModelDescriptor, Provider, ProviderError};
use serde::{Deserialize, Serialize};
use tracing::warn;

/// The most bytes rho writes to the catalogue cache.
pub const MAX_CATALOG_BYTES: usize = 1024 * 1024;
/// A cached entry lives for 24 hours before it is considered stale.
const CACHE_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// One entry in the on-disk cache.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct CacheEntry {
    fetched_at: u64,
    models: Vec<ModelDescriptor>,
}

/// The on-disk cache file format.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct CacheFile {
    entries: HashMap<String, CacheEntry>,
}

/// A cached model catalogue. It reads `~/.rho/model-catalog-cache.json`, calls the
/// provider on a miss or stale entry, and writes the result back.
///
/// A mutex serializes reads and writes so the synchronous filesystem I/O never races
/// with itself. The provider fetch itself runs outside the lock.
pub struct CatalogCache {
    provider: Arc<dyn Provider>,
    path: PathBuf,
    lock: Mutex<()>,
}

impl CatalogCache {
    /// Build a cache that reads and writes `rho_dir/model-catalog-cache.json`.
    pub fn new(provider: Arc<dyn Provider>, rho_dir: &Path) -> Self {
        Self {
            provider,
            path: rho_dir.join("model-catalog-cache.json"),
            lock: Mutex::new(()),
        }
    }

    /// Read the cache file, or return an empty map on any error. A corrupt cache is
    /// treated like a missing cache; rho re-fetches rather than fail.
    fn read(&self) -> CacheFile {
        let bytes = match std::fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return CacheFile::default();
            }
            Err(error) => {
                warn!(path = %self.path.display(), %error, "could not read model-catalog cache");
                return CacheFile::default();
            }
        };
        if bytes.len() > MAX_CATALOG_BYTES {
            warn!(
                path = %self.path.display(),
                bytes = bytes.len(),
                max = MAX_CATALOG_BYTES,
                "model-catalog cache is larger than the byte cap; treating it as corrupt"
            );
            return CacheFile::default();
        }
        match serde_json::from_slice::<CacheFile>(&bytes) {
            Ok(file) => file,
            Err(error) => {
                warn!(path = %self.path.display(), %error, "model-catalog cache is corrupt; re-fetching");
                CacheFile::default()
            }
        }
    }

    /// Write the cache file, creating the directory if needed. A write failure is not
    /// returned to the caller, because a cache miss is recoverable on the next run.
    fn write(&self, file: &CacheFile) {
        let bytes = match serde_json::to_vec(file) {
            Ok(bytes) => bytes,
            Err(error) => {
                warn!(%error, "could not serialize model-catalog cache");
                return;
            }
        };
        if bytes.len() > MAX_CATALOG_BYTES {
            warn!(
                bytes = bytes.len(),
                max = MAX_CATALOG_BYTES,
                "model-catalog cache would exceed the byte cap; refusing to write"
            );
            return;
        }
        if let Some(parent) = self.path.parent()
            && let Err(error) = std::fs::create_dir_all(parent)
        {
            warn!(path = %parent.display(), %error, "could not create model-catalog cache directory");
            return;
        }
        let temp = self.path.with_extension("json.tmp");
        if let Err(error) = std::fs::write(&temp, bytes) {
            warn!(path = %temp.display(), %error, "could not write temporary model-catalog cache");
            return;
        }
        if let Err(error) = std::fs::rename(&temp, &self.path) {
            warn!(from = %temp.display(), to = %self.path.display(), %error, "could not install model-catalog cache");
            let _ = std::fs::remove_file(&temp);
        }
    }

    /// True when an entry is fresh enough to serve.
    fn is_fresh(entry: &CacheEntry) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::MAX)
            .as_secs();
        now.saturating_sub(entry.fetched_at) < CACHE_TTL.as_secs()
    }

    /// Return any cached models for the current provider fingerprint, stale or fresh,
    /// without starting a network call. Used to seed the picker while a fresh list loads.
    pub fn peek_cached_models(&self) -> Option<Vec<ModelDescriptor>> {
        let _guard = self.lock.lock().ok()?;
        let fingerprint = self.provider.catalog_fingerprint();
        let file = self.read();
        file.entries
            .get(&fingerprint)
            .map(|entry| entry.models.clone())
    }
}

#[async_trait]
impl ModelCatalog for CatalogCache {
    async fn list_models(
        &self,
        cancel: CancelToken,
    ) -> Result<Vec<ModelDescriptor>, ProviderError> {
        let fingerprint = self.provider.catalog_fingerprint();

        let cached = {
            let _guard = self.lock.lock().expect("the cache lock is never poisoned");
            self.read()
        };

        if let Some(entry) = cached.entries.get(&fingerprint)
            && Self::is_fresh(entry)
        {
            return Ok(entry.models.clone());
        }

        let catalog = self.provider.catalog().ok_or_else(|| {
            ProviderError::Unsupported("this provider cannot list models".to_string())
        })?;
        let models = catalog.list_models(cancel).await?;

        let fetched_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::MAX)
            .as_secs();

        {
            let _guard = self.lock.lock().expect("the cache lock is never poisoned");
            let mut file = self.read();
            // Evict stale entries so the cache file does not grow without bound as a
            // user switches endpoints or fingerprints.
            file.entries.retain(|_, entry| Self::is_fresh(entry));
            file.entries.insert(
                fingerprint,
                CacheEntry {
                    fetched_at,
                    models: models.clone(),
                },
            );
            self.write(&file);
        }

        Ok(models)
    }

    fn peek_cached_models(&self) -> Option<Vec<ModelDescriptor>> {
        self.peek_cached_models()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use rho_core::{CompletionRequest, ProviderStream};
    use tempfile::TempDir;

    struct ListProvider {
        fingerprint: String,
        models: Vec<ModelDescriptor>,
    }

    #[async_trait]
    impl Provider for ListProvider {
        fn id(&self) -> &str {
            "list"
        }

        fn catalog(&self) -> Option<&dyn ModelCatalog> {
            Some(self)
        }

        fn catalog_fingerprint(&self) -> String {
            self.fingerprint.clone()
        }

        async fn stream(
            &self,
            _request: CompletionRequest,
            _cancel: CancelToken,
        ) -> Result<ProviderStream, ProviderError> {
            unreachable!()
        }
    }

    #[async_trait]
    impl ModelCatalog for ListProvider {
        async fn list_models(
            &self,
            _cancel: CancelToken,
        ) -> Result<Vec<ModelDescriptor>, ProviderError> {
            Ok(self.models.clone())
        }
    }

    fn descriptor(id: &str) -> ModelDescriptor {
        ModelDescriptor {
            id: id.to_string(),
            display_name: None,
        }
    }

    #[tokio::test]
    async fn cache_writes_and_reads_back_a_fresh_entry() {
        let dir = TempDir::new().unwrap();
        let provider = Arc::new(ListProvider {
            fingerprint: "fp".to_string(),
            models: vec![descriptor("a")],
        });
        let cache = CatalogCache::new(provider.clone(), dir.path());

        let first = cache.list_models(CancelToken::new()).await.unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].id, "a");

        // A second cache pointing at the same file, with a provider that would return a
        // different list, must still serve the freshly written entry.
        let stale_provider = Arc::new(ListProvider {
            fingerprint: "fp".to_string(),
            models: vec![descriptor("b")],
        });
        let second_cache = CatalogCache::new(stale_provider, dir.path());
        let second = second_cache.list_models(CancelToken::new()).await.unwrap();
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].id, "a");
    }

    #[tokio::test]
    async fn a_different_fingerprint_writes_a_separate_entry() {
        let dir = TempDir::new().unwrap();
        let a = Arc::new(ListProvider {
            fingerprint: "a".to_string(),
            models: vec![descriptor("a-model")],
        });
        let cache_a = CatalogCache::new(a, dir.path());
        let a_models = cache_a.list_models(CancelToken::new()).await.unwrap();
        assert_eq!(a_models[0].id, "a-model");

        let b = Arc::new(ListProvider {
            fingerprint: "b".to_string(),
            models: vec![descriptor("b-model")],
        });
        let cache_b = CatalogCache::new(b, dir.path());
        let b_models = cache_b.list_models(CancelToken::new()).await.unwrap();
        assert_eq!(b_models[0].id, "b-model");

        // Re-reading a must still return a-model, proving the entries are separate.
        let a_again = cache_a.list_models(CancelToken::new()).await.unwrap();
        assert_eq!(a_again[0].id, "a-model");
    }

    // A provider fingerprint that changes with its endpoint (for example a Bedrock
    // region) must write a new cache entry. Without this a moved endpoint would serve
    // the old endpoint's list.
    #[tokio::test]
    async fn cache_key_changes_with_region() {
        let dir = TempDir::new().unwrap();
        let us = Arc::new(ListProvider {
            fingerprint: "us-east-1".to_string(),
            models: vec![descriptor("us-model")],
        });
        let eu = Arc::new(ListProvider {
            fingerprint: "eu-west-1".to_string(),
            models: vec![descriptor("eu-model")],
        });

        let cache_us = CatalogCache::new(us, dir.path());
        let cache_eu = CatalogCache::new(eu, dir.path());

        assert_eq!(
            cache_us.list_models(CancelToken::new()).await.unwrap()[0].id,
            "us-model"
        );
        assert_eq!(
            cache_eu.list_models(CancelToken::new()).await.unwrap()[0].id,
            "eu-model"
        );
        // Re-reading the US cache must not return the EU model.
        assert_eq!(
            cache_us.list_models(CancelToken::new()).await.unwrap()[0].id,
            "us-model"
        );
    }

    // A stale entry is not served as fresh. The cache re-fetches, so the user never
    // sees a silently outdated list. The "marked stale" UI behaviour is tested in
    // `crates/rho-tui` because `CatalogCache` itself returns only fresh or re-fetched
    // data.
    #[tokio::test]
    async fn stale_cache_is_not_served_as_fresh() {
        let dir = TempDir::new().unwrap();
        let provider = Arc::new(ListProvider {
            fingerprint: "fp".to_string(),
            models: vec![descriptor("fresh")],
        });
        let cache = CatalogCache::new(provider.clone(), dir.path());
        assert_eq!(
            cache.list_models(CancelToken::new()).await.unwrap()[0].id,
            "fresh"
        );

        // Write an entry that is older than the 24-hour TTL. A manual write keeps the
        // test deterministic and avoids a sleep.
        let stale = CacheFile {
            entries: HashMap::from([(
                "fp".to_string(),
                CacheEntry {
                    fetched_at: 0,
                    models: vec![descriptor("stale")],
                },
            )]),
        };
        cache.write(&stale);

        // The cache must ignore the stale entry and re-fetch from the provider.
        assert_eq!(
            cache.list_models(CancelToken::new()).await.unwrap()[0].id,
            "fresh"
        );
    }
}
