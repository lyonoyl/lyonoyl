#!/usr/bin/env bash
#
# Prepares a Linux machine for the engine probe: SSH in, and everything needed to build.
#
# Run it inside the guest, from an ordinary account with sudo — not as root, because the key it
# authorises goes into that account's profile.
#
#     bash prepare-linux-vm.sh [--key <file or key line>] [--nopasswd-sudo]
#
# Every step is checked after it is taken, and the script stops at the first one that did not
# hold. What it installs is what a Tauri build on Ubuntu 24.04 actually needed, and the versions
# it reports at the end are the ones a measurement on this machine belongs to.
#
# --nopasswd-sudo is off by default and weakens the machine on purpose: without it every command
# driven over SSH stops at a password prompt nobody is there to answer. Turn it on for a throwaway
# machine reached through a local port forward, and nowhere else.

set -euo pipefail

step() { printf '\n== %s\n' "$1"; }
ok() { printf '   ok: %s\n' "$1"; }
stop() { printf '   FAILED: %s\n' "$1" >&2; exit 2; }

key_line=""
nopasswd_sudo=0

while [ $# -gt 0 ]; do
  case "$1" in
    --key) key_line="${2:-}"; shift 2 ;;
    --nopasswd-sudo) nopasswd_sudo=1; shift ;;
    -h|--help) sed -n '3,20p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) stop "unknown argument: $1" ;;
  esac
done

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# --- Who is running this ----------------------------------------------------------------------

[ "$(id -u)" -ne 0 ] || stop "run this as the account you will log in as, not as root"
command -v sudo >/dev/null || stop "no sudo on this machine"

printf 'Preparing %s@%s\n' "$USER" "$(hostname)"
# Printed rather than left to be checked by hand: a disc mounted from an earlier run is served from
# a cache, and an old script is indistinguishable from a new one until something is compared.
if command -v sha256sum >/dev/null; then
  printf 'This script: %s\n' "$(sha256sum "${BASH_SOURCE[0]}" | cut -d' ' -f1)"
fi
if [ -r /etc/os-release ]; then
  # shellcheck disable=SC1091
  . /etc/os-release
  printf '%s, %s\n' "${PRETTY_NAME:-unknown}" "$(uname -m)"
  case "${VERSION_ID:-}" in
    24.04) ;;
    *) printf 'Note: measured on Ubuntu 24.04. Elsewhere the package names may differ.\n' ;;
  esac
fi

# --- The key ----------------------------------------------------------------------------------

step "Public key"

if [ -z "$key_line" ]; then
  found=$(find "$here" -maxdepth 1 -name '*.pub' -type f | head -2)
  count=$(printf '%s' "$found" | grep -c . || true)
  if [ "$count" -eq 1 ]; then
    key_line=$(cat "$found")
    ok "read from $(basename "$found")"
  elif [ "$count" -gt 1 ]; then
    stop "more than one .pub file sits beside this script; pass --key instead"
  else
    read -r -p "   Paste the public key line: " key_line
  fi
elif [ -f "$key_line" ]; then
  key_line=$(cat "$key_line")
fi

key_line="$(printf '%s' "$key_line" | tr -d '\r' | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//')"
case "$key_line" in
  ssh-ed25519\ *|ssh-rsa\ *|ecdsa-sha2-*|sk-ssh-ed25519@openssh.com\ *) ;;
  *) stop "that does not look like a public key line" ;;
esac
[ "$(printf '%s' "$key_line" | wc -l)" -eq 0 ] || stop "a public key is one line; this has more"
ok "${key_line:0:24}... accepted"

# --- The way in -------------------------------------------------------------------------------

step "SSH server"

sudo apt-get update -qq
sudo apt-get install -y -qq openssh-server >/dev/null

# The one that costs an afternoon: the service refuses to start with "sshd: no hostkeys
# available" and journalctl does not say so. `sshd -t` does.
if ! ls /etc/ssh/ssh_host_*_key >/dev/null 2>&1; then
  sudo ssh-keygen -A
  ok "host keys generated"
fi

sudo systemctl enable --now ssh >/dev/null 2>&1 || sudo systemctl enable --now sshd >/dev/null
sleep 1
ss -ltn 2>/dev/null | grep -qE '(^|[[:space:]])(0\.0\.0\.0|\*|\[::\]):22[[:space:]]' \
  || stop "nothing is listening on port 22"
