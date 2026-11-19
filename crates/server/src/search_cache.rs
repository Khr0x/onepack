//! Per-feed search index cache (Phase 7). Building the index requires reading all
//! listed versions of the feed; with several concurrent searches that dominates the metadata
//! latency (see docs/performance.md).
//!
//! Freshness: mutations in this process (publish, unlist/relist, block) invalidate the
//! cache immediately. Those from other processes on the same data directory (e.g.
//! `onepackd package block`) are visible at most after `TTL`. Downloading a
//! blocked version is always checked in the database, without cache.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use onepack_core::Feed;
use onepack_nuget::v3::SearchIndex;
use onepack_storage::{Store, StoreError};

const TTL: Duration = Duration::from_secs(5);

/// Key: feed, include prerelease, include SemVer 2.0.0.
type Key = (i64, bool, bool);

struct Entry {
    generation: u64,
    built: Instant,
    index: Arc<SearchIndex>,
}

#[derive(Default)]
pub struct SearchCache {
    generation: AtomicU64,
    entries: Mutex<HashMap<Key, Entry>>,
}

impl SearchCache {
    /// Invalidates everything: called by any catalog change in this process.
    pub fn invalidate(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
    }

    pub async fn get(
        &self,
        store: &Store,
        feed: &Feed,
        prerelease: bool,
        semver2: bool,
    ) -> Result<Arc<SearchIndex>, StoreError> {
        let key = (feed.id, prerelease, semver2);
        // It is taken before reading: if something changes while it is being built, the entry is born already
        // stale and the next search rebuilds it.
        let generation = self.generation.load(Ordering::SeqCst);
        if let Some(e) = self.entries.lock().expect("mutex not poisoned").get(&key)
            && e.generation == generation
            && e.built.elapsed() < TTL
        {
            return Ok(e.index.clone());
        }
        let versions = store.listed_versions(feed, prerelease, semver2).await?;
        let index = Arc::new(SearchIndex::new(versions));
        self.entries.lock().expect("mutex not poisoned").insert(
            key,
            Entry {
                generation,
                built: Instant::now(),
                index: index.clone(),
            },
        );
        Ok(index)
    }
}
