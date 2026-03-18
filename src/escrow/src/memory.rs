use core::cell::RefCell;

use ic_cdk::{storage, trap};

use crate::{types::state::StableState, Config};

thread_local! {
    pub static CONFIG: RefCell<Config> = const { RefCell::new(Config { }) };
}

pub fn save_state() {
    let config: Config = CONFIG.with(|c: &RefCell<Config>| c.borrow().clone());

    let state = StableState { config };

    storage::stable_save((state,)).expect("Save failed");
}

pub fn restore_state() {
    let result: Result<(StableState,), String> = storage::stable_restore();

    let state = match result {
        Ok((s,)) => s,
        Err(e) => {
            trap(&format!("Failed to restore stable state: {e:?}"));
        }
    };

    let StableState { config } = state;

    CONFIG.with(|c| *c.borrow_mut() = config);
}
