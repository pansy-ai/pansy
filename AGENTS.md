# Notes for coding agents

pansy is a Rust rewrite of magpie: a local gateway that lets every coding
agent use every provider's models, with a desktop window on NSH 0.6.1. Read
`README.md`, then `docs/parity.md` for what is ported and what is not.

## Layout

- `crates/` — one Cargo workspace: `pansy-core` (data and files, and the
  subscriptions' sign-ins in `subscription/`),
  `pansy-mask` (privacy masking), `pansy-gateway` (the gateway),
  `pansy-agents` (agent configs), `pansy-plugins` (magpie's community
  plugins on Bun, `docs/plugins.md`), `pansy-cli` (the `pansy` command).
- `desktop/` — a workspace of its own, so NSH's crates (GTK, AppKit, Win32)
  stay out of the core's builds: `App.tsx` and `ui/` (the window in TSX),
  `native/` (`pansy/native`, what the TSX imports), `runner/` (the program:
  the window, or the command line when given arguments).

## Invariants

- **Tests never touch the user's files.** Every path goes through
  `pansy_core::paths` (`PANSY_HOME`, `HOME`); tests set both to a scratch
  directory, and `desktop/test.sh` runs the UI tests in one.
- **An agent's config is the user's file.** Edit one key in place
  (`pansy-agents/src/files.rs`, `jsonedit.rs`), record what it replaced
  (`state.rs`), and put back only that on disconnect.
- **An agent's sessions are the user's files too.** `pansy-agents/src/sessions/`
  only reads them (OpenCode's database opened read only); Delete moves a
  session's files into pansy's trash (`trash.rs`), leaves one written to in
  the last minute alone, and erases only on Delete forever or Empty the
  trash. OpenCode's sessions are rows of a database it keeps open: listed,
  never deleted.
- **A request passes through when the protocols match.** Translate only
  between different protocols, through `ir.rs`; a vendor's reasoning seal
  goes back only to that vendor (`proto/mod.rs`).
- **One run writes nothing.** `pansy run` points an agent at the gateway by
  its arguments and environment (`pansy-agents/src/run.rs`), never its files.
  A project's `.pansy.toml` holds model names alone, and counts only once
  the user said yes to it as it is.
- **An agent is a manifest.** `crates/pansy-agents/agents/*.toml`; a change
  to what one sets is blessed into `agents/edits.golden`
  (`PANSY_BLESS=1 cargo test -p pansy-agents`), never slipped in.
- **`docs/compat.md` is written by a test** (`PANSY_BLESS=1 cargo test -p
  pansy-gateway --test compat`). Quirk rules (`quirks.toml`) change only
  requests the gateway translated.
- **Keys are private.** `providers.json` and `settings.json` are written
  0600; keys are masked everywhere they are shown. With `keyStore` `system`
  they are in the system's key store and `providers.json` says only where;
  tests use a folder (`PANSY_KEYSTORE=dir:…`), never the user's keyring.
- **A sign-in stays where it is kept.** A subscription's account is read
  from the agent's own file (Codex's `auth.json`, Copilot's `apps.json`,
  Gemini CLI's `oauth_creds.json`) or
  pansy's `logins.json` (0600) each time providers load; `providers.json`
  holds who and where, never a token. A refreshed token goes back only where
  it was read, every other field as it was, and only over the sign-in the
  refresh began from (`subscription/codex.rs`). Copilot's and Gemini CLI's
  files are never written (Google's refreshed tokens stay in memory); tests point `HOME` at a scratch directory so no real sign-in is
  read.
- **Masked values stay in memory.** Masking's records on disk
  (`pansy-core/src/masking.rs`) hold a value's kind, first characters and
  length, never the value; the text before and after is kept in memory only.
- **Every window string is in `desktop/ui/i18n.ts`** in English and
  Simplified Chinese; `nsh check` fails otherwise.
- **A release is signed, and its keys stay out of git.** `desktop/keys/`
  holds the secrets (`docs/release.md`); `nsh.json`'s `"updater"` holds what
  every pansy installed trusts (the feed's address, the update key, the
  root), so it does not change. The
  build in `nsh.json` grows with every release (`packaging/bump.sh`), and a
  release of the window alone carries no Rust change.

- **A plugin's tests run on a Bun of their own.** `pansy-plugins`' tests
  that need Bun take `PANSY_BUN` or one on `PATH`, and pass saying so
  without one; nothing is
  downloaded in a test. `host/host.js` and `host/pi.js` stay magpie's: a
  change there is a change to magpie's protocol.

## Commands

```sh
cargo fmt --all && cargo clippy --workspace --all-targets && cargo test --workspace
cd desktop && cargo fmt --all && cargo clippy --workspace && nsh check && ./test.sh
```

NSH 0.6.1 exactly (`nsh --version`): the crates are pinned to `=0.6.1`.
