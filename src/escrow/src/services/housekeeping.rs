use core::{cell::Cell, time::Duration};

use ic_cdk::{api::time, futures::spawn};
use ic_cdk_timers::set_timer_interval;

use crate::services::{disputes, expiry};

/// How often the automatic expiry sweep runs (5 minutes).
const SWEEP_INTERVAL: Duration = Duration::from_mins(5);

/// Maximum number of expired deals to process per sweep cycle.
const SWEEP_BATCH_LIMIT: u32 = 50;

/// How often the dispute auto-finalize sweep runs (RFC-001 step 8).
/// Same cadence as the expiry sweep — both operate on time-driven
/// deadlines that don't need sub-minute granularity.
const DISPUTE_SWEEP_INTERVAL: Duration = Duration::from_mins(5);

/// Maximum number of disputes to auto-finalize per sweep cycle.
const DISPUTE_SWEEP_BATCH_LIMIT: u32 = 20;

thread_local! {
    /// Prevents overlapping async sweeps — at most one in-flight at a time.
    static SWEEP_RUNNING: Cell<bool> = const { Cell::new(false) };
    /// Same re-entrancy guard, but for the dispute auto-finalize sweep
    /// (RFC-001 step 8). Independent from `SWEEP_RUNNING` so the two
    /// sweeps can interleave (they touch disjoint state — expiry only
    /// looks at `Funded` deals; dispute sweep only at `Disputed`).
    static DISPUTE_SWEEP_RUNNING: Cell<bool> = const { Cell::new(false) };
}

/// Starts a repeating timer that automatically refunds expired funded deals.
///
/// Called once from `#[init]` and `#[post_upgrade]`. The timer fires every
/// [`SWEEP_INTERVAL`]; each cycle processes up to [`SWEEP_BATCH_LIMIT`] deals.
/// A re-entrancy guard ensures at most one sweep is in-flight at a time.
pub fn start_expiry_sweep() {
    set_timer_interval(SWEEP_INTERVAL, || async {
        if !try_start_sweep() {
            return;
        }
        spawn(async {
            let _guard = SweepGuard;
            let _ = expiry::process_expired(SWEEP_BATCH_LIMIT).await;
        });
    });
}

/// Starts the dispute auto-finalize sweep (RFC-001 step 8).
///
/// Called once from `#[init]` and `#[post_upgrade]`. The timer fires
/// every [`DISPUTE_SWEEP_INTERVAL`]; each cycle calls
/// [`disputes::auto_finalize_due`] with a per-cycle cap of
/// [`DISPUTE_SWEEP_BATCH_LIMIT`]. The re-entrancy guard
/// [`DISPUTE_SWEEP_RUNNING`] ensures at most one dispute sweep is
/// in-flight at a time. Per-dispute errors inside `auto_finalize_due`
/// are swallowed — they get retried on the next cycle.
pub fn start_dispute_sweep() {
    set_timer_interval(DISPUTE_SWEEP_INTERVAL, || async {
        if !try_start_dispute_sweep() {
            return;
        }
        spawn(async {
            let _guard = DisputeSweepGuard;
            let _ = disputes::auto_finalize_due(DISPUTE_SWEEP_BATCH_LIMIT, time()).await;
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

fn try_start_dispute_sweep() -> bool {
    DISPUTE_SWEEP_RUNNING.with(|r| {
        if r.get() {
            false
        } else {
            r.set(true);
            true
        }
    })
}

struct DisputeSweepGuard;

impl Drop for DisputeSweepGuard {
    fn drop(&mut self) {
        DISPUTE_SWEEP_RUNNING.with(|r| r.set(false));
    }
}

#[cfg(test)]
mod tests {
    use super::{
        try_start_dispute_sweep, try_start_sweep, DisputeSweepGuard, SweepGuard,
        DISPUTE_SWEEP_RUNNING, SWEEP_RUNNING,
    };

    fn reset() {
        SWEEP_RUNNING.with(|r| r.set(false));
        DISPUTE_SWEEP_RUNNING.with(|r| r.set(false));
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

    // --- Dispute sweep guard (RFC-001 step 8) ---

    #[test]
    fn dispute_sweep_blocks_concurrent() {
        reset();
        assert!(try_start_dispute_sweep());
        assert!(!try_start_dispute_sweep());
        reset();
    }

    #[test]
    fn dispute_sweep_resets_on_guard_drop() {
        reset();
        assert!(try_start_dispute_sweep());
        drop(DisputeSweepGuard);
        assert!(try_start_dispute_sweep());
        reset();
    }

    #[test]
    fn dispute_sweep_independent_from_expiry_sweep() {
        // The two sweeps use disjoint flags — taking one should not block
        // the other.
        reset();
        assert!(try_start_sweep());
        assert!(try_start_dispute_sweep());
        reset();
    }
}
