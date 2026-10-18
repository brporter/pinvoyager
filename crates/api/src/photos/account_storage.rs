//! Bounded, short-lived account storage measurements shared by administration pages.
use super::{
    VariantKind,
    storage::{self, ContentHash},
};
use crate::error::{AppError, AppResult};
use sqlx::PgPool;
use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, Weak},
    time::{Duration, Instant},
};
use tokio::sync::Mutex;
use uuid::Uuid;

const TTL: Duration = Duration::from_secs(60);
const CAPACITY: usize = 256;
const HASH_PAGE: i64 = 1000;

#[derive(Default)]
struct Cache(HashMap<Uuid, (Instant, i64)>);
impl Cache {
    fn get(&self, id: Uuid, now: Instant) -> Option<i64> {
        self.0
            .get(&id)
            .filter(|(measured, _)| now.duration_since(*measured) < TTL)
            .map(|(_, bytes)| *bytes)
    }

    fn insert(&mut self, id: Uuid, bytes: i64, now: Instant) {
        self.0
            .retain(|_, (measured, _)| now.duration_since(*measured) < TTL);
        if !self.0.contains_key(&id)
            && self.0.len() >= CAPACITY
            && let Some(oldest) = self
                .0
                .iter()
                .min_by_key(|(_, (time, _))| *time)
                .map(|(id, _)| *id)
        {
            self.0.remove(&oldest);
        }
        self.0.insert(id, (now, bytes));
    }
}

/// At most 256 account totals per API process, valid for 60 seconds. Concurrent
/// callers share a measurement; errors are returned without caching partial totals.
#[derive(Default)]
pub struct AccountStorage(
    Mutex<Cache>,
    std::sync::Mutex<HashMap<Uuid, Weak<Mutex<()>>>>,
);
impl AccountStorage {
    pub(crate) async fn totals(
        &self,
        pool: &PgPool,
        root: &Path,
        ids: &[Uuid],
    ) -> AppResult<HashMap<Uuid, i64>> {
        let cache = self.0.lock().await;
        let now = Instant::now();
        let mut totals = HashMap::new();
        let mut missing = Vec::new();
        for &id in ids {
            if let Some(bytes) = cache.get(id, now) {
                totals.insert(id, bytes);
            } else {
                missing.push(id);
            }
        }
        drop(cache);
        if missing.is_empty() {
            return Ok(totals);
        }
        // Acquire account locks in a consistent order for overlapping pages.
        // Cache hits never wait for these locks or for filesystem measurements.
        missing.sort_unstable();
        missing.dedup();
        let locks: Vec<_> = {
            let mut locks = self
                .1
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            locks.retain(|_, lock| lock.strong_count() > 0);
            missing
                .iter()
                .map(|id| {
                    let lock = locks.get(id).and_then(Weak::upgrade).unwrap_or_default();
                    locks.insert(*id, Arc::downgrade(&lock));
                    lock
                })
                .collect()
        };
        let mut guards = Vec::with_capacity(locks.len());
        for lock in locks {
            guards.push(lock.lock_owned().await);
        }
        // A caller that just released an account lock may have filled its cache.
        let cache = self.0.lock().await;
        let now = Instant::now();
        missing.retain(|id| {
            if let Some(bytes) = cache.get(*id, now) {
                totals.insert(*id, bytes);
                false
            } else {
                true
            }
        });
        drop(cache);
        if missing.is_empty() {
            return Ok(totals);
        }
        let mut measured = HashMap::<Uuid, i64>::new();
        let mut after: Option<(Uuid, String)> = None;
        loop {
            let hashes: Vec<(Uuid, String)> = sqlx::query_as(
                "SELECT DISTINCT owner_id, content_hash FROM photos WHERE owner_id=ANY($1)
                 AND ($2::uuid IS NULL OR (owner_id,content_hash)>($2,$3))
                 ORDER BY owner_id, content_hash LIMIT $4",
            )
            .bind(&missing)
            .bind(after.as_ref().map(|(id, _)| *id))
            .bind(after.as_ref().map(|(_, hash)| hash))
            .bind(HASH_PAGE)
            .fetch_all(pool)
            .await?;
            let count = hashes.len();
            after = hashes.last().cloned();
            let root = root.to_path_buf();
            // The query connection is released before any filesystem work.
            measured = crate::blocking::run_blocking(move || {
                for (owner, hash) in hashes {
                    let hash = ContentHash::try_from(hash)?;
                    let paths = std::iter::once(storage::original_path(&root, &hash)).chain(
                        VariantKind::ALL
                            .into_iter()
                            .map(|kind| storage::variant_path(&root, &hash, kind)),
                    );
                    for path in paths {
                        match std::fs::metadata(path) {
                            Ok(metadata) => {
                                *measured.entry(owner).or_default() += metadata.len() as i64
                            }
                            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                            Err(error) => return Err(AppError::Internal(Box::new(error))),
                        }
                    }
                }
                Ok(measured)
            })
            .await?;
            if count < HASH_PAGE as usize {
                break;
            }
        }
        let mut cache = self.0.lock().await;
        for id in missing {
            let bytes = measured.get(&id).copied().unwrap_or_default();
            cache.insert(id, bytes, Instant::now());
            totals.insert(id, bytes);
        }
        Ok(totals)
    }
}

#[cfg(test)]
#[path = "account_storage_tests.rs"]
mod tests;
