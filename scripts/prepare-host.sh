#!/usr/bin/env bash
#
# Prepares the machine you work on, so a virtual machine can be prepared from it.
#
# That machine is a Mac, which is why the disc at the end is built with hdiutil. Everything
# else here is portable, and the two guest scripts do not care what host they came from.
#
#     bash scripts/prepare-host.sh --for windows
#     bash scripts/prepare-host.sh --for linux
#     bash scripts/prepare-host.sh --for both
#
# It makes the key those machines will trust — once, and reused after that — and gathers what has
# to travel into the guest: the script for that platform, and the public key beside it, so the
# script finds the key on its own and nobody types it. On macOS it also builds an ISO of them,
# which is the one way into a fresh Windows guest that needs nothing installed there first.
#
# Nothing here is secret and nothing here is committed. The private key stays where it is made.

set -euo pipefail

step() { printf '\n== %s\n' "$1"; }
ok() { printf '   ok: %s\n' "$1"; }
stop() { printf '   FAILED: %s\n' "$1" >&2; exit 2; }

want="both"
key="$HOME/.ssh/id_lyonoyl_vm"
out="$HOME/Desktop"

while [ $# -gt 0 ]; do
  case "$1" in
    --for) want="${2:-}"; shift 2 ;;
    --key) key="${2:-}"; shift 2 ;;
    --out) out="${2:-}"; shift 2 ;;
    -h|--help) sed -n '3,15p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) stop "unknown argument: $1" ;;
  esac
done

case "$want" in
  linux|windows|both) ;;
  *) stop "--for takes linux, windows or both" ;;
esac

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
[ -d "$out" ] || stop "no such directory: $out"

# --- The key ------------------------------------------------------------------------------------

step "Key"

if [ -f "$key" ]; then
  ok "reusing $key"
else
  # No passphrase: these machines are driven unattended, and a passphrase means a human at every
  # connection. The key opens throwaway guests reached through a local port forward, nothing else.
  ssh-keygen -t ed25519 -N '' -C 'lyonoyl platform vms' -f "$key" >/dev/null
  ok "made $key"
fi
[ -f "$key.pub" ] || stop "there is no public half at $key.pub"
printf '   %s\n' "$(cat "$key.pub")"

# --- What travels -------------------------------------------------------------------------------

made=()

check_it() {         # check_it <platform> <script> <fingerprint>
  if [ "$1" = "windows" ]; then
    cat <<TXT
If this disc has been attached before, Windows may still be serving the old one from a
cache. The run prints the script's fingerprint near the top, on the line that starts
"This script:". It should read:

    $3

If it does not, eject the disc in UTM and attach it again.
TXT
  else
    cat <<TXT
If this disc has been mounted before, the guest may still be serving the old one from a
cache. The run prints the script's fingerprint near the top, on the line that starts
"This script:". It should read:

    $3

If it does not, eject the disc and mount it again.
TXT
  fi
}

# One fingerprint, however this machine spells the command for it. It travels with the script so
# that "is the disc in the guest the current one" has an answer instead of a guess: a disc left
# attached from a previous run is read from a cache, and the old script looks exactly like the new.
fingerprint() {
  if command -v shasum >/dev/null; then shasum -a 256 "$1" | cut -d' ' -f1
  elif command -v sha256sum >/dev/null; then sha256sum "$1" | cut -d' ' -f1
  else echo "unavailable"
  fi
}

