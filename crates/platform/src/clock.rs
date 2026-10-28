//! The clock behind every limit PV gives up after.
//!
//! In production each function is the Tokio or standard-library call it wraps. With the
//! `test-support` feature, a test can install a fake clock whose time moves only when the test
//! advances it past a limit PV is waiting on, so a test of a frozen process never waits out the real
//! limit.

use std::fmt;
use std::future::{self, Future};
use std::thread;
use std::time::{Duration, Instant};

#[cfg(feature = "test-support")]
pub use fake::{FakeClock, LimitNotPending, install_fake_for_test};

/// A named limit PV gives up after.
///
/// The name lets a test advance past exactly this limit while other limits are pending.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Limit {
    name: &'static str,
    duration: Duration,
}

impl Limit {
    pub const fn new(name: &'static str, duration: Duration) -> Self {
        Self { name, duration }
    }

    pub const fn name(&self) -> &'static str {
        self.name
    }

    pub const fn duration(&self) -> Duration {
        self.duration
    }
}

/// The limit passed before the operation finished.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Elapsed;

impl fmt::Display for Elapsed {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("deadline has elapsed")
    }
}

/// Returns the current time, or the fake clock's time when a test installed one.
pub fn now() -> Instant {
    #[cfg(feature = "test-support")]
    if let Some(fake) = fake::installed() {
        return fake.now();
    }

    Instant::now()
}

/// Runs `future` until it finishes or `limit` passes.
pub async fn timeout<F: Future>(limit: &Limit, future: F) -> Result<F::Output, Elapsed> {
    #[cfg(feature = "test-support")]
    if let Some(fake) = fake::installed() {
        return fake.timeout(limit, None, future).await;
    }

    tokio::time::timeout(limit.duration, future)
        .await
        .map_err(|_elapsed| Elapsed)
}

/// Sleeps until `limit` passes, for schedules such as the health scan interval.
pub async fn sleep(limit: &Limit) {
    // A timeout around a future that never finishes ends exactly when the limit passes.
    let _elapsed = timeout(limit, future::pending::<()>()).await;
}

/// The synchronous [`sleep`], for schedules in code that runs outside an async runtime.
pub fn sleep_blocking(limit: &Limit) {
    #[cfg(feature = "test-support")]
    if let Some(fake) = fake::installed() {
        fake.sleep_blocking(limit);
        return;
    }

    thread::sleep(limit.duration);
}

/// Sleeps for a short real interval between checks of a real process.
///
/// Limits go through [`timeout`] or [`Deadline`]. This pause stays real under a fake clock so PV
/// keeps watching real processes at real speed.
pub async fn poll(interval: Duration) {
    tokio::time::sleep(interval).await;
}

/// The synchronous [`poll`], for polling loops that run outside an async runtime.
pub fn poll_blocking(interval: Duration) {
    thread::sleep(interval);
}

/// The moment a limit passes, for loops that poll until something happens or the limit passes.
#[derive(Debug)]
pub struct Deadline {
    at: Instant,
    #[cfg(feature = "test-support")]
    _registration: Option<fake::Registration>,
}

impl Deadline {
    pub fn after(limit: &Limit) -> Self {
        #[cfg(feature = "test-support")]
        if let Some(fake) = fake::installed() {
            let (at, registration) = fake.deadline(limit);
            return Self {
                at,
                _registration: Some(registration),
            };
        }

        Self {
            at: Instant::now() + limit.duration,
            #[cfg(feature = "test-support")]
            _registration: None,
        }
    }

    pub fn is_expired(&self) -> bool {
        now() >= self.at
    }

    /// Runs `future` until it finishes, `limit` passes, or this deadline passes, whichever is
    /// first.
    pub async fn timeout<F: Future>(&self, limit: &Limit, future: F) -> Result<F::Output, Elapsed> {
        #[cfg(feature = "test-support")]
        if let Some(fake) = fake::installed() {
            return fake.timeout(limit, Some(self.at), future).await;
        }

        let remaining = self.at.saturating_duration_since(Instant::now());
        tokio::time::timeout(remaining.min(limit.duration), future)
            .await
            .map_err(|_elapsed| Elapsed)
    }
}

#[cfg(feature = "test-support")]
mod fake {
    use std::collections::BTreeMap;
    use std::future::Future;
    use std::io::{self, Write};
    use std::pin::Pin;
    use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
    use std::task::{Context, Poll, Waker};
    use std::time::{Duration, Instant};
    use std::{process, thread};

    use thiserror::Error;

    use super::{Elapsed, Limit};

