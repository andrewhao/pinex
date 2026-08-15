#!/usr/bin/env bash
#
# Build Pinex for a Raspberry Pi and install it.
#
#   ./deploy/deploy.sh [user@host]        # default pi@rpi3.local
#
# Detects the Pi's architecture rather than assuming: a Pi 3 runs 64-bit
# Raspberry Pi OS on some cards and 32-bit on others, and the wrong binary
# fails with a confusing "cannot execute binary file".
#
# Idempotent. Safe to re-run after a code change.

set -euo pipefail

TARGET_HOST="${1:-pi@rpi3.local}"
REMOTE_BIN=/usr/local/bin/pinex

say() { printf '\n=== %s\n' "$*"; }

say "checking $TARGET_HOST"
if ! ssh -o ConnectTimeout=10 -o BatchMode=yes "$TARGET_HOST" true 2>/dev/null; then
  echo "cannot reach $TARGET_HOST over ssh." >&2
  echo "Is the Pi powered on and on the network? Try: ping ${TARGET_HOST#*@}" >&2
  exit 1
fi

BITS=$(ssh "$TARGET_HOST" 'getconf LONG_BIT')
case "$BITS" in
  64) RUST_TARGET=aarch64-unknown-linux-gnu ;;
  32) RUST_TARGET=armv7-unknown-linux-gnueabihf ;;
  *)  echo "unexpected word size '$BITS'" >&2; exit 1 ;;
esac
say "Pi is ${BITS}-bit -> $RUST_TARGET"

# `--no-default-features` leaves the PTY simulator out of the shipped binary.
say "building"
rustup target add "$RUST_TARGET" >/dev/null 2>&1 || true
if command -v cargo-zigbuild >/dev/null 2>&1; then
  # --no-default-features drops the PTY simulator; --features hat adds the
  # ST7735S panel and the joystick/keys. A missing HAT is reported and skipped
  # at runtime, so this binary is still correct on a bare Pi.
  cargo zigbuild --release -p pinex --no-default-features --features hat --target "$RUST_TARGET"
else
  echo "cargo-zigbuild not found. Install with:" >&2
  echo "  brew install zig && cargo install cargo-zigbuild" >&2
  exit 1
fi

BIN="target/$RUST_TARGET/release/pinex"

# Root gets the tidy install: /usr/local/bin, a udev symlink, a system service.
# Without passwordless sudo we install into the user's home instead, which works
# because the login user is already in `dialout` and can open the tty directly.
# Same binary, same behaviour — only the paths and the device name differ.
if ssh "$TARGET_HOST" 'sudo -n true' >/dev/null 2>&1; then
  PRIVILEGED=yes
else
  PRIVILEGED=no
  say "no passwordless sudo — installing to the user's home instead"
fi

say "installing $(du -h "$BIN" | cut -f1) binary (privileged=$PRIVILEGED)"

if [ "$PRIVILEGED" = no ]; then
  # Remember whether it was running, so it can be put back. Leaving a stage box
  # stopped because a deploy stopped it has happened twice, and both times it
  # looked like a bug in the thing being deployed: the panel holds its last
  # frame, so a dead service is indistinguishable from a frozen one.
  WAS_ACTIVE=$(ssh "$TARGET_HOST" 'systemctl --user is-active pinex 2>/dev/null || true')
  ssh "$TARGET_HOST" 'systemctl --user stop pinex 2>/dev/null || true; pkill -x pinex 2>/dev/null || true'
  ssh "$TARGET_HOST" 'mkdir -p ~/bin ~/.config/systemd/user'
  scp -q "$BIN" "$TARGET_HOST:/tmp/pinex.new"
  scp -q deploy/pinex-user.service "$TARGET_HOST:/tmp/"
  ssh "$TARGET_HOST" "bash -s" <<'REMOTE'
set -euo pipefail
install -m 0755 /tmp/pinex.new ~/bin/pinex
install -m 0644 /tmp/pinex-user.service ~/.config/systemd/user/pinex.service
rm -f /tmp/pinex.new /tmp/pinex-user.service
systemctl --user daemon-reload
echo "--- installed to ~/bin/pinex"
id -nG | tr ' ' '\n' | grep -qx dialout && echo "user is in dialout: can open the tty" \
  || echo "WARNING: user is NOT in dialout; run: sudo usermod -aG dialout $USER"
ls -l /dev/ttyACM* 2>/dev/null || echo "no /dev/ttyACM* — plug the pedal into the Pi"
REMOTE
  # Put it back if it was up. A deploy should leave the box in the state it
  # found it, not in the state that was convenient halfway through.
  if [ "$WAS_ACTIVE" = active ]; then
    say "restarting the service it was running before"
    ssh "$TARGET_HOST" 'systemctl --user start pinex'
    sleep 3
    STATE=$(ssh "$TARGET_HOST" 'systemctl --user is-active pinex')
    echo "  service is now: $STATE"
    [ "$STATE" = active ] || echo "  WARNING: it did not come back up" >&2
  else
    say "the service was not running before, so it has been left stopped"
  fi

  say "installed (unprivileged). Run it with:"
  echo "  ssh -t $TARGET_HOST '~/bin/pinex /dev/ttyACM0'"
  echo
  echo "For the tidy install (/dev/tonex symlink + system service), run once on the Pi:"
  echo "  sudo cp /tmp/99-tonex.rules /etc/udev/rules.d/ && sudo udevadm control --reload"
  exit 0
fi

# Stop first: a running service holds the binary open.
ssh "$TARGET_HOST" 'sudo systemctl stop pinex 2>/dev/null || true'

scp -q "$BIN" "$TARGET_HOST:/tmp/pinex.new"
scp -q deploy/99-tonex.rules "$TARGET_HOST:/tmp/"
scp -q deploy/pinex.service "$TARGET_HOST:/tmp/"

ssh "$TARGET_HOST" "bash -s" <<'REMOTE'
set -euo pipefail
sudo install -m 0755 /tmp/pinex.new /usr/local/bin/pinex
sudo install -m 0644 /tmp/99-tonex.rules /etc/udev/rules.d/99-tonex.rules
sudo install -m 0644 /tmp/pinex.service /etc/systemd/system/pinex.service
rm -f /tmp/pinex.new /tmp/99-tonex.rules /tmp/pinex.service

# The service runs as its own user, in dialout so it can open the tty.
id pinex >/dev/null 2>&1 || sudo useradd --system --no-create-home --shell /usr/sbin/nologin pinex
sudo usermod -aG dialout pinex

sudo udevadm control --reload-rules
sudo udevadm trigger --subsystem-match=tty
sudo systemctl daemon-reload

echo "--- pedal present?"
if [ -e /dev/tonex ]; then
  ls -l /dev/tonex
else
  echo "no /dev/tonex — plug the pedal into the Pi (udev creates the symlink)"
  ls -l /dev/ttyACM* 2>/dev/null || true
fi

echo "--- ModemManager (the classic CDC-ACM footgun)"
if systemctl is-active --quiet ModemManager 2>/dev/null; then
  echo "ModemManager is RUNNING. The udev rule sets ID_MM_DEVICE_IGNORE, but if"
  echo "the handshake misbehaves, suspect it first: sudo systemctl disable --now ModemManager"
else
  echo "not running — good"
fi
REMOTE

say "installed. Start it with:"
echo "  ssh $TARGET_HOST 'sudo systemctl enable --now pinex && sudo journalctl -fu pinex'"
echo
echo "Or run it in the foreground first, which is the better first test:"
echo "  ssh -t $TARGET_HOST '/usr/local/bin/pinex'"
