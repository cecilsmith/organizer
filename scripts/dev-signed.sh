#!/bin/sh
# Build and run the dev app with a code signature that stays the same.
#
# `cargo build` leaves the binary ad-hoc signed, and an ad-hoc signature is
# derived from the binary itself — so every rebuild looks like a different app
# to macOS, and the Keychain asks again for any token Organizer holds. Clicking
# "Always Allow" only ever covers the build that asked. Signing with a
# certificate that outlives the build makes that click stick.
#
# One-time setup: Keychain Access > Certificate Assistant > Create a
# Certificate…, name it "Organizer Dev", identity type "Self Signed Root",
# certificate type "Code Signing". Then run this instead of `npm run tauri dev`.
set -eu

identity="${ORGANIZER_SIGN_IDENTITY:-Organizer Dev}"
root=$(cd "$(dirname "$0")/.." && pwd)
bin="$root/src-tauri/target/debug/organizer"
url="http://localhost:1420"

if ! security find-certificate -c "$identity" >/dev/null 2>&1; then
  echo "No code-signing certificate named \"$identity\" in your keychain." >&2
  echo "Create one: Keychain Access > Certificate Assistant > Create a Certificate…," >&2
  echo "identity type \"Self Signed Root\", certificate type \"Code Signing\"." >&2
  echo "Or set ORGANIZER_SIGN_IDENTITY to one you already have." >&2
  exit 1
fi

# A debug build loads the UI from vite, so it has to be up first.
up() { curl -fsS --max-time 1 "$url" >/dev/null 2>&1; }
if ! up; then
  echo "Starting the vite dev server…"
  (cd "$root" && npm run dev >/dev/null 2>&1) &
  vite=$!
  trap 'kill "$vite" 2>/dev/null || true' EXIT INT TERM
  waited=0
  until up; do
    sleep 1
    waited=$((waited + 1))
    if [ "$waited" -ge 60 ]; then
      echo "vite didn't come up on $url — run \`npm run dev\` yourself to see why." >&2
      exit 1
    fi
  done
fi

(cd "$root/src-tauri" && cargo build)
codesign --force --sign "$identity" "$bin"
echo "Signed with \"$identity\"; macOS will remember your Keychain choice across rebuilds."
"$bin"
