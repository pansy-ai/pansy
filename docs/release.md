# Releases and updates

pansy's window updates itself from one place: a Cloudflare R2 bucket behind
`https://releases.pansyai.dev` (`desktop/nsh.json`, `"updater"`). A release is
files put there; nothing else serves them.

```
latest.json, latest.json.sig          the feed, signed by the update key
v0.2.0/pansy-0.2.0-linux-x86_64.tar.gz
v0.2.0/pansy-0.2.0-linux-aarch64.tar.gz
v0.2.0/pansy-0.2.0-macos-aarch64.tar.gz
v0.2.0/pansy-0.2.0-macos-x86_64.tar.gz
v0.2.0/pansy-0.2.0-windows-x86_64.zip
v0.2.0/dev.pansy.app-7.nshpkg         the window alone, build 7
v0.2.0/notes.zh-Hans.md               what changed, in Chinese
beta/latest.json, beta/…              the same feed, for those on beta
index.html                            the download page
```

## Two kinds of release

| | pansy itself | The window alone |
| :-- | :-- | :-- |
| What changed | anything | only the TSX (`desktop/App.tsx`, `desktop/ui/`) |
| What is published | an archive for each system | one signed package (`.nshpkg`) for all of them |
| What the user's pansy does | on a click: downloads its system's archive, replaces itself, restarts | downloads the package when it finds it; a click restarts into it, as does opening pansy the next time; the program stays |
| Number | a new version, and the next build | the same version, the next build |
| Tag | `pansy-v0.2.0` | `pansy-v0.2.0-ui.7` |

