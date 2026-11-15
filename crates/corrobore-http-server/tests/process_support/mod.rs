// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

//! Shared polling for tests that spawn the real `corrobore` binary (#365).
//!
//! The first launch of a freshly built binary can take seconds on a loaded
//! machine (macOS scans new executables before they run), so waits end at a
//! generous deadline rather than after a fixed number of attempts. A fast run
//! returns at the first successful poll.
#![allow(dead_code)]

use std::{
    env, thread,
    time::{Duration, Instant},
};

/// Delay between two polls.
pub const POLL: Duration = Duration::from_millis(40);

/// Default ceiling for one wait.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// The wait ceiling, overridable with `CORROBORE_TEST_PROCESS_TIMEOUT_MS`.
pub fn process_timeout() -> Duration {
    env::var("CORROBORE_TEST_PROCESS_TIMEOUT_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|millis| *millis > 0)
        .map_or(DEFAULT_TIMEOUT, Duration::from_millis)
}

/// Call `step` until it returns a value or the ceiling passes.
pub fn poll_until<T>(mut step: impl FnMut() -> Option<T>) -> Option<T> {
    let deadline = Instant::now() + process_timeout();
    loop {
        if let Some(value) = step() {
            return Some(value);
        }
        if Instant::now() >= deadline {
            return None;
        }
        thread::sleep(POLL);
    }
}