    /// How long, in real time, [`FakeClock::advance_when_waiting`] waits for PV to start waiting.
    const WAIT_FOR_LIMIT: Duration = Duration::from_secs(30);
    const WAIT_FOR_LIMIT_POLL: Duration = Duration::from_millis(5);
    /// The status a failing Rust test exits with.
    const TEST_FAILURE_STATUS: i32 = 101;

    static FAKE: OnceLock<FakeClock> = OnceLock::new();

    pub(super) fn installed() -> Option<&'static FakeClock> {
        FAKE.get()
    }

    /// Switches every later clock call in this process to a fake whose time moves only when the
    /// test advances it.
    ///
    /// The fake stays installed until the process exits, so it relies on nextest running each test
    /// in its own process. Calling this again returns the same fake.
    pub fn install_fake_for_test() -> FakeClock {
        FAKE.get_or_init(|| FakeClock {
            shared: Arc::new(Shared {
                start: Instant::now(),
                state: Mutex::new(State::default()),
            }),
        })
        .clone()
    }

    /// A clock whose time moves only through [`FakeClock::advance_when_waiting`].
    #[derive(Clone, Debug)]
    pub struct FakeClock {
        shared: Arc<Shared>,
    }

    #[derive(Debug)]
    struct Shared {
        start: Instant,
        state: Mutex<State>,
    }

    #[derive(Debug, Default)]
    struct State {
        elapsed: Duration,
        next_id: u64,
        pending: BTreeMap<u64, Pending>,
    }

    #[derive(Debug)]
    struct Pending {
        name: &'static str,
        due: Duration,
        waker: Option<Waker>,
    }

    /// PV didn't start waiting on the limit within the real-time guard.
    #[derive(Debug, Error)]
    #[error("PV never waited on the `{limit}` limit; pending limits: {pending:?}")]
    pub struct LimitNotPending {
        limit: &'static str,
        pending: Vec<&'static str>,
    }

    impl FakeClock {
        /// Waits until PV is waiting on `limit`, then moves time to the moment it passes.
        ///
        /// Only a pending limit with the same name counts, so other limits that are already
        /// pending can't satisfy the wait. Gives up after 30 seconds of real time and lists the
        /// pending limits.
        pub async fn advance_when_waiting(&self, limit: &Limit) -> Result<(), LimitNotPending> {
            self.advance_when_waiting_within(limit, WAIT_FOR_LIMIT)
                .await
        }

        /// Does what [`FakeClock::advance_when_waiting`] does, from a helper thread, for a wait the
        /// test can't cancel: synchronous code on the test's own thread, or a blocking thread,
        /// which Tokio waits for when the test's runtime shuts down.
        ///
        /// A stuck test can't receive an error, so if PV doesn't wait on `limit` within 30 seconds
        /// of real time, the helper writes the pending limits to stderr and exits the test
        /// process with a failure status. nextest runs each test in its own process, so that fails
        /// only this test.
        #[expect(
            clippy::disallowed_methods,
            reason = "the helper must run while the test's own thread is blocked inside PV"
        )]
        pub fn advance_when_waiting_in_background(&self, limit: &Limit) {
            let clock = self.clone();
            let limit = *limit;
            thread::spawn(move || {
                if let Err(error) = clock.advance_when_waiting_blocking(&limit) {
                    let _write_result = writeln!(io::stderr(), "{error}");
                    process::exit(TEST_FAILURE_STATUS);
                }
            });
        }

        /// Whether PV is waiting on `limit` right now, for a test that must act once PV waits,
        /// without moving time.
        pub fn is_waiting(&self, limit: &Limit) -> bool {
            self.pending_due(limit.name).is_some()
        }

        pub(super) async fn advance_when_waiting_within(
            &self,
            limit: &Limit,
            guard: Duration,
        ) -> Result<(), LimitNotPending> {
            let give_up_at = Instant::now() + guard;
            while !self.try_advance(limit) {
                if Instant::now() >= give_up_at {
                    return Err(self.not_pending(limit));
                }
                super::poll(WAIT_FOR_LIMIT_POLL).await;
            }

            Ok(())
        }

        fn advance_when_waiting_blocking(&self, limit: &Limit) -> Result<(), LimitNotPending> {
            let give_up_at = Instant::now() + WAIT_FOR_LIMIT;
            while !self.try_advance(limit) {
                if Instant::now() >= give_up_at {
                    return Err(self.not_pending(limit));
                }
                super::poll_blocking(WAIT_FOR_LIMIT_POLL);
            }

            Ok(())
        }

        fn try_advance(&self, limit: &Limit) -> bool {
            let Some(due) = self.pending_due(limit.name) else {
                return false;
            };
            self.advance_to(due);

            true
        }

        fn not_pending(&self, limit: &Limit) -> LimitNotPending {
            LimitNotPending {
                limit: limit.name,
                pending: self.pending_names(),
            }
        }

        pub(super) fn now(&self) -> Instant {
            self.shared.start + self.lock().elapsed
        }

        pub(super) async fn timeout<F: Future>(
            &self,
            limit: &Limit,
            deadline: Option<Instant>,
            future: F,
        ) -> Result<F::Output, Elapsed> {
            let expiry = Expiry {
                registration: self.register(limit, deadline),
            };
            tokio::pin!(future);
            tokio::select! {
                biased;
                output = &mut future => Ok(output),
                () = expiry => Err(Elapsed),
            }
        }

        pub(super) fn sleep_blocking(&self, limit: &Limit) {
            let registration = self.register(limit, None);
            while self.lock().elapsed < registration.due {
                super::poll_blocking(WAIT_FOR_LIMIT_POLL);
            }
        }

        pub(super) fn deadline(&self, limit: &Limit) -> (Instant, Registration) {
            let registration = self.register(limit, None);

            (self.shared.start + registration.due, registration)
        }

        /// Registers `limit`, ending no later than `deadline`. The cap is applied under the same
        /// lock as the advance, so a limit started just after its deadline passed ends at once.
        fn register(&self, limit: &Limit, deadline: Option<Instant>) -> Registration {
            let mut state = self.lock();
            let id = state.next_id;
            state.next_id += 1;
            let mut due = state.elapsed + limit.duration;
            if let Some(deadline) = deadline {
                due = due.min(deadline.saturating_duration_since(self.shared.start));
            }
            state.pending.insert(
                id,
                Pending {
                    name: limit.name,
                    due,
                    waker: None,
                },
            );

            Registration {
                clock: self.clone(),
                id,
                due,
            }
        }

        fn pending_due(&self, name: &str) -> Option<Duration> {
            let state = self.lock();
            state
                .pending
                .values()
                .filter(|pending| pending.name == name && pending.due > state.elapsed)
                .map(|pending| pending.due)
                .min()
        }

        fn pending_names(&self) -> Vec<&'static str> {
            let state = self.lock();
            state
                .pending
                .values()
                .filter(|pending| pending.due > state.elapsed)
                .map(|pending| pending.name)
                .collect()
        }

        fn advance_to(&self, due: Duration) {
            let wakers = {
                let mut state = self.lock();
                state.elapsed = state.elapsed.max(due);
                let elapsed = state.elapsed;
                state
                    .pending
                    .values_mut()
                    .filter(|pending| pending.due <= elapsed)
                    .filter_map(|pending| pending.waker.take())
                    .collect::<Vec<_>>()
            };
            for waker in wakers {
                waker.wake();
            }
        }

        fn lock(&self) -> MutexGuard<'_, State> {
            match self.shared.state.lock() {
                Ok(state) => state,
                Err(poisoned) => poisoned.into_inner(),
            }
        }
    }

    /// A pending limit, removed from the fake when dropped.
    #[derive(Debug)]
    pub(super) struct Registration {
        clock: FakeClock,
        id: u64,
        due: Duration,
    }

    impl Drop for Registration {
        fn drop(&mut self) {
            self.clock.lock().pending.remove(&self.id);
        }
    }

    struct Expiry {
        registration: Registration,
    }

    impl Future for Expiry {
        type Output = ();

        fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<()> {
            let registration = &self.registration;
            let mut state = registration.clock.lock();
            if state.elapsed >= registration.due {
                return Poll::Ready(());
            }
            if let Some(pending) = state.pending.get_mut(&registration.id) {
                pending.waker = Some(context.waker().clone());
            }

            Poll::Pending
        }
    }
}

