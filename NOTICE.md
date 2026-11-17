# Notices

pansy is a Rust rewrite of [magpie](https://github.com/yetone/magpie) by
yetone (MIT). The features, the provider presets and the way agents are wired
follow magpie's; the code is new. magpie's history stays in this
repository's git history (the `main` branch).

The window draws [Lucide](https://lucide.dev) icons (ISC,
`desktop/ui/icons/LICENSE`) and runs on [Native Shell](https://nsh.dev)
0.5.1 (Apache-2.0).

The provider and agent logos in `desktop/ui/logos/` are
[LobeHub's icons](https://github.com/lobehub/lobe-icons) (MIT), as magpie
ships them, and Crush's own mark (`crush.png`, from magpie). The marks belong
to their owners; pansy draws them only to name their products.

`crates/pansy-core/src/subscription/codex_prompt.md` is Codex CLI's prompt
from [openai/codex](https://github.com/openai/codex) (Apache-2.0), as magpie
carries it: the instructions a ChatGPT account's requests are sent with when
the account's model list gives none.

`crates/pansy-plugins/host/host.js` and `pi.js` are magpie's plugin hosts
(MIT), taken as they are so magpie's community plugins run unchanged;
`mw-host.js`, which runs middleware and agent plugins, is pansy's own.
`crates/pansy-plugins/data/registry.json` is a copy of
[magpie-community](https://github.com/yetone/magpie-community)'s registry
(MIT), shipped for when pansy's plugin hub (hub.pansyai.dev), whose
curated list began from it, can't be reached. The plugins themselves are
their authors', installed from npm or their repositories when added; pansy
ships none. Bun is downloaded from its releases when a plugin needs it.
