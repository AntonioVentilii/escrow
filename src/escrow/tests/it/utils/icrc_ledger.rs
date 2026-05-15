//! ICRC-1 / ICRC-2 ledger fixture for pocket-ic integration tests.
//!
//! Downloads the official ICRC ledger suite wasm from a pinned
//! release tag on `github.com/dfinity/ic` (verified SHA-256), caches
//! it under `target/.icrc1-ledger-cache/`, and exposes a small
//! builder + helper surface for installing the ledger in a
//! `PocketIc` instance and driving its update / query methods from
//! tests.
//!
//! Types are defined inline rather than depending on the external
//! `icrc-ledger-types` crate so the test fixture doesn't pull a new
//! workspace dependency. We only encode the subset of the ICRC
//! ledger init / interaction surface we actually exercise; expand
//! when a new flow needs it.

use std::{
    fs,
    path::PathBuf,
    process::{self, Command, Stdio},
    sync::{Arc, LazyLock},
};

use candid::{
    decode_one, encode_args, encode_one, utils::ArgumentEncoder, CandidType, Deserialize, Int, Nat,
    Principal,
};
use pocket_ic::PocketIc;

use super::pic_canister::PicCanisterTrait;

// ---------------------------------------------------------------------------
// Wasm cache + download
// ---------------------------------------------------------------------------

/// Pinned release tag of the dfinity/ic ICRC ledger suite to install
/// in pocket-ic. Updating this is intentional — bump the constant
/// and `LEDGER_WASM_SHA256` together, verify integration tests still
/// pass, and treat it as a routine dependency bump.
const LEDGER_RELEASE_TAG: &str = "ledger-suite-icrc-2025-10-27";

/// Verified SHA-256 of the compressed wasm at the pinned release
/// tag. Cross-checked against the `SHA256SUMS` published with the
/// release. The download path verifies against this before
/// uncompressing, so a corrupt or tampered download aborts the
/// test run loudly.
const LEDGER_WASM_SHA256: &str = "71c27c5dc10034a1175296892b37827df0265d0ae072f5c59e99b8a1f6c45c76";

/// Download URL constructed from the pinned release tag.
fn ledger_wasm_url() -> String {
    format!(
        "https://github.com/dfinity/ic/releases/download/{LEDGER_RELEASE_TAG}/ic-icrc1-ledger.wasm.gz",
    )
}

/// On-disk cache directory for the downloaded + decompressed wasm.
/// Lives under the cargo `target/` so it's git-ignored and gets
/// blown away by `cargo clean`. Per release tag so bumping the
/// constant doesn't reuse a stale binary.
fn cache_dir() -> PathBuf {
    let target_dir = super::pic_canister::PicCanister::workspace_dir().join("target");
    target_dir
        .join(".icrc1-ledger-cache")
        .join(LEDGER_RELEASE_TAG)
}

/// Returns the path to a usable decompressed ICRC-1 ledger wasm.
/// Downloads + verifies + decompresses on first call; reuses the
/// cached file on subsequent calls. Idempotent across test threads
/// — initialised exactly once per test process via a `LazyLock`.
///
/// # Panics
///
/// Tests fail hard rather than degrade — any failure to obtain a
/// valid ledger wasm panics with a descriptive message. The integration
/// suite cannot meaningfully run without a real ledger and silently
/// skipping would let regressions slip through CI.
pub fn icrc1_ledger_wasm_path() -> PathBuf {
    LEDGER_WASM_PATH.clone()
}

static LEDGER_WASM_PATH: LazyLock<PathBuf> = LazyLock::new(prepare_ledger_wasm);

