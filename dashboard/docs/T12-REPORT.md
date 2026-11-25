# T12 report: security proofs, review, mutation checks, idle budgets

State of the work tree on 2026-11-25. Nothing here is committed by this report's author.

## 1. Security review

The `security-review` skill could not start (its setup command runs `git log origin/HEAD...`
and the clone has a dangling `origin/HEAD`), so the review was done by hand over the Rust,
shell and CI side of `git diff main...HEAD -- dashboard scripts/ci.sh` plus the uncommitted
changes. Read in full: `proc.rs`, `http.rs`, `secrets.rs`, `fsutil.rs`, the SQL in `store.rs`,
`service.rs` (loop and shutdown), `watch.rs`, `timer.rs`, `hardening.rs`, `app.rs`,
`window.rs`, `commands.rs`, `autostart.rs`, `bus.rs`, `notifier.rs`, `tauri.conf.json`,
`capabilities/main.json`, `install.sh`, `smoke.sh`, `measure-idle.sh`, `ci.sh`,
`webdriver.py`. No secret, prompt or identity reaches IPC, files, logs or the network
beyond what D8 allows; the findings below are the exceptions.

| #   | Finding                                                                                                                                                                                                                                                                                                                     | Severity            | State                                                                                                                                                                                                        |
| --- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| F1  | `next_deadline` counted a provider's `observed_at + fresh_for` even after it had passed. For any stale reading the deadline was "now", so the service loop never slept: one core busy while hidden (621 s CPU and 9.1 million wakeups per minute in the first idle run). A quiet gateway log triggers it on a real machine. | high (availability) | **Fixed** in `usage-io/src/service.rs` (only a future expiry is waited for). Regression test `usage-io/tests/service_idle.rs` (10,212 timer sleeps in 2 s without the fix, 0 with it).                       |
| F2  | `SessionBusTrayHost::present()` built its own runtime and called `block_on` on the calling thread. From an async command (`get_diagnostics`) tokio panics ("Cannot start a runtime from within a runtime"), so the diagnostics command could never answer.                                                                  | medium              | **Fixed** in `src-tauri/src/bus.rs` (the bus call runs on its own thread). Regression test `src-tauri/tests/async_safety.rs::tray_host_can_be_asked_from_inside_a_runtime`.                                  |
| F3  | `install.sh` put the destination into a `sed` replacement and an unquoted `Exec=` line: a path with a space, `&` or `\|` gave a broken or altered desktop entry.                                                                                                                                                            | low                 | **Fixed**: the line is quoted as the Desktop Entry spec asks (`"`, backtick, `$`, `\` escaped, `%` doubled), written with a read loop, a newline in the path is refused. Checked with a path `pre fix&\|$x`. |
| F4  | `autostart::reconcile` wrote the running binary's path into `Exec=` without refusing a line break, which would start a new line of the entry.                                                                                                                                                                               | low                 | **Fixed**: `InvalidExec` for `\n` or `\r`. Test `async_safety.rs::a_line_break_in_the_exec_path_writes_nothing`.                                                                                             |
| F5  | New env-gated behaviour: `window.rs::automation_requested()` (see 1.1).                                                                                                                                                                                                                                                     | low, accepted       | Assessed below; flag parsing pinned by `async_safety.rs::only_the_exact_value_true_turns_automation_on`.                                                                                                     |
| F6  | T12's acceptance list requires that a `std::io::Result` in a `pub` signature fails a stage. No gate existed (the mutation was not caught).                                                                                                                                                                                  | gap                 | **Fixed**: `usage-io/tests/pub_signatures.rs` scans the three app crates; mutation now caught.                                                                                                               |
| F7  | Could `test-support` (plain-HTTP client, fakes) reach a release build through feature unification? Checked with `cargo tree -e features`: no. Dev-dependencies only, resolver 2.                                                                                                                                            | none                | Pinned by `async_safety.rs::test_support_is_enabled_only_by_dev_dependencies`.                                                                                                                               |
| F8  | `proc::resolve` and the spawn honour empty or relative `PATH` entries (relative to the app's working directory), as any shell does. A planted `./codex` would run only if the user's own `PATH` has such an entry.                                                                                                          | informational       | Reported, not changed: D8.6 says the child is started "by the name `PATH` gave", wrapper included.                                                                                                           |
| F9  | Store recovery (`SqliteStore::open`) renames a corrupt `history.db` to `.corrupt-<ns>` but leaves its `-wal`/`-shm` behind, and the `.corrupt-*` files are never pruned.                                                                                                                                                    | low                 | Reported to the lead (retention and a stale-WAL risk), not changed.                                                                                                                                          |
| F10 | `lib/ipc/select.ts` ships a mock client in the bundle, chosen when `__TAURI_INTERNALS__` is absent. Inside Tauri the global is injected before page scripts, so the real client is always used; the mock makes no request and its data is synthetic.                                                                        | informational       | Reported (UI file, not edited by this fork).                                                                                                                                                                 |
| F11 | `check-types.ts` currently reports 2 findings in UI files under construction: `HistoryChart.svelte:178` (`svelte-ignore`) and `FirstRun.svelte:13` (type predicate). `scripts/check-types.test.ts` "finds nothing in the project itself" fails until they are fixed.                                                        | gate failing        | **Open**, UI side (not edited by this fork).                                                                                                                                                                 |

Also found and fixed on the way (not security): `canary_e2e.rs` took 64 s because
`#[tokio::test]` waits at the end for the timer's blocking thread. It now builds its runtime
by hand and shuts it down with a 1 s limit (1.9 s).

### 1.1 `TAURI_WEBVIEW_AUTOMATION` makes the view non-incognito

`tauri-driver` sets the variable for the app it launches; `window.rs` then builds the view
without `incognito`, because wry gives an incognito view a fresh ephemeral context that
cannot be automated (section 5).

- **What it changes:** the view uses a persistent WebKit context. The only content ever
  loaded is the bundled UI (CSP `default-src 'none'`, no remote origin), so what it can store
  is the UI's own state, and the UI stores nothing (ESLint bans every web-storage API).
  tauri-runtime-wry also allows automation on that context only when the same variable is
  `true`.
- **Who can set it:** whoever controls the app's environment. That party can already set
  `LD_PRELOAD`, `PATH`, `HOME` or replace the binary, so no privilege boundary is crossed.
  Automation additionally needs a WebDriver server that launches the app itself; it cannot
  attach to a running window.
- **Residual risk:** a session that exports the variable globally (a developer machine)
  would run Flashboard non-incognito and automatable. Mitigation in place: only the exact
  value `true` counts (tested), the normal launch and the smoke test never set it, and the
  smoke test asserts that a normal launch holds no page-data store. Not done: refusing the
  variable in a build without a debug marker, because the WebDriver stage must drive the
  release binary.

## 2. Mutation checks

Each rule was broken in a copy-aside file or a throwaway new file, the matching command run,
and the file restored from the copy (`git diff` clean afterwards for every mutated file).

| Mutation                                                    | Command that failed                                | Proof that caught it                                                                             |
| ----------------------------------------------------------- | -------------------------------------------------- | ------------------------------------------------------------------------------------------------ |
| `unwrap()` in `src-tauri/src/cli.rs`                        | `cargo clippy -p jev-dashboard -- -D warnings`     | `clippy::unwrap_used`                                                                            |
| `unsafe` block in `src-tauri/src/cli.rs`                    | `cargo check -p jev-dashboard`                     | `unsafe_code = "forbid"`                                                                         |
| `#[allow(dead_code)]` in `usage-core/src/time.rs`           | `cargo test -p jev-usage-core --test manifests`    | `the_only_lint_allows_are_the_test_header_and_the_one_cast`                                      |
| `println!` in `usage-core/src/time.rs`                      | `cargo test -p jev-usage-io --test architecture`   | `no_print_or_dbg_macros_in_source`                                                               |
| a second `expose_secret(` in `usage-io/src/secrets.rs`      | `cargo test -p jev-usage-io --test architecture`   | `each_forbidden_api_lives_only_in_its_one_file`                                                  |
| `pub fn ... -> std::io::Result` in `usage-core/src/time.rs` | `cargo test -p jev-usage-io --test pub_signatures` | `no_io_error_type_in_a_public_signature` (new; before it the mutation passed every stage)        |
| extra permission `core:window:allow-close`                  | `cargo test -p jev-dashboard --test hardening`     | `capabilities_are_exactly_the_event_listeners_and_six_commands`                                  |
| `style="..."` in a new `.svelte` file                       | `eslint --no-inline-config`                        | `svelte/no-inline-styles`                                                                        |
| `localStorage` call in a new `.ts` file                     | `eslint --no-inline-config`                        | `no-restricted-globals` ("The web view stores nothing")                                          |
| `el.innerHTML = ...` in a new `.ts` file                    | `eslint --no-inline-config`                        | `no-restricted-syntax` ("No raw-HTML sink")                                                      |
| a `#ff0000` colour in a new `.svelte` `<style>`             | `stylelint`                                        | `declaration-strict-value`, `color-no-hex`                                                       |
| `as` assertion, non-null `!`, `object` type                 | `eslint --no-inline-config`                        | `@typescript-eslint/no-restricted-types` (first hit); the rest by `scripts/eslint-rules.test.ts` |

Earlier mutations from the canary and IPC work (a `println!`, a second `expose_secret`, a
`serde_json::Value`, an `email` key in a generated `.ts` file, an extra capability
permission) were re-confirmed by the fork that wrote those proofs. The remaining items of
the T12 acceptance list (`any`, `unknown`, `never`, `@ts-expect-error`, `catch (e)`, type
predicate, `JSON.parse`, inferred `any`, `eslint-disable`, second float cast) are proven by
the sample table in `scripts/banned-samples.ts` through `scripts/eslint-rules.test.ts` and
`scripts/check-types.test.ts`, not by a fresh mutation. A remote `<img>` is proven by
`e2e/screens/security.spec.ts` ("the production CSP blocks a remote image, a remote script
and a remote fetch"), a low-contrast token by the contrast test of T13.

## 3. D8.10: rule to proof

All of these pass on this tree except where a note says otherwise.

| Rule                                                                       | Proof (test or manual step)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| -------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| No prompt can surface from the log parser                                  | `usage-core/tests/parse_security.rs::fixture_canaries_do_not_surface`; `usage-core/tests/entry_points.rs`; fuzz target `claude_log` (stage 12, advisory)                                                                                                                                                                                                                                                                                                                                                                                         |
| No secret or identity crosses any adapter                                  | `usage-io/tests/deepseek_balance.rs::canary_key_and_canary_payload_never_leak_on_success_or_any_failure`; `usage-io/tests/canary_e2e.rs` (Codex e-mail, Claude e-mail, org id and name); `usage-core/tests/parse_security.rs`                                                                                                                                                                                                                                                                                                                    |
| End to end: nothing leaks to IPC, store, settings, logs                    | `usage-io/tests/canary_e2e.rs::canaries_never_reach_ipc_store_settings_or_diag` (snapshots, every file's raw bytes incl. WAL, `diag`); `e2e/smoke/smoke.sh` (the whole temp tree and stderr). Limit: no DeepSeek key canary in the e2e (the endpoint is a constant, a key would make a real request); `settings.json` is scanned only if it is written                                                                                                                                                                                           |
| IPC carries only allowed fields                                            | `usage-core/tests/ipc_types.rs::every_key_is_on_the_allow_list_and_every_listed_key_is_used`, `::allow_list_has_no_identity_or_secret_shaped_key`; `usage-core/tests/ipc_audit.rs` (generated `MeterView` proptest, generated `.ts` files)                                                                                                                                                                                                                                                                                                       |
| `Secret` cannot be printed, cloned, serialised                             | `usage-core/src/secret.rs` `assert_not_impl_any!` (compile-time)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| One exit for secrets, one HTTP client, one process spawner, one D-Bus user | `usage-io/tests/architecture.rs::each_forbidden_api_lives_only_in_its_one_file`, `::the_engine_is_named_only_in_the_bridge`; `usage-io/tests/deepseek_balance.rs::secret_exit_and_http_client_crate_exist_only_in_http_rs`; `src-tauri/tests/async_safety.rs::test_support_is_enabled_only_by_dev_dependencies`                                                                                                                                                                                                                                  |
| Children the dashboard spawns get no secrets                               | `usage-io/tests/proc_subprocess.rs::status_probe_and_sanitized_child_environment`; `usage-io/src/proc.rs::environment_lists_equal_the_design`                                                                                                                                                                                                                                                                                                                                                                                                    |
| Files are private                                                          | `usage-io/tests/settings_persistence.rs::private_settings_round_trip_and_invalid_recovery`; `usage-io/tests/canary_e2e.rs` (0700 directory, 0600 `history.db`, `-wal`, `-shm`; the test relies on the app's own modes, it does not set a umask itself); `e2e/smoke/smoke.sh`                                                                                                                                                                                                                                                                     |
| The web view stores nothing                                                | ESLint bans the storage APIs (mutation above). **Deviation:** the smoke test's check is on content, not on the directory: WebKit makes `<data>/dev.jev.flashboard/{WebKitCache,storage,mediakeys}` even for an incognito view, holding only an 8-byte `salt` and an empty `hsts-storage.sqlite`. `smoke.sh` fails on `localstorage`, `indexeddb`, `cookies*`, `serviceworkers`, `cacheStorage`, any other file there, or a non-empty HSTS table. The literal "no WebKit data directory" cannot be met; the lead should confirm the reworded rule |
| The panic hook leaks no path                                               | `src-tauri/tests/hardening.rs::panic_hook_prints_a_base_name_and_a_line_and_no_path_or_payload`                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| Capabilities and CSP do not drift                                          | `src-tauri/tests/hardening.rs::capabilities_are_exactly_the_event_listeners_and_six_commands`, `::csp_and_security_flags_are_frozen`, `::plugin_list_is_one_and_no_devtools_feature`                                                                                                                                                                                                                                                                                                                                                             |
| The web view makes no network request                                      | `e2e/screens/security.spec.ts` (7 tests: no foreign request and no CSP violation on each screen, the production CSP blocks a remote image, script and fetch, no inline style or script)                                                                                                                                                                                                                                                                                                                                                          |
| No raw HTML sinks                                                          | ESLint `no-restricted-syntax` (mutation above); `scripts/eslint-rules.test.ts`                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |

D9.10 to test map: see `docs/T13-AUDIT.md`.

## 4. Idle budgets (D11.3)

`dashboard/scripts/measure-idle.sh --floor <T00 scaffold> <app>`, the full 60 s to 660 s
window, release (`dist`) binaries, hidden, private activation-free bus, temp `HOME` and every
`XDG_*` inside a temp tree, stub CLIs. The floor is T00's commit (`53b7869e`) built the same
way. Figures are summed over the process tree and all threads.

| Figure                       | Budget      | App (fixed build)           | Empty scaffold (floor) | Result                          |
| ---------------------------- | ----------- | --------------------------- | ---------------------- | ------------------------------- |
| CPU time over the window     | < 0.5 s     | 0.03 s                      | 0.03 s                 | pass                            |
| Wakeups (voluntary switches) | < 2 per min | 136.1 per min               | 131.9 per min          | **FAIL**                        |
| RSS                          | <= 60 MB    | 61.1 MB                     | 50.6 MB                | **FAIL** (1.1 MB over)          |
| Binary                       | <= 15 MB    | 14,889,448 bytes (14.2 MiB) | 8,418,344 bytes        | pass                            |
| Processes alive              | -           | 1                           | 1                      | no WebKit helper remains hidden |

The first run, before F1 was fixed, measured 620.98 s CPU and 9,144,388 wakeups per minute
(RSS 42.4 MB): that run is what exposed the spin.

Reading the two failures (numbers, not a decision):

- **Wakeups.** The empty scaffold already makes 132 per minute; Flashboard adds about 4.
  Per-thread sampling of the app over 30 s: 61 switches on the thread named `blocking-1`
  (about 2 per second, steady), 7 on the main thread, one or two elsewhere. `blocking-1` is
  the pool of the `blocking` crate that the D-Bus stack (zbus, async-io, used by
  `tauri-plugin-single-instance`) runs on; the service loop itself sleeps (0 timer sleeps in
  a 2 s window, `service_idle.rs`). So the budget cannot be met while the single-instance
  plugin keeps a D-Bus connection. One option for the lead: single-instance by a lock file
  and Unix socket under `XDG_RUNTIME_DIR`, which removes that thread; not tried.
- **RSS.** 61.1 MB against 60 MB; the floor alone is 50.6 MB by this script's method.
  T00's report quoted 73 MB RSS for the floor, measured by a different procedure (~90 s
  after launch, one 60 s window); the two are not comparable.

**Restated by the user** (after this run; measurement method unchanged): wakeups at most 10
per minute above the floor, RSS <= 65 MB, and `WindowReady` <= 550 ms. Against the figures
above: wakeups +4.2 per minute (pass), RSS 61.1 MB (pass). The idle run was not repeated.
The single-instance replacement is not planned.

## 5. D16 findings

1. **A `webkitgtk-6.0` `WebKitWebDriver` drives the `webkit2gtk-4.1` app: works.** Installed
   versions: `webkit2gtk-4.1` 2.54.1 and `webkitgtk-6.0` 2.54.1 (same engine version).
   `e2e/webdriver/webdriver.py` passes in the real view with `tauri-driver` 2.0.1: overview
   numbers, detail and settings by keyboard, a threshold saved to `settings.json`, axe clean
   on all three screens. Not tried: mismatched engine versions.
2. **An incognito view cannot be automated.** wry builds `WebContext::new_ephemeral()` for an
   incognito view and ignores the configured context on which `tauri-runtime-wry` allows
   automation, so session creation fails ("failed to create a new browsing context"). The
   WebDriver run therefore launches the view non-incognito through
   `TAURI_WEBVIEW_AUTOMATION=true` (section 1.1). The first failure before that, "Failed to
   match capabilities", was `browserName: wry` in the capabilities: tauri-driver wants only
   `tauri:options`.
3. **WebKit creates files even for an incognito view:** `<data>/dev.jev.flashboard/` with
   `WebKitCache/Version 17/{salt,Blobs}`, `storage/salt`, `mediakeys/v1/salt` and an empty
   `hsts-storage.sqlite`, plus the GPU shader cache under `XDG_CACHE_HOME` (`mesa_shader_cache`).
   No page data. See the deviation in section 3.

## 6. Supply chain

- `cargo deny check`: advisories ok, bans ok, licenses ok, sources ok. Two
  `license-not-encountered` warnings (allowances no crate in the graph uses; `Zlib` is one).
  `deny.toml` has `ignore = []`: no exception.
- `cargo audit`: no vulnerabilities; 6 allowed warnings, all transitive or pinned by D8.9:
  `chrono` RUSTSEC-2026-0543 (unmaintained, pinned directly by D8.9), `glib` RUSTSEC-2024-0429
  (unsound `VariantStrIter`, via Tauri's webkit2gtk stack), `proc-macro-error`
  RUSTSEC-2024-0370, `zbus_names` RUSTSEC-2026-0371, `zvariant_derive` RUSTSEC-2026-0372,
  `zvariant` RUSTSEC-2026-0370 (unmaintained, single-instance's zbus 5 line). None is
  time-boxed in a config file; they are the accepted set of D8.9.
- `pnpm audit --prod`: no known vulnerabilities.

## 7. Not done

- The `security-review` skill itself (setup failure above); the review was manual.
- No fresh mutation for each TypeScript type-discipline rule (covered by the sample tables).
- The idle budgets fail as measured (section 4); nothing was changed to meet them.
- F9, F11 and the single-instance option are reported, not fixed.
