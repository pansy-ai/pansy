# The plugin hosts' scripts

What pansy runs on Bun. `pansy-plugins` embeds them and writes them under
`plugins/` in pansy's folder, named by what they hold, before it starts a host.

| File | Host | Whose |
| --- | --- | --- |
| `host.js` | A: OpenCode provider plugins, `magpie.hooks` | magpie's `internal/plugin/host.js` at `5d23ae860` (MIT, © yetone), with `input.pansy` added beside `input.magpie` |
| `pi.js` | A: pi packages, loaded by `host.js` | magpie's `internal/plugin/pi.js` at `5d23ae860` (MIT, © yetone), unchanged |
| `mw-host.js` | B: `magpie.middleware` and `magpie.agent` | pansy's own |

The names community plugins read stay magpie's: `input.magpie`,
package.json's `magpie.*`, the `x-magpie-*` headers and `MAGPIE_*_PROXY`.

## Bringing in a fix from magpie

1. `git show <magpie commit>:internal/plugin/host.js > host/host.js` (and
   `pi.js`).
2. Put back the comment at the top and the `input.pansy = input.magpie` line
   after `loadPlugins`'s `input`.
3. Read the protocol comment at the top of `host.js` for anything new the
   Rust side (`src/rpc.rs`, `src/host.rs`) has to answer, and update the
   commit named above.
4. `cargo test -p pansy-plugins` with Bun on `PATH` (or `PANSY_BUN` set).