fn prepare_ledger_wasm() -> PathBuf {
    let dir = cache_dir();
    fs::create_dir_all(&dir).expect("create ledger cache dir");

    let wasm_path = dir.join("ic-icrc1-ledger.wasm");
    if wasm_path.exists() {
        return wasm_path;
    }

    let gz_path = dir.join("ic-icrc1-ledger.wasm.gz");
    let url = ledger_wasm_url();

    // Download to a per-process temp path first, then atomically
    // rename into the cache location. Without the rename hop,
    // concurrent test binaries (or a re-run that interrupted a
    // previous download) can observe a partial `.wasm.gz` and
    // crash here with a SHA mismatch / decompression error.
    let tmp_gz = dir.join(format!("ic-icrc1-ledger.wasm.gz.{}.tmp", process::id()));
    let curl = Command::new("curl")
        .args([
            "--silent",
            "--show-error",
            "--fail",
            "--location",
            "--output",
        ])
        .arg(&tmp_gz)
        .arg(&url)
        .status()
        .expect("invoke curl");
    assert!(
        curl.success(),
        "failed to download ICRC-1 ledger wasm from {url}",
    );

    let actual_sha = sha256_of(&tmp_gz);
    assert_eq!(
        actual_sha, LEDGER_WASM_SHA256,
        "ICRC-1 ledger wasm SHA-256 mismatch: expected {LEDGER_WASM_SHA256}, got {actual_sha}. \
         Either the upstream release was rewritten (unlikely) or the download was tampered with.",
    );

    fs::rename(&tmp_gz, &gz_path).expect("rename downloaded wasm into cache");

    // Decompress with `gunzip -c` and stream stdout into the
    // destination file. Avoids the `--keep`/`--force` semantics
    // dance, never leaves a partial output, and works the same
    // on GNU and BusyBox gunzip variants.
    let tmp_wasm = dir.join(format!("ic-icrc1-ledger.wasm.{}.tmp", process::id()));
    let tmp_file = fs::File::create(&tmp_wasm).expect("create temp wasm file");
    let gunzip = Command::new("gunzip")
        .arg("-c")
        .arg(&gz_path)
        .stdout(Stdio::from(tmp_file))
        .status()
        .expect("invoke gunzip");
    assert!(
        gunzip.success(),
        "failed to decompress {}",
        gz_path.display(),
    );
    fs::rename(&tmp_wasm, &wasm_path).expect("rename decompressed wasm into cache");

    assert!(
        wasm_path.exists(),
        "decompressed wasm not present at {}",
        wasm_path.display(),
    );
    fs::remove_file(&gz_path).ok();
    wasm_path
}

fn sha256_of(path: &PathBuf) -> String {
    let output = Command::new("shasum")
        .args(["-a", "256"])
        .arg(path)
        .stdout(Stdio::piped())
        .output()
        .expect("invoke shasum");
    assert!(
        output.status.success(),
        "shasum failed on {}",
        path.display(),
    );
    let stdout = String::from_utf8(output.stdout).expect("shasum output utf8");
    stdout
        .split_whitespace()
        .next()
        .expect("shasum output empty")
        .to_owned()
}

// ---------------------------------------------------------------------------
// ICRC-1 / ICRC-2 candid types
// ---------------------------------------------------------------------------

#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct Account {
    pub owner: Principal,
    pub subaccount: Option<Vec<u8>>,
}

impl Account {
    pub fn for_owner(owner: Principal) -> Self {
        Self {
            owner,
            subaccount: None,
        }
    }
}

#[derive(CandidType, Clone, Debug)]
struct ArchiveOptions {
    num_blocks_to_archive: u64,
    max_transactions_per_response: Option<u64>,
    trigger_threshold: u64,
    max_message_size_bytes: Option<u64>,
    cycles_for_archive_creation: Option<u64>,
    node_max_memory_size_bytes: Option<u64>,
    controller_id: Principal,
    more_controllers: Option<Vec<Principal>>,
}

#[derive(CandidType, Clone, Debug)]
#[expect(dead_code)] // Variants are part of the ledger candid surface;
                     // tests use an empty metadata vec.
enum MetadataValue {
    Nat(Nat),
    Int(Int),
    Text(String),
    Blob(Vec<u8>),
}

#[derive(CandidType, Clone, Debug)]
struct FeatureFlags {
    icrc2: bool,
}

#[derive(CandidType, Clone, Debug)]
struct InitArgs {
    decimals: Option<u8>,
    token_symbol: String,
    transfer_fee: Nat,
    metadata: Vec<(String, MetadataValue)>,
    minting_account: Account,
    initial_balances: Vec<(Account, Nat)>,
    maximum_number_of_accounts: Option<u64>,
    accounts_overflow_trim_quantity: Option<u64>,
    fee_collector_account: Option<Account>,
    archive_options: ArchiveOptions,
    max_memo_length: Option<u16>,
    token_name: String,
    feature_flags: Option<FeatureFlags>,
}

