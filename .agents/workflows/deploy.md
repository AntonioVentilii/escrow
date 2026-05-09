---
description: local + staging + production deployment workflow for the escrow canister
---

The escrow canister supports three deploy targets, all driven by
`dfx`. The `package.json` wraps each one as an npm script.

> [!IMPORTANT]
> `npm run deploy:staging` and `npm run deploy:prod` touch **real**
> canisters with **real** funds. **Never** run them without an
> explicit user prompt.

## Local

For iterating locally against a fresh dfx replica.

1. Start the local replica (in a separate terminal):

   ```bash
   dfx start --clean --background
   ```

2. Build + deploy:

   ```bash
   npm run deploy
   ```

   Wraps `dfx deploy --network local --upgrade-unchanged`.

3. (Optional) Run the integration suite, which spins up its own
   `pocket-ic` instance and is independent of the local replica:

   ```bash
   npm run test:integration
   ```

## Staging

`canister_ids.json` pins the staging canister id
(`umxj5-niaaa-aaaae-af2sq-cai`).

```bash
npm run deploy:staging
```

Wraps `dfx deploy --network staging --upgrade-unchanged`. Requires
controller credentials in your local dfx identity.

> [!IMPORTANT]
> Don't run this from an agent. Surface the request to the user.

## Production

```bash
npm run deploy:prod
```

Wraps `dfx deploy --ic --upgrade-unchanged`.

> [!IMPORTANT]
> Don't run this from an agent. Surface the request to the user.

## Cutting a release

```bash
npm run release
```

Wraps `scripts/release.sh`, which:

1. Verifies a clean working tree on `main`.
2. Bumps `package.json` version + `Cargo.toml` package version + lock
   files.
3. Tags the commit `v<NEW_VERSION>`.
4. Pushes the tag, which triggers
   [`.github/workflows/release.yml`](../../.github/workflows/release.yml).

The release workflow builds the canister WASM + did and publishes a
GitHub Release with the artifact attached.

## Coordinating with PandaMe

Whenever the canister releases a new version, the PandaMe frontend
should pick up the new `escrow.did`:

1. Open `../pandame/`.
2. Run `npm run did` — fetches the latest `escrow.did` from upstream
   `main`, regenerates the TS bindings.
3. Open a follow-up PR in PandaMe with title
   `chore(declarations): bump escrow bindings to v<NEW_VERSION>`.
4. If the new version added endpoints PandaMe wants to consume,
   that's a separate `feat:` PR following the bindings bump.

See [`docs/ai/pr-and-ci.md#9-cross-repo-coordination-pandame`](../../docs/ai/pr-and-ci.md#9-cross-repo-coordination-pandame).

## Troubleshooting

- **`dfx deploy --upgrade-unchanged` says "already deployed".** Drop
  the flag to force a redeploy: `dfx deploy --network local`.
- **`pocket-ic` integration tests fail with "binary not found".**
  Run `scripts/setup` to re-install the test toolchain.
- **`cargo build` fails after a Rust toolchain bump.** Confirm
  `rust-toolchain.toml` matches what's installed (`rustup show`); if
  it doesn't, `rustup install` the pinned version.
- **`npm run did` produces a `.did` diff against an unrelated change.**
  Likely an `@icp-sdk/bindgen` bump. Commit the regenerated `.did`
  together with the bump as `chore(did): regenerate against bindgen
X.Y.Z`.

## See also

- [`docs/ai/pr-and-ci.md`](../../docs/ai/pr-and-ci.md) — full CI matrix.
- [`.claude/rules/ic.md`](../../.claude/rules/ic.md) — IC + Candid quick-ref.
- [`src/escrow/README.md`](../../src/escrow/README.md) — canister
  README (deal lifecycle, ICRC-7 NFT views, scaling roadmap).