#[cfg(test)]
mod tests {
    use std::future::pending;
    use std::time::Duration;

    use anyhow::{Result, ensure};

    use super::{
        Deadline, Elapsed, Limit, now, poll, poll_blocking, sleep, sleep_blocking, timeout,
    };

    const RESPONSE: Limit = Limit::new("response", Duration::from_secs(3));
    const WRITE: Limit = Limit::new("write", Duration::from_secs(3));
    const HEALTH: Limit = Limit::new("health", Duration::from_secs(30));
    const SETTLE: Duration = Duration::from_millis(50);

    #[tokio::test]
    async fn real_clock_times_out_after_the_limit() -> Result<()> {
        let short = Limit::new("short", Duration::from_millis(20));
        let deadline = Deadline::after(&short);

        ensure!(timeout(&short, pending::<()>()).await == Err(Elapsed));
        ensure!(deadline.is_expired());
        ensure!(timeout(&short, async { 7 }).await == Ok(7));
        ensure!(
            Deadline::after(&short)
                .timeout(&HEALTH, pending::<()>())
                .await
                == Err(Elapsed)
        );
        let deadline = Deadline::after(&short);
        sleep(&short).await;
        ensure!(deadline.is_expired());
        let deadline = Deadline::after(&short);
        sleep_blocking(&short);
        ensure!(deadline.is_expired());
        Ok(())
    }

