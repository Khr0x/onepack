//! Caché del índice de búsqueda por feed (Fase 7). Construir el índice exige leer todas las
//! versiones listadas del feed; con varias búsquedas concurrentes eso domina la latencia de
//! los metadatos (ver docs/performance.md).
//!
//! Frescura: las mutaciones de este proceso (publicar, unlist/relist, bloquear) invalidan la
//! caché al momento. Las de otros procesos sobre el mismo directorio de datos (p. ej.
//! `onepackd package block`) se ven como mucho tras `TTL`. La descarga de una versión
//! bloqueada se comprueba siempre en la base, sin caché.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use onepack_core::Feed;
use onepack_nuget::v3::SearchIndex;
use onepack_storage::{Store, StoreError};

const TTL: Duration = Duration::from_secs(5);

/// Clave: feed, incluir prerelease, incluir SemVer 2.0.0.
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
    /// Invalida todo: lo llama cualquier cambio del catálogo en este proceso.
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
        // Se toma antes de leer: si algo cambia mientras se construye, la entrada nace ya
        // obsoleta y la siguiente búsqueda la reconstruye.
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
