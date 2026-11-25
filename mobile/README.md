# pansy on the phone

The phone app of `docs/plans/phone-ui.md`, in NSH 0.6.1 TSX: one source for
iOS, Android and the web. It speaks `docs/plans/phone-protocol.md` to a home
pansy (`pansy-remote`), or to the mock in `mock/`.

## Layout

- `App.tsx` mounts `ui/Root.tsx`: the two tabs' stacks, the tab bar, and the
  sheets any screen opens (添加机器, 新任务, 重命名).
- `net/` — the protocol. `types.ts` its shapes, `client.ts` one machine's
  `Connection` (ordered routes, refresh, `/ws` with a ticket, JSON-RPC,
  jittered reconnect), `dpop.ts` the device key and proofs, `pair.ts`
  `#pair=` links and pairing, `transport.ts` fetch and WebSocket (tests swap
  it).
- `state/` — `store.ts` the machines and their shell snapshots,
  `thread.ts` a thread's feed folded from history and events, `prefs.ts`.
- `ui/` — the screens (`Threads`, `Thread`, `Composer`, `ModelSheet`,
  `ModeSheet`, `Git`, `Diff`, `NewTask`, `Pair`, `Settings`), their parts
  (`parts.tsx`, `status.tsx`, `actions.tsx`, `Markdown.tsx`), `i18n.ts`
  (every string, English and Simplified Chinese), `icons/` (Lucide) and
  `logos.ts` (LobeHub's, as the desktop's).
- `host.js` — the web page's host (`nsh.json` `web.host`): the browser's
  speech recognition for dictation, where NSH's `Speech` has none.
- `mock/` — `fake.ts` a home pansy in memory with a scripted agent;
  `server.ts` serves it and the web build on Bun.
- `test/harness.tsx`, `App.test.tsx`, `logic.test.ts` — the tests.

## Build and run with the mock

```sh
nsh check && nsh test
nsh package --target web -o dist/web App.tsx
~/.config/pansy/bun/1.4.2/bun-linux-x64/bun mock/server.ts
```

The server listens on `0.0.0.0:7268`, serves `dist/web/Pansy` at `/` and
prints a pairing link (`http://<lan-ip>:7268/#pair=…`): open it on a phone
or in a browser and the app pairs itself. Or open the address and type any
six digits in 添加机器 (`000-000` is refused).

添加机器's code takes six digits (`123-456`) and sends them as the
pairing secret. A real pansy's secret is the 43 characters in the QR code's
link, so typed codes pair only with the mock until pansy-remote issues a
six-digit code of its own; the QR code and the link pair with either.
`--approval` makes new devices wait for approval, `--slow 0.2` speeds the
scripted agent up, `--port`, `--host` and `--dir` move it.

Under the mock, a message to a thread runs a scripted turn: it reads a file,
starts a subagent, streams an answer, then asks to run `cargo test`.

## Against the pansy on this computer

```sh
nsh package --target web -o dist/web App.tsx   # served from mobile/dist/web/Pansy
pansy remote on                                 # then restart pansy serve, or the window
pansy pair                                      # a link to open on the phone
```

Settings › Network and sharing in the window does the same, with a QR code.
The phone reaches port 7268 on this computer's private network or tailnet.

## Notes

- The device key is a WebCrypto P-256 key kept extractable in `Storage`;
  phase 2 moves it into the keychain. Without WebCrypto (an old WebView) the
  app pairs in cookie mode.
- `nsh.json` asks for `"network": { "hosts": ["*"] }`: a home pansy is at
  whatever address the user pairs with.
- The `/ws` proof's `htu` is the `ws://` URL.
