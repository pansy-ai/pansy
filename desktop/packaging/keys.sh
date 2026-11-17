#!/usr/bin/env bash
# Makes the keys pansy's releases are signed with, once (docs/release.md):
#
#   keys/root.secret      certifies release keys; keep it offline
#   keys/release.secret   signs latest.json and the window's packages; the
#                         release workflow's PANSY_RELEASE_KEY
#   packaging/release.cert   the root's word for the release key, a year
#
# and writes what the window trusts into nsh.json's "updater": the release
# key for the manifest (`publicKey`), the root for packages
# (`packages.roots`). keys/ is not in git. Every pansy out there trusts
# these keys: new ones reach it only with an update signed by the old.
#
#   keys.sh          the keys, when there are none
#   keys.sh renew    a new certificate for the release key (it lasts a year)
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
export PATH="$HOME/.cargo/bin:$HOME/.nsh/bin:$PATH"

id=$(python3 -c 'import json; print(json.load(open("nsh.json"))["id"])')
certify() {
  nsh certify --root keys/root.secret --key keys/release.pub --apps "$id" --days 365 --out packaging/release.cert
}

if [[ ${1:-} == renew ]]; then
  [[ -f keys/root.secret ]] || { echo "keys.sh: keys/root.secret is not here: bring the root key back to renew" >&2; exit 1; }
  certify
  exit
fi
if [[ -e keys/root.secret || -e keys/release.secret ]]; then
  echo "keys.sh: keys/ has keys already; pansy as released trusts those" >&2
  exit 1
fi

umask 077
mkdir -p keys
nsh keygen keys/root
nsh keygen keys/release
certify
chmod 644 packaging/release.cert

# nsh.json keeps its layout: only "updater" is written again
python3 - "$(cat keys/release.pub)" "$(cat keys/root.pub)" <<'PY'
import json, re, sys
text = open("nsh.json").read()
updater = json.loads(text)["updater"]
updater.pop("github", None)
updater.setdefault("url", "https://releases.pansy.dev/latest.json")
updater["publicKey"] = sys.argv[1]
updater["packages"] = {"roots": [sys.argv[2]]}
block = json.dumps(updater, indent=2).replace("\n", "\n  ")
text, n = re.subn(r'"updater": \{.*?\n  \}', lambda _: '"updater": ' + block, text, count=1, flags=re.S)
assert n == 1, 'nsh.json has no "updater"'
open("nsh.json", "w").write(text)
PY
echo "nsh.json trusts the new keys. Keep keys/root.secret offline; keys/release.secret is the workflow's PANSY_RELEASE_KEY."
