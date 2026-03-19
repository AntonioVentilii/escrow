use core::{cell::Cell, time::Duration};

use ic_cdk::spawn;
use ic_cdk_timers::set_timer_interval;

use crate::services::expiry;

/// How often the automatic expiry sweep runs (5 minutes).
const SWEEP_INTERVAL: Duration = Duration::from_secs(5 * 60);

/// Maximum number of expired deals to process per sweep cycle.
const SWEEP_BATCH_LIMIT: u32 = 50;

thread_local! {
    /// Prevents overlapping async sweeps — at most one in-flight at a time.
    static SWEEP_RUNNING: Cell<bool> = const { Cell::new(false) };
}

/// Starts a repeating timer that automatically refunds expired funded deals.
///
/// Called once from `#[init]` and `#[post_upgrade]`. The timer fires every
/// [`SWEEP_INTERVAL`]; each cycle processes up to [`SWEEP_BATCH_LIMIT`] deals.
/// A re-entrancy guard ensures at most one sweep is in-flight at a time.
pub fn start_expiry_sweep() {
    set_timer_interval(SWEEP_INTERVAL, || {
        if !try_start_sweep() {
            return;
        }
        spawn(async {
            let _guard = SweepGuard;
            let _ = expiry::process_expired(SWEEP_BATCH_LIMIT).await;
        });
    });
}

/// Attempts to mark a sweep as in-progress.
///
/// Returns `true` if the sweep was started, `false` if one is already running.
fn try_start_sweep() -> bool {
    SWEEP_RUNNING.with(|r| {
        if r.get() {
            false
        } else {
            r.set(true);
            true
        }
    })
}

/// RAII guard that resets the [`SWEEP_RUNNING`] flag when dropped,
/// ensuring the flag is cleared even if the sweep future is cancelled.
struct SweepGuard;

impl Drop for SweepGuard {
    fn drop(&mut self) {
        SWEEP_RUNNING.with(|r| r.set(false));
    }
}

#[cfg(test)]
mod tests {
    use super::{try_start_sweep, SweepGuard, SWEEP_RUNNING};

    fn reset() {
        SWEEP_RUNNING.with(|r| r.set(false));
    }

    #[test]
    fn blocks_concurrent_sweep() {
        reset();
        assert!(try_start_sweep(), "first attempt should succeed");
        assert!(!try_start_sweep(), "concurrent attempt should be blocked");
        reset();
    }

    #[test]
    fn resets_on_guard_drop() {
        reset();
        assert!(try_start_sweep());
        drop(SweepGuard);
        assert!(try_start_sweep(), "should succeed after guard is dropped");
        reset();
    }

    #[test]
    fn allows_sequential_sweeps() {
        reset();
        for _ in 0..5 {
            assert!(try_start_sweep());
            drop(SweepGuard);
        }
        reset();
    }

    #[test]
    fn guard_scoped_lifetime() {
        reset();
        assert!(try_start_sweep());
        {
            let _guard = SweepGuard;
            assert!(!try_start_sweep(), "should be blocked while guard is alive");
        }
        assert!(try_start_sweep(), "should succeed after scope ends");
        reset();
    }
}