ok "listening on 22"

step "Authorised key"

mkdir -p "$HOME/.ssh"
chmod 700 "$HOME/.ssh"
touch "$HOME/.ssh/authorized_keys"
if grep -qxF "$key_line" "$HOME/.ssh/authorized_keys"; then
  ok "already there, left alone"
else
  printf '%s\n' "$key_line" >> "$HOME/.ssh/authorized_keys"
  ok "added to ~/.ssh/authorized_keys"
fi
chmod 600 "$HOME/.ssh/authorized_keys"
grep -qxF "$key_line" "$HOME/.ssh/authorized_keys" || stop "the key did not reach the file"
[ "$(stat -c '%a' "$HOME/.ssh/authorized_keys")" = "600" ] || stop "the key file has the wrong mode"
ok "rights are what sshd insists on"

if [ "$nopasswd_sudo" -eq 1 ]; then
  step "Sudo without a password"
  rule="/etc/sudoers.d/$USER"
  printf '%s ALL=(ALL) NOPASSWD:ALL\n' "$USER" | sudo tee "$rule" >/dev/null
  sudo chmod 440 "$rule"
  # Checked before it can matter: a malformed file here takes sudo away from everybody.
  sudo visudo -cf "$rule" >/dev/null || { sudo rm -f "$rule"; stop "the sudoers file was malformed and was removed"; }
  ok "$USER may sudo without a password — this machine is now a throwaway one"
fi

# --- What a build needs -------------------------------------------------------------------------

step "Build dependencies"

sudo apt-get install -y -qq \
  build-essential curl wget file git rsync \
  libwebkit2gtk-4.1-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev patchelf \
  libgtk-3-dev libsoup-3.0-dev >/dev/null
pkg-config --exists webkit2gtk-4.1 || stop "webkit2gtk-4.1 is not installed"
ok "webkit2gtk $(pkg-config --modversion webkit2gtk-4.1) and the rest"

step "Rust"

if ! command -v rustc >/dev/null && [ ! -x "$HOME/.cargo/bin/rustc" ]; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --profile minimal --default-toolchain stable >/dev/null
fi
# shellcheck disable=SC1091
[ -r "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
command -v rustc >/dev/null || stop "rustc is not on the path after installing it"
ok "$(rustc --version)"

step "Node"

if ! command -v node >/dev/null || [ "$(node --version | cut -c2- | cut -d. -f1)" -lt 20 ]; then
  # NodeSource, because 24.04 ships Node 18 and the frontend needs newer. This pipes a remote
  # script into a root shell: acceptable on a machine built to be thrown away, nowhere else.
  curl -fsSL https://deb.nodesource.com/setup_22.x | sudo -E bash - >/dev/null
  sudo apt-get install -y -qq nodejs >/dev/null
fi
command -v node >/dev/null || stop "node is not on the path after installing it"
[ "$(node --version | cut -c2- | cut -d. -f1)" -ge 20 ] || stop "node is older than the frontend needs"
ok "node $(node --version), npm $(npm --version)"

# --- What this machine turned out to be ---------------------------------------------------------

step "What a measurement here belongs to"

printf '   webkit2gtk : %s\n' "$(pkg-config --modversion webkit2gtk-4.1)"
printf '   session    : %s\n' "${XDG_SESSION_TYPE:-not in a graphical session from here}"
printf '   portal     : %s\n' "$(command -v xdg-desktop-portal >/dev/null && echo present || echo MISSING)"
printf '   zenity     : %s\n' "$(command -v zenity >/dev/null && echo present || echo missing)"

cat <<EOF

Ready. Log in as '$USER'.

  In UTM, with the machine shut down: Network -> Emulated VLAN, then a port forward with
  Guest Port 22 and Host Port 2222. In that order: the host port is the number you type
  after -p, and it must be above 1024 or nothing binds and nothing says so.
  Not Shared Network: there the guest reaches the host and not the other way round.

  Then, from the host:
      ssh -i <private key> -p <host port> $USER@127.0.0.1

  The probe needs an open graphical session in this guest, but nobody has to look at it.
EOF