    #[cfg(feature = "test-support")]
    #[tokio::test]
    async fn fake_sleep_ends_only_after_the_test_advances_it() -> Result<()> {
        let clock = super::install_fake_for_test();
        ensure!(!clock.is_waiting(&HEALTH));
        let sleeping = tokio::spawn(async { sleep(&HEALTH).await });

        poll(SETTLE).await;
        ensure!(!sleeping.is_finished());
        ensure!(clock.is_waiting(&HEALTH));

        clock.advance_when_waiting(&HEALTH).await?;

        sleeping.await?;
        ensure!(!clock.is_waiting(&HEALTH));
        Ok(())
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn background_advance_ends_a_blocking_sleep() -> Result<()> {
        let clock = super::install_fake_for_test();
        let started_at = now();
        clock.advance_when_waiting_in_background(&RESPONSE);

        sleep_blocking(&RESPONSE);

        ensure!(now() == started_at + RESPONSE.duration());
        Ok(())
    }

    #[cfg(feature = "test-support")]
    #[tokio::test]
    async fn fake_limit_started_after_its_deadline_passed_ends_at_once() -> Result<()> {
        let clock = super::install_fake_for_test();
        let deadline = Deadline::after(&HEALTH);
        clock.advance_when_waiting(&HEALTH).await?;

        ensure!(deadline.timeout(&RESPONSE, pending::<()>()).await == Err(Elapsed));
        Ok(())
    }

    #[cfg(feature = "test-support")]
    #[tokio::test]
    async fn fake_limit_passes_only_after_the_test_advances_it() -> Result<()> {
        let clock = super::install_fake_for_test();
        let started_at = now();
        let waiting = tokio::spawn(timeout(&RESPONSE, pending::<()>()));

        poll(SETTLE).await;
        ensure!(!waiting.is_finished());
        ensure!(now() == started_at);

        clock.advance_when_waiting(&RESPONSE).await?;

        ensure!(waiting.await? == Err(Elapsed));
        ensure!(now() == started_at + RESPONSE.duration());
        Ok(())
    }

    #[cfg(feature = "test-support")]
    #[tokio::test]
    async fn advance_waits_for_the_named_limit_while_others_are_pending() -> Result<()> {
        let clock = super::install_fake_for_test();
        let started_at = now();
        let health = tokio::spawn(timeout(&HEALTH, pending::<()>()));
        let write = tokio::spawn(timeout(&WRITE, pending::<()>()));
        let advance = tokio::spawn({
            let clock = clock.clone();
            async move { clock.advance_when_waiting(&RESPONSE).await }
        });

        poll(SETTLE).await;
        ensure!(!advance.is_finished());
        ensure!(now() == started_at);

        let response = tokio::spawn(timeout(&RESPONSE, pending::<()>()));
        advance.await??;

        ensure!(response.await? == Err(Elapsed));
        ensure!(write.await? == Err(Elapsed));
        poll(SETTLE).await;
        ensure!(!health.is_finished());
        health.abort();
        Ok(())
    }

    #[cfg(feature = "test-support")]
    #[tokio::test]
    async fn fake_deadline_expires_only_after_the_test_advances_it() -> Result<()> {
        let clock = super::install_fake_for_test();
        let deadline = Deadline::after(&RESPONSE);

        ensure!(!deadline.is_expired());
        clock.advance_when_waiting(&RESPONSE).await?;
        ensure!(deadline.is_expired());
        Ok(())
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn background_advance_expires_a_deadline_the_test_thread_polls() {
        let clock = super::install_fake_for_test();
        clock.advance_when_waiting_in_background(&RESPONSE);
        let deadline = Deadline::after(&RESPONSE);

        while !deadline.is_expired() {
            poll_blocking(Duration::from_millis(1));
        }
    }

    #[cfg(feature = "test-support")]
    #[tokio::test]
    async fn advance_lists_pending_limits_when_the_limit_never_appears() -> Result<()> {
        let clock = super::install_fake_for_test();
        let _health = Deadline::after(&HEALTH);

        let Err(error) = clock.advance_when_waiting_within(&RESPONSE, SETTLE).await else {
            anyhow::bail!("expected the response limit to be missing");
        };

        insta::assert_snapshot!(error.to_string(), @r#"PV never waited on the `response` limit; pending limits: ["health"]"#);
        Ok(())
    }
}