#[derive(CandidType, Clone, Debug)]
enum LedgerArg {
    // Box the large `InitArgs` (~400 B) so the variants stay
    // roughly balanced in size — pedantic clippy flags otherwise.
    Init(Box<InitArgs>),
    #[expect(dead_code)] // Reserved for future upgrade-path tests.
    Upgrade(Option<()>),
}

#[derive(CandidType, Clone, Debug)]
pub struct ApproveArgs {
    pub from_subaccount: Option<Vec<u8>>,
    pub spender: Account,
    pub amount: Nat,
    pub expected_allowance: Option<Nat>,
    pub expires_at: Option<u64>,
    pub fee: Option<Nat>,
    pub memo: Option<Vec<u8>>,
    pub created_at_time: Option<u64>,
}

#[derive(CandidType, Deserialize, Clone, Debug)]
pub enum ApproveError {
    BadFee { expected_fee: Nat },
    InsufficientFunds { balance: Nat },
    AllowanceChanged { current_allowance: Nat },
    Expired { ledger_time: u64 },
    TooOld,
    CreatedInFuture { ledger_time: u64 },
    Duplicate { duplicate_of: Nat },
    TemporarilyUnavailable,
    GenericError { error_code: Nat, message: String },
}

// ---------------------------------------------------------------------------
// Fixture builder + canister wrapper
// ---------------------------------------------------------------------------

/// A test fixture wrapping a deployed ICRC-1 / ICRC-2 ledger canister.
///
/// Construct via [`IcrcLedgerBuilder`]. Holds an `Arc<PocketIc>` so it
/// can be cloned cheaply into other helpers; the underlying ledger
/// state is shared.
pub struct IcrcLedger {
    pub pic: Arc<PocketIc>,
    pub canister_id: Principal,
    pub minter: Principal,
    pub fee: u128,
}

impl IcrcLedger {
    /// The ledger's `Principal` — the canister id callers pass to
    /// `create_deal({ asset: Asset::Icrc(ledger.principal()) })`.
    pub fn principal(&self) -> Principal {
        self.canister_id
    }

    /// Approves `spender` (typically the escrow canister) to pull up
    /// to `amount` from `from_owner`'s default subaccount.
    pub fn approve(&self, from_owner: Principal, spender: Principal, amount: u128) {
        let arg = ApproveArgs {
            from_subaccount: None,
            spender: Account::for_owner(spender),
            amount: Nat::from(amount),
            expected_allowance: None,
            expires_at: None,
            fee: None,
            memo: None,
            created_at_time: None,
        };
        let result: Result<Nat, ApproveError> = self
            .update_call(from_owner, "icrc2_approve", (arg,))
            .expect("ledger approve call");
        result.expect("ledger approve succeeded");
    }

    /// Reads `icrc1_balance_of(account)` and returns the value as
    /// `u128` (panics on overflow — not expected for test amounts).
    pub fn balance_of(&self, account: Account) -> u128 {
        let nat: Nat = self
            .query_call(self.minter, "icrc1_balance_of", (account,))
            .expect("ledger balance_of call");
        nat_to_u128(&nat)
    }

    /// Convenience: balance of an owner's default subaccount.
    pub fn balance_of_owner(&self, owner: Principal) -> u128 {
        self.balance_of(Account::for_owner(owner))
    }

    /// Convenience: balance of a canister's specific subaccount.
    pub fn balance_of_subaccount(&self, owner: Principal, subaccount: Vec<u8>) -> u128 {
        self.balance_of(Account {
            owner,
            subaccount: Some(subaccount),
        })
    }

    fn update_call<T, A>(&self, caller: Principal, method: &str, arg: A) -> Result<T, String>
    where
        T: for<'a> Deserialize<'a> + CandidType,
        A: ArgumentEncoder,
    {
        let bytes = self
            .pic
            .update_call(self.canister_id, caller, method, encode_args(arg).unwrap())
            .map_err(|e| format!("ledger update call error: {e:?}"))?;
        decode_one(&bytes).map_err(|e| format!("ledger decode error: {e:?}"))
    }

