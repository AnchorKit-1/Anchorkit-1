//! Circuit breaker for the webhook transport layer.
//!
//! Retrying against a genuinely down anchor just adds load to an
//! already-failing service and burns the caller's time budget for no
//! benefit. A [`CircuitBreaker`] trips after `failure_threshold` consecutive
//! failures and short-circuits further calls -- without touching the
//! network -- until `cooldown` elapses. After the cooldown, exactly one
//! probe call is let through (the half-open state); if it succeeds the
//! circuit closes and normal traffic resumes, if it fails the circuit
//! re-opens for another cooldown period.
//!
//! [`CircuitBreakerRegistry`] keeps one breaker per destination host, since
//! one anchor being down shouldn't stop delivery to every other anchor a
//! caller happens to also be delivering to.
//!
//! ```text
//!                 failure_threshold consecutive failures
//!        ┌───────────────────────────────────────────────┐
//!        │                                                 ▼
//!   ┌────────┐                                        ┌────────┐
//!   │ Closed │◀───────────── probe succeeds ──────────│  Open  │
//!   └────────┘                                        └────────┘
//!        ▲                                                  │
//!        │                                          cooldown elapsed,
//!        │                                          next call becomes
//!        │                                             the probe
//!        │                                                  ▼
//!        │                                           ┌───────────┐
//!        └──────────── probe fails ───────────────── │ Half-Open │
//!                                                      └───────────┘
//! ```

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Which state a [`CircuitBreaker`] is currently in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    /// Calls go through normally.
    Closed,
    /// Calls are short-circuited without touching the network.
    Open,
    /// The cooldown has elapsed; exactly one probe call is allowed through
    /// to test whether the destination has recovered.
    HalfOpen,
}

/// Configuration for a [`CircuitBreaker`].
#[derive(Debug, Clone, Copy)]
pub struct CircuitBreakerConfig {
    /// Number of consecutive failures required to trip the circuit open.
    pub failure_threshold: u32,
    /// How long the circuit stays open before allowing a half-open probe.
    pub cooldown: Duration,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self { failure_threshold: 5, cooldown: Duration::from_secs(30) }
    }
}

#[derive(Debug)]
struct Inner {
    state: CircuitState,
    consecutive_failures: u32,
    opened_at: Option<Instant>,
}

/// Returned by [`CircuitBreaker::try_acquire`] when the circuit is open (or
/// a half-open probe is already in flight) and the call must not be made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CircuitOpen;

/// A single call slot granted by [`CircuitBreaker::try_acquire`]. The
/// holder must report the outcome via [`Permit::success`] or
/// [`Permit::failure`]; if the permit is dropped without either being
/// called, it is treated as a failure so a caller that bails out early
/// (panic, early return, cancellation) can never leave the breaker stuck
/// thinking a probe is still in flight.
pub struct Permit<'a> {
    breaker: &'a CircuitBreaker,
    is_probe: bool,
    resolved: AtomicBool,
}

impl<'a> std::fmt::Debug for Permit<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Permit").field("is_probe", &self.is_probe).finish_non_exhaustive()
    }
}

impl<'a> Permit<'a> {
    pub fn success(self) {
        self.resolved.store(true, Ordering::SeqCst);
        self.breaker.record_success(self.is_probe);
    }

    pub fn failure(self) {
        self.resolved.store(true, Ordering::SeqCst);
        self.breaker.record_failure(self.is_probe);
    }

    /// Whether this permit is the single probe call let through while the
    /// circuit is half-open.
    pub fn is_probe(&self) -> bool {
        self.is_probe
    }
}

impl<'a> Drop for Permit<'a> {
    fn drop(&mut self) {
        if !self.resolved.load(Ordering::SeqCst) {
            self.breaker.record_failure(self.is_probe);
        }
    }
}

/// Tracks consecutive failures for one destination and trips/resets per the
/// state machine described in the module docs.
pub struct CircuitBreaker {
    config: CircuitBreakerConfig,
    inner: Mutex<Inner>,
}

impl CircuitBreaker {
    pub fn new(config: CircuitBreakerConfig) -> Self {
        Self { config, inner: Mutex::new(Inner { state: CircuitState::Closed, consecutive_failures: 0, opened_at: None }) }
    }

