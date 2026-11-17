# Plugins

pansy runs [magpie](https://github.com/yetone/magpie)'s community plugins
as magpie does: on [Bun](https://bun.sh), in processes of their own. A
plugin is an npm package, a repository or a folder, and can be several of:

| Kind | What it does | Runs in |
| :-- | :-- | :-- |
| OpenCode auth plugin | Signs in to a provider (a page, a code or a key) and sends its requests | Host A (`host.js`, magpie's) |
| pi package | A provider pi's packages add: models, sign-in, requests | Host A (`pi.js`, magpie's), with pi installed beside it |
| `magpie.hooks` | Hears each request before it goes out and how it went, without holding it up | Host A |
| `magpie.middleware` | Sees each request before it is masked and may change, reject or redirect it, pick its account and provider, rewrite or drop streamed events, and give routing groups a routing of its own | Host B (`mw-host.js`, pansy's) |
| `magpie.agent` | Adds an agent to the Agents page, with the config file it is wired through | Host B |

OpenCode's V2 plugins (`@opencode/plugin`) are refused: they are a
different API.

## Using them

In the window: Models › Plugins. Type a community plugin's name, an npm
package (`name`, `name@1.2.3`), `github:owner/repo` or a path, and Add; or
pick one from the community lists, read again once a day from pansy's
plugin hub, [hub.pansyai.dev](https://hub.pansyai.dev)
([pansy-ai/pansy-registry](https://github.com/pansy-ai/pansy-registry)),
which reads npm and GitHub for them: OpenCode's plugins on npm that sign in
to a provider (`opencode-plugin`), pi's packages on npm that add one
(`pi-package`), both most downloaded first, and the hub's curated list,
begun from magpie-community's registry (with a copy shipped for when the hub
can't be reached). The npm lists are found by keywords and words like
"auth" and "provider", so a few that turn out to serve no provider may be
among them. Search asks the hub too. A plugin's switch
turns it off without removing it; Options holds its options as JSON; a
provider it signs in to has Sign in, and each account Sign out.

国内镜像 (Mirrors in China, at the top of the page, or `pansy settings
set chinaMirror true`), as magpie's: Bun's release and plugins' packages
are asked of npmmirror first (`bun install` told so, unless a registry of
your own is named in `BUN_CONFIG_REGISTRY`, `NPM_CONFIG_REGISTRY`, a
`.bunfig.toml` or an `.npmrc`); what it can't answer is asked of the
official address. Bun's zip is checked against its checksums all the same.
The hub needs no mirror: it is reached from China as it is.

On the command line: `pansy plugin` (`pansy help` lists every command).

```sh
pansy plugin add workbuddy           # a community plugin by its name
pansy plugin add ./my-plugin         # a folder of your own, loaded where it is
pansy plugin login workbuddy         # sign in, as the plugin asks
pansy plugin                         # what is added and the providers it serves
pansy group add fast models=a/x,b/y routing=myplugin/fastest
```

A provider a plugin serves is listed with the others, its models as
`<provider>/<model>`, and fails over between its accounts like any keys.
Two plugins serving one provider: the second's is `<provider>-plugin`, or
`pansy plugin use <plugin> <provider>` picks which.

Coming from magpie: Models › Plugins offers to bring magpie's plugins
over (`pansy plugin import magpie`): they are installed again here and
their sign-ins copied; magpie's files are only read.

## Files

All under pansy's folder (`~/.config/pansy`, `PANSY_HOME`):

| Path | What it holds |
| :-- | :-- |
| `plugins.json` | The plugins added, in their order, on or off, with their options (0600) |
| `plugins/` | Their `package.json`, `bun.lock`, `node_modules` and the hosts' scripts (0700) |
| `plugin-auth.json` | The accounts plugins sign in to, as OpenCode keeps them (0600) |
| `plugin-providers.json` | The providers and models the plugins gave last, so pansy starts without them (0600) |
| `agent-plugins.json` | The agents plugins added, likewise |
| `account-ref.key` | The key a middleware's account references are made with (0600) |
| `bun/<version>/` | Bun, downloaded the first time a plugin is added and checked against Bun's release checksums (0700) |
| `cache/` | models.dev's list and the hub's lists (`plugin-registry.json`, `plugin-hub.json`) |

`PANSY_BUN` points at a Bun of your own instead.

## What a plugin may do

A plugin runs with your permissions, like any program you install: add only
what you trust. Packages are installed with `--ignore-scripts`. Middleware
and hooks see what agents send before it is masked.

Middleware fail open: a hook that throws or takes too long (250 ms on a
request, 50 ms on an event, 20 ms for a routing; 1 s to load) is passed
over and the request goes on; the failures are counted on the Plugins page.
Host B is restarted when it stops answering.

## Compatibility

The hosts speak magpie's protocol, and the names plugins see are magpie's:
`input.magpie` (also `input.pansy`), `package.json`'s `magpie` field,
`MAGPIE_*_PROXY`, `x-magpie-*` headers. An account reference is magpie's
(an HMAC of the account), so a middleware that keys state on it works
alike. The protocol is in `crates/pansy-plugins/host/README.md`.

Not yet: an agent's User-Agent passed on to a plugin's provider as magpie
maps it, and ending a host's processes as a Windows job (Unix kills the
process group).