    fn query_call<T, A>(&self, caller: Principal, method: &str, arg: A) -> Result<T, String>
    where
        T: for<'a> Deserialize<'a> + CandidType,
        A: ArgumentEncoder,
    {
        let bytes = self
            .pic
            .query_call(self.canister_id, caller, method, encode_args(arg).unwrap())
            .map_err(|e| format!("ledger query call error: {e:?}"))?;
        decode_one(&bytes).map_err(|e| format!("ledger decode error: {e:?}"))
    }
}

/// Builder for installing an ICRC-1 / ICRC-2 ledger in a pocket-ic
/// instance. Defaults:
///   - Symbol = `TEST`, name = `Test Token`, decimals = 8
///   - Transfer fee = `10_000` (matches ICP's per-transfer fee)
///   - ICRC-2 enabled
///   - Minter = `Principal::from_slice(&[10, 0, 0])`
pub struct IcrcLedgerBuilder {
    transfer_fee: u128,
    symbol: String,
    name: String,
    decimals: u8,
    minter: Principal,
    initial_balances: Vec<(Principal, u128)>,
    controllers: Vec<Principal>,
}

impl IcrcLedgerBuilder {
    pub fn new() -> Self {
        Self {
            transfer_fee: 10_000,
            symbol: "TEST".to_owned(),
            name: "Test Token".to_owned(),
            decimals: 8,
            minter: Principal::from_slice(&[10, 0, 0]),
            initial_balances: Vec::new(),
            controllers: vec![Principal::from_slice(&[10, 0, 1])],
        }
    }

    /// Pre-funds `owner`'s default subaccount with `amount` tokens
    /// at canister-install time. The ICRC-1 ledger applies these
    /// balances atomically during init — no `mint` calls needed.
    pub fn with_initial_balance(mut self, owner: Principal, amount: u128) -> Self {
        self.initial_balances.push((owner, amount));
        self
    }

    pub fn deploy_to(self, pic: &Arc<PocketIc>) -> IcrcLedger {
        let canister_id = pic.create_canister();
        pic.add_cycles(canister_id, 2_000_000_000_000);

        let init = InitArgs {
            decimals: Some(self.decimals),
            token_symbol: self.symbol.clone(),
            transfer_fee: Nat::from(self.transfer_fee),
            metadata: vec![],
            minting_account: Account::for_owner(self.minter),
            initial_balances: self
                .initial_balances
                .iter()
                .map(|(owner, amount)| (Account::for_owner(*owner), Nat::from(*amount)))
                .collect(),
            maximum_number_of_accounts: None,
            accounts_overflow_trim_quantity: None,
            fee_collector_account: None,
            archive_options: ArchiveOptions {
                num_blocks_to_archive: 1_000,
                max_transactions_per_response: None,
                trigger_threshold: 10_000,
                max_message_size_bytes: None,
                cycles_for_archive_creation: Some(1_000_000_000_000),
                node_max_memory_size_bytes: None,
                controller_id: self.controllers[0],
                more_controllers: None,
            },
            max_memo_length: None,
            token_name: self.name.clone(),
            feature_flags: Some(FeatureFlags { icrc2: true }),
        };

        let wasm_path = icrc1_ledger_wasm_path();
        let wasm_bytes = fs::read(&wasm_path)
            .unwrap_or_else(|e| panic!("read icrc1 ledger wasm at {}: {e}", wasm_path.display()));
        let arg = encode_one(LedgerArg::Init(Box::new(init))).expect("encode ledger init arg");

        pic.install_canister(canister_id, wasm_bytes, arg, None);
        pic.set_controllers(canister_id, None, self.controllers.clone())
            .expect("set ledger controllers");

        IcrcLedger {
            pic: pic.clone(),
            canister_id,
            minter: self.minter,
            fee: self.transfer_fee,
        }
    }
}

impl Default for IcrcLedgerBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Small utilities
// ---------------------------------------------------------------------------

fn nat_to_u128(nat: &Nat) -> u128 {
    nat.0
        .to_string()
        .parse()
        .expect("Nat fits in u128 in tests")
}