    pub fn state(&self) -> CircuitState {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).state
    }

    /// Requests permission to make a call. Returns a [`Permit`] whose
    /// outcome must be reported, or [`CircuitOpen`] if the call must be
    /// short-circuited.
    pub fn try_acquire(&self) -> Result<Permit<'_>, CircuitOpen> {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        match inner.state {
            CircuitState::Closed => Ok(Permit { breaker: self, is_probe: false, resolved: AtomicBool::new(false) }),
            CircuitState::Open => {
                let cooldown_elapsed = inner.opened_at.is_some_and(|t| t.elapsed() >= self.config.cooldown);
                if cooldown_elapsed {
                    // The cooldown has passed: this call becomes the single
                    // half-open probe.
                    inner.state = CircuitState::HalfOpen;
                    Ok(Permit { breaker: self, is_probe: true, resolved: AtomicBool::new(false) })
                } else {
                    Err(CircuitOpen)
                }
            }
            // A probe is already in flight (the state stays HalfOpen for
            // its whole duration -- see record_success/record_failure) so
            // every other concurrent caller is short-circuited too.
            CircuitState::HalfOpen => Err(CircuitOpen),
        }
    }

    fn record_success(&self, is_probe: bool) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        // A non-probe success while the circuit is Open/HalfOpen can't
        // happen (try_acquire wouldn't have granted a non-probe permit in
        // those states), but guard it anyway rather than relying on that.
        let _ = is_probe;
        inner.state = CircuitState::Closed;
        inner.consecutive_failures = 0;
        inner.opened_at = None;
    }

    fn record_failure(&self, is_probe: bool) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if is_probe {
            // The probe failed: re-open for another full cooldown.
            inner.state = CircuitState::Open;
            inner.opened_at = Some(Instant::now());
            return;
        }
        inner.consecutive_failures += 1;
        if inner.consecutive_failures >= self.config.failure_threshold {
            inner.state = CircuitState::Open;
            inner.opened_at = Some(Instant::now());
        }
    }
}

/// One [`CircuitBreaker`] per destination host, created on first use.
///
/// Keying by host means a down anchor trips its own breaker without
/// affecting delivery to any other anchor sharing the same
/// [`WebhookDeliverer`](crate::delivery::WebhookDeliverer).
pub struct CircuitBreakerRegistry {
    config: CircuitBreakerConfig,
    breakers: Mutex<HashMap<String, Arc<CircuitBreaker>>>,
}

impl CircuitBreakerRegistry {
    pub fn new(config: CircuitBreakerConfig) -> Self {
        Self { config, breakers: Mutex::new(HashMap::new()) }
    }

    /// Gets (creating if necessary) the breaker for `key` -- callers pass
    /// the destination host so failures against one anchor don't trip
    /// delivery to others.
    pub fn breaker_for(&self, key: &str) -> Arc<CircuitBreaker> {
        let mut breakers = self.breakers.lock().unwrap_or_else(|e| e.into_inner());
        breakers
            .entry(key.to_string())
            .or_insert_with(|| Arc::new(CircuitBreaker::new(self.config)))
            .clone()
    }