stage_for() {
  local platform="$1" script="$2" howto="$3"
  local dir="$out/lyonoyl-vm-$platform"

  rm -rf "$dir"
  mkdir -p "$dir"
  cp "$here/$script" "$dir/"
  cp "$key.pub" "$dir/"

  # Windows opens a cmd window by default, where a .ps1 cannot be typed, and the disc is not always
  # drive D. The launcher removes both: it works from cmd, from PowerShell and from a double-click,
  # and it finds the script by its own path rather than by a letter.
  if [ "$platform" = "windows" ]; then
    cp "$here/prepare-windows-vm.cmd" "$dir/"
    [ -f "$dir/prepare-windows-vm.cmd" ] || stop "the launcher did not reach $dir"
  fi
  printf '%s\n\n%s\n' "$howto" "$(check_it "$platform" "$script" "$(fingerprint "$here/$script")")" \
    > "$dir/READ-ME-FIRST.txt"

  [ -f "$dir/$script" ] || stop "the script did not reach $dir"
  [ -f "$dir/$(basename "$key.pub")" ] || stop "the public key did not reach $dir"
  ok "$dir"
  printf '   %s  %s\n' "$(fingerprint "$here/$script")" "$script"
  made+=("$dir")
}

if [ "$want" = "linux" ] || [ "$want" = "both" ]; then
  step "For the Linux guest"
  stage_for linux prepare-linux-vm.sh \
"In the Linux guest, open this disc in the file manager, right-click inside it and choose
'Open in Terminal'. Then:

    bash prepare-linux-vm.sh --nopasswd-sudo

Say 'bash' rather than './': a mounted disc is usually read-only and marked no-exec, so
running the file directly fails for a reason that has nothing to do with the script.

The .pub file beside it is found on its own, so no key has to be typed. Leave off
--nopasswd-sudo if a person will be at the keyboard for every command."
fi

if [ "$want" = "windows" ] || [ "$want" = "both" ]; then
  step "For the Windows guest"
  stage_for windows prepare-windows-vm.ps1 \
"In the Windows guest, open this disc and RIGHT-CLICK prepare-windows-vm.cmd,
then choose 'Run as administrator'. That is the whole of it.

Nothing else has to be typed, and the drive letter does not matter: the launcher finds
everything by its own path. The .pub file beside it is found on its own too. Do this while the
guest still has network: the OpenSSH server may be fetched from Windows Update."
fi

# --- A way in that needs nothing installed in the guest ------------------------------------------

carry_in="Get the folder above into it — share a directory, or paste the script into a terminal there."

if command -v hdiutil >/dev/null; then
  step "ISO"
  for dir in "${made[@]}"; do
    iso="$dir.iso"
    rm -f "$iso"
    hdiutil makehybrid -iso -joliet -quiet -o "$iso" "$dir" >/dev/null
    [ -f "$iso" ] || stop "the image was not written: $iso"
    ok "$iso"
  done
  printf '\n   Attach it in UTM as a CD/DVD drive. A fresh guest can read an ISO with nothing\n'
  printf '   installed in it, which drag-and-drop cannot.\n'
  carry_in="Attach the .iso above in UTM as a CD/DVD drive, and open it in the guest."
else
  printf '\n   No hdiutil here, so no disc was built — it is a macOS tool, and this is not macOS.\n'
  printf '   The folder is ready either way. To make a disc of it by hand:\n\n'
  for dir in "${made[@]}"; do
    printf '       xorriso -as mkisofs -J -o %s.iso %s\n' "$dir" "$dir"
  done
  printf '\n   Or carry the folder in some other way: a shared directory, or pasting the script\n'
  printf '   into a terminal in the guest.\n'
fi

cat <<EOF

Next, in this order:

  1. Create the guest in UTM and install the system. Log in graphically.
  2. $carry_in
  3. Run the script inside the guest. It ends by telling you the account to log in as.
  4. Shut the guest down. In UTM: Network -> Emulated VLAN, then a port forward with
     Guest Port 22 and Host Port 2222 for Linux, 2223 for Windows. In that order, and the
     host port above 1024 — below that nothing binds and nothing reports it.
  5. Start it again and log in graphically. Leave that session open: the probe opens a
     window, and a window needs a session. Nobody has to look at it.
  6. From here:  ssh -i $key -p <host port> <account>@127.0.0.1

docs/platforms/virtual-machines.md is the same thing at length, with what each step is for.
EOF
