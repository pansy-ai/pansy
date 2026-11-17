# Releases and updates

pansy's window updates itself from one place: a Cloudflare R2 bucket behind
`https://releases.pansy.dev` (`desktop/nsh.json`, `"updater"`). A release is
files put there; nothing else serves them.

```
latest.json, latest.json.sig          the feed, signed by the release key
v0.2.0/pansy-0.2.0-linux-x86_64.tar.gz
v0.2.0/pansy-0.2.0-linux-aarch64.tar.gz
v0.2.0/pansy-0.2.0-macos-aarch64.tar.gz
v0.2.0/pansy-0.2.0-macos-x86_64.tar.gz
v0.2.0/pansy-0.2.0-windows-x86_64.zip
v0.2.0/dev.pansy.app-7.nshpkg         the window alone, build 7
index.html                            the download page
```

## Two kinds of release

| | pansy itself | The window alone |
| :-- | :-- | :-- |
| What changed | anything | only the TSX (`desktop/App.tsx`, `desktop/ui/`) |
| What is published | an archive for each system | one signed package (`.nshpkg`) for all of them |
| What the user's pansy does | downloads its system's archive, replaces itself, restarts | downloads the package, restarts into it; the program stays |
| Number | a new version, and the next build | the same version, the next build |
| Tag | `pansy-v0.2.0` | `pansy-v0.2.0-ui.7` |

Both show the same Update pill (`desktop/ui/Update.tsx`): a look a little
after the start and every six hours, a click to download, a wait for the
gateway's requests to finish, a restart. The window alone is named by its
build: "0.2.0 (7)".

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

## Releasing

```sh
cd desktop
packaging/bump.sh 0.2.0     # pansy itself; or `packaging/bump.sh` for the window alone
git commit -am "pansy 0.2.0"
git tag pansy-v0.2.0 && git push origin pansy-main pansy-v0.2.0
```

`bump.sh` prints the tag. The tag starts `.github/workflows/release.yml`:
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
| `keys/release.secret` | `latest.json` and the window's packages | the workflow's `PANSY_RELEASE_KEY` |

`nsh.json` holds what every pansy trusts: the release key for the feed
(`publicKey`) and the root for packages (`packages.roots`). They cannot be
changed for a pansy already installed except by an update the old keys
sign, so set them before the first release and keep both secrets.
`packaging/keys.sh renew` writes a new certificate when the year is up.

## Cloudflare, once

1. An R2 bucket, `pansy-releases` (`wrangler r2 bucket create pansy-releases`).
2. A custom domain on it, `releases.pansy.dev` (the bucket's Settings ›
   Custom Domains). The updater needs HTTPS and this exact address: it is in
   every pansy released, so choose it before the first release and change
   `nsh.json`'s `updater.url` to match.
3. An API token that may edit the bucket's objects (Workers R2 Storage:
   Edit), and the account's id.
4. In the GitHub repository, an environment `release` with the secrets
   `PANSY_RELEASE_KEY`, `CLOUDFLARE_API_TOKEN` and `CLOUDFLARE_ACCOUNT_ID`
   (and the variable `PANSY_RELEASE_BUCKET` for another bucket name).
5. R2 serves no index for `/`: link to `https://releases.pansy.dev/index.html`,
   or add a redirect rule from `/` to it.

## What is not done

- The archives are not installers: Linux has `install.sh`, macOS's
  `Pansy.app` is signed ad hoc and not notarized (Gatekeeper asks; the
  download page says how to clear the quarantine;
  `PANSY_CODESIGN_IDENTITY` signs with a Developer ID), Windows is a folder
  with `pansy.exe`.
- Channels (beta), staged rollouts, mirrors and What's new.
