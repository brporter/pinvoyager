//! Per-application upload quota. Explicit time keeps refill tests deterministic.
use std::{
    collections::HashMap,
    sync::{Mutex, PoisonError},
    time::{Duration, Instant},
};
use uuid::Uuid;

/// Entries idle for longer than this are dropped on the next sweep: a bucket
/// that has been full for two minutes is indistinguishable from a fresh one.
const BUCKET_IDLE: Duration = Duration::from_secs(120);

/// The map is only walked once it has grown enough to be worth walking, so the
/// common path stays one hash lookup.
const BUCKET_SWEEP_AT: usize = 4096;

struct Bucket {
    tokens: f64,
    updated: Instant,
}

#[derive(Default)]
pub struct UploadLimiter {
    buckets: Mutex<HashMap<Uuid, Bucket>>,
}

impl UploadLimiter {
    /// Takes one token, or reports how many seconds until one exists.
    ///
    /// A token bucket rather than a fixed window: a fixed window lets a user spend
    /// a whole minute's budget in the last second of one window and again in the
    /// first second of the next, which is twice the rate it claims to enforce, at
    /// exactly the moment it matters.
    pub fn take_at(&self, user_id: Uuid, per_minute: u32, now: Instant) -> Result<(), u64> {
        if per_minute == 0 {
            // Configured off. An honest 429 rather than the division by zero below.
            return Err(60);
        }

        let capacity = f64::from(per_minute);
        let per_second = capacity / 60.0;

        // A poisoned mutex means another caller panicked while holding it. What is
        // behind it is a rate-limit counter, not an invariant anything depends on,
        // so recovering beats propagating that panic into every later upload.
        let mut buckets = self.buckets.lock().unwrap_or_else(PoisonError::into_inner);

        if buckets.len() > BUCKET_SWEEP_AT {
            buckets.retain(|_, bucket| now.duration_since(bucket.updated) < BUCKET_IDLE);
        }

        let bucket = buckets.entry(user_id).or_insert(Bucket {
            tokens: capacity,
            updated: now,
        });

        let elapsed = now.duration_since(bucket.updated).as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * per_second).min(capacity);
        bucket.updated = now;

        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            return Ok(());
        }

        // Rounded up, floored at a second: a `Retry-After: 0` invites an immediate
        // retry that is guaranteed to fail again.
        let wait = ((1.0 - bucket.tokens) / per_second).ceil().max(1.0);
        Err(wait as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refill_and_independent_instances_use_explicit_time() {
        let limiter = UploadLimiter::default();
        let other = UploadLimiter::default();
        let user = Uuid::now_v7();
        let now = Instant::now();
        assert_eq!(limiter.take_at(user, 2, now), Ok(()));
        assert_eq!(limiter.take_at(user, 2, now), Ok(()));
        assert_eq!(limiter.take_at(user, 2, now), Err(30));
        assert_eq!(other.take_at(user, 2, now), Ok(()));
        assert_eq!(
            limiter.take_at(user, 2, now + Duration::from_secs(15)),
            Err(15)
        );
        assert_eq!(
            limiter.take_at(user, 2, now + Duration::from_secs(30)),
            Ok(())
        );
        assert_eq!(limiter.take_at(user, 0, now), Err(60));
    }

    #[test]
    fn sweep_removes_idle_users_and_keeps_recent_ones() {
        let limiter = UploadLimiter::default();
        let now = Instant::now();
        for _ in 0..=BUCKET_SWEEP_AT {
            assert!(limiter.take_at(Uuid::now_v7(), 1, now).is_ok());
        }
        let recent = Uuid::now_v7();
        assert!(
            limiter
                .take_at(recent, 1, now + Duration::from_secs(60))
                .is_ok()
        );
        assert!(
            limiter
                .take_at(Uuid::now_v7(), 1, now + BUCKET_IDLE)
                .is_ok()
        );
        let buckets = limiter.buckets.lock().expect("limiter lock");
        assert_eq!(buckets.len(), 2);
        assert!(buckets.contains_key(&recent));
    }

    #[test]
    fn poisoned_counter_does_not_disable_future_uploads() {
        let limiter = UploadLimiter::default();
        let _ = std::panic::catch_unwind(|| {
            let _lock = limiter.buckets.lock().expect("limiter lock");
            panic!("simulate interrupted counter update");
        });
        assert_eq!(limiter.take_at(Uuid::now_v7(), 1, Instant::now()), Ok(()));
    }
}
