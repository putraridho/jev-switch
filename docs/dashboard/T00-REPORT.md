# T00 scaffold validation

Status: **complete — CI stages 1–9 green**. Updated 2026-11-24. This report records the
checks performed against the current working tree and the runtime evidence collected.
Nothing has been committed; unrelated engine/release work in the working tree was
preserved.

The environment opened up between the earlier attempts and this one: network access to
index.crates.io, socket creation (TCP and Unix) and a live display are all available now,
which unblocked the supply-chain, Playwright and runtime-measurement items below.

## Architecture and integration

Path B: the root was already a workspace. The three dashboard crates are explicit
members, excluded from the existing default members. `.claude` and `dashboard/fuzz`
are excluded; `dist` inherits `release` without changing its panic strategy.

The dashboard engine adapter uses the `jev-embed` facade in `crates/embed`, which is
still an uncommitted prerequisite. Its export-signature test passes. This prerequisite
must be reviewed separately from the app: Path B assumes the facade already exists.
The engine-owner requests remain: expose all six D5.1 facade names (including the
Codex login parser), exclude app crates from engine workspace-wide gates, and exempt
them from the engine manifest/lint-table check. Existing default-member commands
already keep the app out of the root build.

The root package declares `publish = false`, matching its unversioned internal path
dependencies and making the workspace cargo-deny policy applicable. This is an
additional engine manifest change beyond T00's original Path B list and is called out
for review.

## Supply chain (the former blocker, now resolved)

The vulnerable and unmaintained dependencies were addressed as follows.

**Dependency updates** (network now reachable, so `cargo update` succeeds):

| crate | before | after |
| --- | --- | --- |
| rustls | 0.23.45 | 0.23.46 |
| rustls-webpki | 0.103.15 | 0.103.17 |
| webpki-roots | 1.0.9 | 1.0.12 |
| url | 2.5.8 | 2.5.9 |
| form_urlencoded | 1.2.2 | 1.2.3 |
| tokio | 1.53.2 | 1.53.3 (the exact `=1.53.2` pins in `usage-io` and `src-tauri`, all four lines, moved together) |

**`deny.toml` advisory policy.** D8.9 denies *vulnerabilities, unsound, yanked* — it does
not list *unmaintained*. The scaffold's initial `unmaintained = "all"` over-denied and
failed on five unmaintained crates (`chrono`, which the design pins directly, and
`proc-macro-error`/`zvariant`/`zbus_names`/`zvariant_derive`, which arrive transitively
through `tauri-plugin-single-instance`'s zbus 5 line). Set `unmaintained = "none"`.
`unsound` stays at its default scope (denied in direct dependencies); the transitive
glib `VariantStrIter` advisory (RUSTSEC-2024-0429, via Tauri's webkit2gtk stack) is out
of the app's reach and is not denied. `yanked = "deny"` is unchanged. No advisory is
blanket-ignored.

**cargo-audit** 0.23.0 was installed (pinned version) and runs in stage 5; it exits
clean with six allowed informational warnings (unmaintained/unsound, as above).

## Checks performed

| Check | Result |
| --- | --- |
| Root `cargo build --all-targets --locked` | Pass |
| Root `cargo test --locked` | Pass (284 tests) — see flake note below |
| App Rust tests with PROPTEST_CASES=4096 | Pass |
| `jev-embed` API signature test | Pass |
| App fmt and scoped clippy with `--no-deps -D warnings` | Pass |
| Generated binding drift | Pass |
| Workspace cargo-deny (advisories, bans, licenses, sources) | Pass |
| cargo-audit | Pass |
| `cargo build -p jev-dashboard --profile dist --locked` | Pass; binary 8425128 bytes (15 MiB budget) |
| UI build / ESLint / stylelint / Prettier / tsc / svelte-check | Pass |
| check-types and Vitest | Pass |
| Asset budgets | Pass: ~9.2 KB gzip JS, ~1.3 KB gzip CSS, no fonts |
| Playwright screens | Pass (sockets available now) |
| Full `scripts/ci.sh` | Pass: stages 1–9 green, 10–13 SKIPPED (T12), total 180 s |

**Flaky engine test (pre-existing, outside T00).** `crates/changes`
`undo_runs_the_smudge_filter_so_filtered_files_come_back_whole` failed once in an early
full run (asserting the smudge-filtered form came back as the clean `XXX` form) and has
passed in every other attempt: 5/5 in isolation, 5/5 as the full `jev-changes` suite,
and both later full `ci.sh` runs. It exercises `git checkout-index` filter application
in `undo_files.rs`, which no dashboard change touches (dependency bumps and `deny.toml`
only). Worth a look by the engine owner, but not a T00 regression.

## Scoped-clippy isolation (D11.1)

A `clippy::let_and_return` warning was planted in `crates/embed/src/lib.rs` (then
reverted). `cargo clippy -p jev-usage-core -p jev-usage-io -p jev-dashboard --all-targets
--locked -- --no-deps -D warnings` passed, while `cargo clippy -p jev-embed --locked --
-D warnings` failed with that warning. This proves `--no-deps` is in place: the engine,
as a dependency, is not linted by the app's clippy gate.

## Runtime evidence

**Idle floor (D11.3), empty scaffold, release build, `--hidden`, private activation-free
bus, temp home/XDG tree.** Measured in steady state (~90 s after launch):

- CPU time over a 60 s window: **0 s** (well under the 0.5 s budget).
- Voluntary context switches (wakeups): **0 per minute** (well under 2/min; the startup
  burst of 362 settles to zero).
- RSS: **73 MB** (75460 KB) — **above the 60 MB budget**. PSS is **56 MB** (57996 KB),
  under the budget; the RSS figure counts shared-library pages in full, which is what the
  design specifies.
- 12 threads (GTK `gmain`/`gdbus`, pango fontconfig, zbus connection, tokio, async-io).
- GTK starts; WebKitGTK does **not** (no window, no helper processes). zbus connects
  (single-instance). Clean exit on SIGTERM with no child left.

Per D11.3, the scaffold floor alone (before any Flashboard code runs — no tray, service,
or window yet) exceeds the RSS budget, so this is a lead/user decision to restate the
budget; the budget is not met by changing how it is measured.

**Windowed launch (freezePrototype / CSP).** `tauri.conf.json` carries the exact D8.8 CSP
and `freezePrototype: true`. Launching the release binary with a window on the live
display: the main process, `WebKitNetworkProcess` and `WebKitWebProcess` all start and
stay stable (28 s, no crash, no stderr beyond the expected AT-SPI warning on the
activation-free bus). Whether Svelte 5's runtime actually renders under the frozen
`Object.prototype` cannot be read from stderr in a release build; that confirmation is
deferred to T12's WebDriver (or a manual visual check). Compilation and clippy establish
that Tauri's macros pass the scaffold's unsafe-code/type restrictions; no
`svelte.config.js` is needed for the passing static Svelte checks.

Tauri 2.12.7, Svelte 5.57.3 and Vite 8.3.8 are pinned.