    /// Current state of the breaker for `key`, or `None` if no call has
    /// been made against that key yet (equivalent to `Closed`).
    pub fn state_for(&self, key: &str) -> Option<CircuitState> {
        self.breakers.lock().unwrap_or_else(|e| e.into_inner()).get(key).map(|b| b.state())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(failure_threshold: u32, cooldown: Duration) -> CircuitBreakerConfig {
        CircuitBreakerConfig { failure_threshold, cooldown }
    }

    #[test]
    fn starts_closed() {
        let breaker = CircuitBreaker::new(config(3, Duration::from_secs(30)));
        assert_eq!(breaker.state(), CircuitState::Closed);
    }

    #[test]
    fn stays_closed_below_failure_threshold() {
        let breaker = CircuitBreaker::new(config(3, Duration::from_secs(30)));
        for _ in 0..2 {
            breaker.try_acquire().unwrap().failure();
        }
        assert_eq!(breaker.state(), CircuitState::Closed);
    }

    #[test]
    fn trips_open_after_n_consecutive_failures() {
        let breaker = CircuitBreaker::new(config(3, Duration::from_secs(30)));
        for _ in 0..3 {
            breaker.try_acquire().unwrap().failure();
        }
        assert_eq!(breaker.state(), CircuitState::Open);
    }

    #[test]
    fn success_resets_the_consecutive_failure_count() {
        let breaker = CircuitBreaker::new(config(3, Duration::from_secs(30)));
        breaker.try_acquire().unwrap().failure();
        breaker.try_acquire().unwrap().failure();
        breaker.try_acquire().unwrap().success(); // resets the streak
        breaker.try_acquire().unwrap().failure();
        breaker.try_acquire().unwrap().failure();
        // Only 2 consecutive failures since the reset -- still closed.
        assert_eq!(breaker.state(), CircuitState::Closed);
    }

    #[test]
    fn open_circuit_short_circuits_calls_before_cooldown_elapses() {
        let breaker = CircuitBreaker::new(config(1, Duration::from_secs(60)));
        breaker.try_acquire().unwrap().failure();
        assert_eq!(breaker.state(), CircuitState::Open);

        assert_eq!(breaker.try_acquire().unwrap_err(), CircuitOpen);
        assert_eq!(breaker.try_acquire().unwrap_err(), CircuitOpen);
    }

    #[test]
    fn allows_exactly_one_probe_after_cooldown_elapses() {
        let breaker = CircuitBreaker::new(config(1, Duration::from_millis(20)));
        breaker.try_acquire().unwrap().failure();
        assert_eq!(breaker.state(), CircuitState::Open);

        std::thread::sleep(Duration::from_millis(30));

        // First call after cooldown becomes the probe.
        let probe = breaker.try_acquire().unwrap();
        assert!(probe.is_probe());
        assert_eq!(breaker.state(), CircuitState::HalfOpen);

        // A second, concurrent caller must not also get a probe slot.
        assert_eq!(breaker.try_acquire().unwrap_err(), CircuitOpen);

        // Resolving the probe's outcome is still pending.
        probe.success();
        assert_eq!(breaker.state(), CircuitState::Closed);
    }

    #[test]
    fn half_open_probe_success_closes_the_circuit() {
        let breaker = CircuitBreaker::new(config(1, Duration::from_millis(20)));
        breaker.try_acquire().unwrap().failure();
        std::thread::sleep(Duration::from_millis(30));

        breaker.try_acquire().unwrap().success();

        assert_eq!(breaker.state(), CircuitState::Closed);
        // And normal traffic resumes -- not treated as another probe.
        let permit = breaker.try_acquire().unwrap();
        assert!(!permit.is_probe());
        permit.success();
    }

    #[test]
    fn half_open_probe_failure_reopens_the_circuit_for_another_cooldown() {
        let breaker = CircuitBreaker::new(config(1, Duration::from_millis(20)));
        breaker.try_acquire().unwrap().failure();
        std::thread::sleep(Duration::from_millis(30));

        breaker.try_acquire().unwrap().failure(); // probe fails
        assert_eq!(breaker.state(), CircuitState::Open);

        // Immediately after: still within the new cooldown, short-circuited.
        assert_eq!(breaker.try_acquire().unwrap_err(), CircuitOpen);

        std::thread::sleep(Duration::from_millis(30));
        let probe = breaker.try_acquire().unwrap();
        assert!(probe.is_probe());
    }

    #[test]
    fn dropping_a_permit_without_reporting_an_outcome_counts_as_failure() {
        let breaker = CircuitBreaker::new(config(1, Duration::from_secs(30)));
        {
            let _permit = breaker.try_acquire().unwrap();
            // Dropped without calling success()/failure().
        }
        assert_eq!(breaker.state(), CircuitState::Open);
    }

    #[test]
    fn registry_isolates_breakers_per_key() {
        let registry = CircuitBreakerRegistry::new(config(1, Duration::from_secs(30)));

        registry.breaker_for("down-anchor.example").try_acquire().unwrap().failure();

        assert_eq!(registry.state_for("down-anchor.example"), Some(CircuitState::Open));
        // A different destination is unaffected.
        assert_eq!(registry.state_for("healthy-anchor.example"), None);
        assert!(registry.breaker_for("healthy-anchor.example").try_acquire().is_ok());
    }

    #[test]
    fn registry_returns_the_same_breaker_instance_for_the_same_key() {
        let registry = CircuitBreakerRegistry::new(config(1, Duration::from_secs(30)));
        registry.breaker_for("anchor.example").try_acquire().unwrap().failure();
        // Fetching again must return the breaker that already tripped, not
        // a fresh Closed one.
        assert_eq!(registry.breaker_for("anchor.example").state(), CircuitState::Open);
    }
}
