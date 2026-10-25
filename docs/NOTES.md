# Notes

Facts gitroute depends on, with where each was checked.

## GitHub OAuth device flow

Checked against docs.github.com on 2026-10-22
([authorizing OAuth apps](https://docs.github.com/en/apps/oauth-apps/building-oauth-apps/authorizing-oauth-apps),
[creating an OAuth app](https://docs.github.com/en/apps/oauth-apps/building-oauth-apps/creating-an-oauth-app)).

- `POST https://github.com/login/device/code` with `client_id`, `scope`. The
  response is form-encoded unless the request sends `Accept: application/json`.
  Fields: `device_code`, `user_code`, `verification_uri`, `expires_in` (900),
  `interval`.
- Poll `POST https://github.com/login/oauth/access_token` with `client_id`,
  `device_code`, `grant_type=urn:ietf:params:oauth:grant-type:device_code`.
  No client secret.
- `slow_down` adds 5 s and "the error response includes the new `interval`".
  gitroute uses that value, or adds 5 s itself if it's missing.
- Expiry: the docs' table says `expired_token` and their text says
  `token_expired`. Both are handled.
- The OAuth App form has **"Expire user access tokens" ticked by default**.
  When it's ticked, the token response includes `expires_in` and
  `refresh_token`. gitroute keeps the default and refreshes (see below).
  It still works with long-lived tokens if the app opts out.
- Observed on 2026-10-25 with the gitroute app: a `gho_` token with
  `expires_in` = 28800 and a refresh token. Per the
  [2026-08-14 changelog](https://github.blog/changelog/2026-08-14-multiple-redirect-uris-and-token-refresh-for-oauth-apps/),
  "Short-lived tokens are enabled by default for all new applications", and
  apps can opt out. After registration, the switch is on the app's
  **Optional features** page
  ([docs](https://docs.github.com/en/apps/oauth-apps/maintaining-oauth-apps/activating-optional-features-for-oauth-apps)),
  not the main form.
- Refresh: `POST /login/oauth/access_token` with `client_id`,
  `grant_type=refresh_token` and `refresh_token`. `client_secret` is
  "required unless the token was generated using the device flow", so
  gitroute sends none. Refresh tokens are single-use; an invalid or expired
  one returns `bad_refresh_token`. Access tokens last 8 h, refresh tokens
  6 months without use.

### How gitroute refreshes

- The Keychain item holds JSON: the access token, its absolute expiry, the
  refresh token and its expiry.
- `TokenStore::access` refreshes when under 5 minutes are left. It takes an
  exclusive lock on `~/Library/Application Support/gitroute/locks/token-<id>.lock`,
  then re-reads the token before refreshing, in case another git process
  just did. A test runs 4 threads at once and checks that exactly one
  refresh happens.
- `bad_refresh_token`, an expired refresh token, or no token at all →
  "needs re-login". A network error is just an error: the account isn't
  flagged.
- Expiring tokens are a trade-off. The helper has to make a network call
  about once every 8 hours per account. Not using an account for 6 months
  means logging in again.
- "Enable Device Flow" must be ticked, or polling returns
  `device_flow_disabled`.

## GitHub REST

- `GET /user/emails` needs the `user:email` scope. Each entry has `email`,
  `primary`, `verified` and `visibility`. `per_page` defaults to 30 (max 100).
  gitroute asks for 100.
- The `X-GitHub-Api-Version` value shown on the emails page is `2026-03-10`.
- noreply address: `ID+USERNAME@users.noreply.github.com` for accounts
  created after 2017-07-18
  ([email addresses reference](https://docs.github.com/en/account-and-profile/reference/email-addresses-reference)).
  gitroute adds it to the commit email choices if `/user/emails` doesn't
  list one. Observed 2026-10-25: for AmanKumarVerma11, `/user/emails` did
  list `93284136+AmanKumarVerma11@users.noreply.github.com`.

## Keychain: who can read a token without a prompt

Experiment, 2026-10-25, macOS 26.6.2. Two small Rust programs, A and B, used
the same Keychain code gitroute used in M2 (`keyring-core` +
`apple-native-keyring-store`, legacy keychain API). The service was a
throwaway, `gitroute-experiment`. Reads ran with
`SecKeychainSetUserInteractionAllowed(false)`, so a read that would show an
"allow access" dialog fails with -25293 instead of showing it.

| Reader → item created by | Result |
|---|---|
| A → A | ✅ ~5 ms |
| B → A | ❌ would prompt |
| byte-identical copy of A at another path → A | ✅ (identity is the binary's hash, not its path) |
| A rebuilt with a one-line change → A | ❌ would prompt |
| A or B → `/usr/bin/security` (even `add-generic-password -A`) | ❌ would prompt |
| `/usr/bin/security` → `/usr/bin/security` | ✅ ~13–19 ms, no prompt |

So with the keyring crates, the helper would prompt (or, with UI disabled,
fail) for any token the CLI or app wrote, and again after every rebuild or
upgrade of an unsigned binary. Avoiding that would need all binaries signed
with one Developer ID.

**What gitroute does:** every Keychain read, write and delete goes through
`/usr/bin/security`, which is what `gh` does too. Item: generic password,
service `gitroute`, account = GitHub user id, value = the token JSON.
- Write: `security -i` with `add-generic-password -U -s gitroute -a <id> -w
  '<json>'` sent on stdin, so the token never appears in `ps`. Interactive
  mode can exit 0 after a failed command, so gitroute reads the value back
  to confirm the write.
- Read: `find-generic-password -s gitroute -a <id> -w`. Exit 44 means not
  found.
- Trade-off: any process running as the user can read the token through
  `/usr/bin/security` without a prompt. `gh` has the same exposure.
- Migration: items created by the M2 build (keyring crates) can only be read
  or deleted without a prompt by that exact binary. A copy is kept for this
  (see the M3 report).

## Real install, 2026-10-25

- Deleting an item has a stricter check than reading it. A byte-identical
  copy of the binary that created the item could *read* it from another
  path, but deleting failed with "Invalid attempt to change the owner of
  this item". It worked from the original path (`target/debug/gitroute`).
- After migrating, both tokens are read through `/usr/bin/security` from
  `~/.cargo/bin/gitroute` and `git-credential-gitroute` with no prompt.
- A private personal repo (`AmanKumarVerma11/indiabites`) answered
  `git ls-remote` over HTTPS with the personal token.
- **Org OAuth App access restrictions.** `SynthesysApp` has them enabled.
  The work token (scopes `read:user, repo, user:email, workflow`) gets
  `403 … the SynthesysApp organization has enabled OAuth App access
  restrictions` from the API. Over git, it's "Repository not found": a 404,
  so git calls no `erase` and the account isn't flagged. Fix: the member
  requests approval (Settings → Applications → Authorized OAuth Apps →
  gitroute → Request access next to the org), and an org owner grants it
  (org Settings → Third-party Access → OAuth app policy → Review → Grant
  access).
  ([member docs](https://docs.github.com/en/account-and-profile/setting-up-and-managing-your-personal-account-on-github/managing-your-membership-in-organizations/requesting-organization-approval-for-oauth-apps),
  [owner docs](https://docs.github.com/en/organizations/managing-oauth-access-to-your-organizations-data/approving-oauth-apps-for-your-organization)).
  `doctor` and the app should explain this case, since git's message
  ("not found") is misleading.

## Experiment 1: what git gives a credential helper

2026-10-25, git 2.54.0 (Apple Git-157). A local HTTP server answered 401 to
everything, and a probe helper logged its working directory and env.
`GIT_CONFIG_NOSYSTEM=1`, temp HOME.

| Operation | cwd | `GIT_DIR` |
|---|---|---|
| fetch / pull / push / ls-remote, run from `repo/sub/dir` | `repo` (git moves to the repo's top folder) | `.git` (relative) |
| `git -C repo fetch` | `repo` | `.git` |
| `GIT_DIR=/abs/repo/.git git fetch`, run from `/` | `/` | `/abs/repo/.git` |
| `git clone URL cloned` | where `git clone` was typed | `<that dir>/cloned/.git` (absolute) |
| `git clone URL /elsewhere/x` | where `git clone` was typed | `/elsewhere/x/.git` |
| fetch in a linked worktree | the worktree | `main/.git/worktrees/<name>` |
| fetch inside a submodule | the submodule's folder | `super/.git/modules/<path>` |
| `git submodule update --init` | the **superproject** | `super/.git/modules/<path>` |

- git always passes `GIT_DIR`. `git rev-parse --show-toplevel` run from the
  helper is **wrong** when `GIT_DIR` is absolute: it gave the clone's parent,
  and `/` in the `GIT_DIR` case. So the helper resolves on
  `cwd.join(GIT_DIR)`, and falls back to cwd only if `GIT_DIR` is unset. This
  replaces the brief's "rev-parse --show-toplevel" plan.
- A clone resolves on the *new* repo's path, not its parent. That's better
  than the brief expected.
- `erase` follows `get` after a 401, with the rejected `username` and
  `password`.
- git 2.54 sends extra attributes (`capability[]=authtype`,
  `capability[]=state`, `wwwauth[]=…`). The helper ignores attributes it
  doesn't use.

## Experiment 2: helper ordering vs Apple's osxkeychain

- Apple git reads `credential.helper=osxkeychain` from
  `/Applications/Xcode.app/Contents/Developer/usr/share/git-core/gitconfig`,
  which `git config --show-scope` reports as scope `unknown`.
  `GIT_CONFIG_NOSYSTEM=1` hides it too, which is how tests stay isolated.
- `GIT_TRACE=1 git credential fill` for github.com:
  - With no reset, `osxkeychain` runs **before** gitroute: the stale-token
    problem.
  - With `[credential "https://github.com"] helper =` then `helper = <ours>`
    in a file included at the end of the global config, only gitroute runs.
  - An unscoped `credential.helper = osxkeychain` in `~/.gitconfig` is
    cleared for github.com, whether it comes before gitroute's block or after
    it. Unscoped values after a URL-scoped one are ignored by git's URL
    matching.
  - Other hosts (gitlab.com) still use `osxkeychain`, so the reset only
    affects github.com.

## Experiment 3: `includeIf "gitdir/i:…"` matching

| Case | Matches? |
|---|---|
| Rule = canonical path, repo opened via its real path | ✅ |
| … repo opened through a symlinked parent | ✅ (git also checks the real path) |
| … `cd` path in different case (`desktop/code/WORK`) | ✅ |
| Rule written through a symlink path, repo opened via its real path | ❌ |
| Rule in different case, `gitdir/i:` | ✅ |
| Rule in different case, `gitdir:` | ❌ |
| `gitdir/i:~/desktop/...` with real `~/Desktop` | ✅ |
| Linked worktree **outside** the rule, main repo inside | ✅ (follows the main repo) |
| Linked worktree **inside** the rule, main repo outside | ❌ |
| Submodule of a repo inside the rule | ✅ (its gitdir is in `super/.git/modules`) |
| `GIT_DIR=<repo inside rule>/.git` run from `/` | ✅ |

So rules are stored as canonical paths, written as `gitdir/i:<path>/`. Since
the helper resolves on `GIT_DIR` too, a repo's token and commit identity
always come from the same rule, worktrees included. Glob characters in
folder names (`* ? [ \`) are escaped in the pattern.

## Experiment 4: eframe + tray-icon (for M4)

Spike, 2026-10-25: eframe 0.37.0 (glow), tray-icon 0.26.1, muda 0.21.2,
winit 0.30.13.
- ✅ Create the `TrayIcon` inside eframe's app-creator closure: main thread,
  after the event loop started. Keep it in app state; dropping it removes
  the icon.
- ✅ Closing the window keeps the app alive: on `close_requested`, send
  `ViewportCommand::CancelClose` + `Visible(false)`. The process stays up,
  and `App::logic` keeps running while the window is hidden. `ui()` doesn't
  run while hidden. eframe 0.37's `App` has `logic` and `ui`, no `update`.
- ⚠️ Re-showing: send `Visible(true)` + `Focus`, then keep calling
  `request_repaint_after` until `viewport().visible() == Some(true)`. A
  single repaint didn't bring the window back.
- Menu events: `MenuEvent::set_event_handler` can be set only once, and
  once it's set `MenuEvent::receiver()` gets nothing. Forward events to our
  own channel and call `ctx.request_repaint()` from the handler. The same
  goes for `TrayIconEvent`.
- ✅ No Dock icon without a bundle: `ActivationPolicy::Accessory` via
  `NativeOptions::event_loop_builder`. This needs `winit = "0.30"` as a
  direct dependency for `EventLoopBuilderExtMacOS`. The bundle will set
  `LSUIElement` anyway.
- Use `with_icon_templated` (`with_icon_as_template` is deprecated).
- Not tested: real menu clicks while the window is hidden, which can't be
  driven from a shell.

## Helper performance

`get` with the file token store, 30 runs: median 4.2 ms (debug), 4.5 ms
(release). The first run of a freshly built release binary took 473 ms,
consistent with macOS checking a new executable once. A Keychain read adds
~13 ms (one `security` call), so a typical `get` is ~20 ms against a 50 ms
target.

## M4: the app

- egui's bundled fonts have no `●` (U+25CF), and egui_kittest panics on
  missing glyphs. The status dot is painted with
  `painter().circle_filled` instead.
- egui_kittest checks accessibility by default. Text fields and dropdowns
  get a name from their visible label via `Response::labelled_by`, so
  VoiceOver can announce them.
- `install` records the helper path in `config.toml` (`helper_path`), so
  later changes from either the app or the CLI keep git pointed at the same
  binary. When git uses a helper that isn't the app's own, the app says so
  and offers "Use this app's helper".
- The app always opens its window at launch. Closing it hides it to the menu
  bar; Quit is in the menu. Starting hidden wasn't tested on a real screen,
  so it isn't used.
- Measured (bundled release build, window open, idle): 0.1–0.2% CPU,
  ~114 MB RSS. Launched with a temp HOME, it creates no files until
  something changes.
- `scripts/bundle.sh` builds `dist/gitroute.app` (`LSUIElement`, ad-hoc
  signed, ~26 MB). The app, helper and CLI are all in `Contents/MacOS`, and
  installing from the app points git at the helper inside the bundle.