Both show the same Update pill (`desktop/ui/Update.tsx`): a look a little
after the start and every six hours, a click, a wait for the gateway's
requests to finish, a restart. The window alone is named by its build,
"0.2.0 (7)", and is downloaded by the look that finds it: its pill says
Restart. As magpie's, the pill in the title bar has a × under the
pointer that puts it away for three days (Settings › About still offers the
update), and what the release says changed is shown once pansy has
restarted into it (What's new, `desktop/ui/WhatsNew.tsx`), and again from
Settings › About.

A package of the window runs on the Rust of the pansy that is installed, so:

- it is released only **for the version that is out** (the feed's
  `version`), and a pansy takes it only when it is that version; an older
  pansy is offered the newer pansy instead;
- `publish.sh ui` refuses when Rust changed since that version's tag
  (`desktop/native`, `desktop/runner`, `crates/`, the lock files);
- the program starts it on trial (`runner/src/main.rs`, NSH's
  `updates::launch`): one that does not run ten seconds without a fatal
  error is put away, and the window shipped with the program runs again.

The build (`nsh.json` `"build"`) grows with every release of either kind:
a pansy runs a package only when its build is newer than its own.

## Beta

Settings › About › Update channel is stable or beta. A pansy on beta asks
`beta/latest.json` (`nsh.json`'s `updater.channels`) for a newer pansy; the
name is all the window passes, the address is the package's. A version with
a suffix, `0.3.0-beta.1` (`bump.sh 0.3.0-beta.1`, tag `pansy-v0.3.0-beta.1`),
is written to the beta feed alone. Any other is written to both, unless
beta is already ahead of it, so those on beta get `0.3.0` after its betas.
Back on stable, a pansy keeps its beta until a stable version is newer.

The window alone is released for the stable pansy only: NSH reads packages
from the main feed, and a package is for one version.

## When an update goes in

| | When it is downloaded | When it goes in |
| :-- | :-- | :-- |
| The window alone | by the look that finds it | a click on Restart, or the next time pansy starts |
| pansy itself | by the look that finds it | a click on Restart, or as pansy quits (NSH's `installOnQuit`) |
| pansy itself, with Settings › About › Update when pansy quits off | on a click on Update | then, once the gateway's requests have finished |

One left to the quit that never came (pansy was killed) is offered again.

## Releasing

```sh
cd desktop
packaging/bump.sh 0.2.0     # pansy itself; or `packaging/bump.sh` for the window alone
git commit -am "pansy 0.2.0"
git tag pansy-v0.2.0 && git push origin pansy-main pansy-v0.2.0
```

What changed goes in `packaging/notes.md` before the tag (headings, bullets
and lines; written anew for each release), and in `notes.zh-Hans.md` beside
it in Chinese: the feed's notes, which a pansy asks for in its window's
language for What's new. `bump.sh` prints the tag. The tag starts `.github/workflows/release.yml`:
`packaging/build.sh` on each system (for pansy itself), then
`packaging/publish.sh`, which writes the feed with `nsh update-manifest`,
signs it, and uploads the archives first and the feed last.

By hand, with the archives of every system gathered in `dist/`:

```sh
packaging/build.sh                            # on each system
PANSY_PUBLISH_DRY=1 packaging/publish.sh      # dist/release, as the bucket would be
packaging/publish.sh                          # or `publish.sh ui`
```

## Keys

`packaging/keys.sh` made them once; `desktop/keys/` is not in git.

| File | What it signs | Where it lives |
| :-- | :-- | :-- |
| `keys/root.secret` | the release key's certificate (`packaging/release.cert`, a year) | offline |
| `keys/release.secret` | the window's packages | the workflow's `PANSY_RELEASE_KEY` |
| `keys/update.secret` | `latest.json` | the workflow's `PANSY_UPDATE_KEY` |

`nsh.json` holds what every pansy trusts: the update key for the feed
(`publicKey`) and the root for packages (`packages.roots`). They cannot be
changed for a pansy already installed except by an update the old keys
sign, so set them before the first release and keep the secrets. The
release key is not in any pansy: one that leaked is replaced by a new one
the root certifies, with no new pansy. `packaging/keys.sh renew` writes a
new certificate when the year is up. `publish.sh ui` checks each package as
a pansy will (`nsh verify`, against `nsh.json`'s roots) before it goes up.

## Cloudflare, once

1. An R2 bucket, `pansy-releases` (`wrangler r2 bucket create pansy-releases`).
2. A custom domain on it, `releases.pansyai.dev` (the bucket's Settings ›
   Custom Domains). The updater needs HTTPS and this exact address: it is in
   every pansy released, so choose it before the first release and change
   `nsh.json`'s `updater.url` to match.
3. An API token that may edit the bucket's objects (Workers R2 Storage:
   Edit), and the account's id.
4. In the GitHub repository, an environment `release` with the secrets
   `PANSY_RELEASE_KEY`, `PANSY_UPDATE_KEY`, `CLOUDFLARE_API_TOKEN` and
   `CLOUDFLARE_ACCOUNT_ID`
   (and the variable `PANSY_RELEASE_BUCKET` for another bucket name).
5. R2 serves no index for `/`: link to `https://releases.pansyai.dev/index.html`,
   or add a redirect rule from `/` to it.

## When a release is bad

| What | What to do |
| :-- | :-- |
| The window's package fails as it starts | nothing: pansy puts it away and runs the window it had; release a fixed one, a build higher |
| The window's package runs but is wrong | release a fixed one, a build higher: a pansy that ran a build takes none older |
| pansy itself is wrong | release a higher version: the feed never offers an older one |
| The release key leaked | `nsh keygen` a new one, `keys.sh renew`, release again |

## Trying a release on this computer

NSH takes a feed over plain HTTP from `127.0.0.1` alone. With `nsh.json`'s
`updater.url` set to `http://127.0.0.1:8787/latest.json` for the trial (not
committed): `packaging/build.sh`, `PANSY_PUBLISH_DRY=1 packaging/publish.sh`,
`python3 -m http.server 8787` in a copy of `dist/release`, and the pansy
unpacked from `dist/` finds there what is published next the same way.

## What is not done

- The archives are not installers: Linux has `install.sh`, macOS's
  `Pansy.app` is signed ad hoc and not notarized (Gatekeeper asks; the
  download page says how to clear the quarantine;
  `PANSY_CODESIGN_IDENTITY` signs with a Developer ID), Windows is a folder
  with `pansy.exe`.
- Staged rollouts and mirrors. What's new has the notes of
  one release, not of every release since the one installed.
