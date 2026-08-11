# Deploying to a Raspberry Pi

One command:

```sh
./deploy/deploy.sh pi@rpi3.local
```

It detects whether the Pi runs 32- or 64-bit userland and builds the matching
target — a Pi 3 can be either, and the wrong binary fails with a confusing
"cannot execute binary file".

## Toolchain

Cross-compiling from macOS needs a linker for the target. `cargo-zigbuild` uses
zig's bundled one, which needs no Docker daemon:

```sh
brew install zig
cargo install cargo-zigbuild
```

The design doc named `cross` + Docker Desktop. Either works; zigbuild was chosen
because it has no daemon to start and builds in ~30s.

## Build the Pi image without the simulator

The simulator is a default feature so `cargo test` exercises it. Leave it out
of the deployed binary:

```sh
cargo build --release --no-default-features -p pinex
```

## If the handshake misbehaves

Check ModemManager first. It probes CDC-ACM devices and sends AT commands at
them; the udev rule sets `ID_MM_DEVICE_IGNORE` to stop that. Confirm with
`udevadm info -q property -n /dev/tonex | grep MM`.

Second: permissions. The service runs as `pinex` in `dialout`; the rule sets
the group to match.

## If the pedal stops answering entirely

Observed on firmware 1.3.17: after a burst of requests the pedal can stop
replying to everything, including `Hello`, while still enumerating on USB and
presenting its tty. Reads return zero bytes and writes succeed. DTR/RTS are
already asserted, so that is not the cause.

A power cycle of the pedal clears it. Worth pacing bulk preset fetches if it
recurs — see the open questions in `docs/plans/README.md`.

## Verified on hardware

A Pi 3 (64-bit, kernel 6.18) with the pedal on `/dev/ttyACM0`, firmware 1.3.17:

- Handshake, all twenty preset names and colours, over USB from the Pi
- A preset change through the browser UI, and via `verify_write` — both PASS,
  both restored the original preset
- Running as a systemd **user** service with `NRestarts=0`, which is the direct
  evidence the `/dev/null` stdin bug is gone; before the fix this crash-looped

Enabled at boot with `loginctl enable-linger pi`, so the user service starts
without anyone logging in.

## If you have root

The unprivileged install addresses `/dev/ttyACM0` directly. For the stable
`/dev/tonex` name and a system-wide service:

```sh
sudo cp deploy/99-tonex.rules /etc/udev/rules.d/
sudo udevadm control --reload && sudo udevadm trigger --subsystem-match=tty
```

Then re-run `deploy.sh`, which takes the privileged path automatically once
passwordless sudo is available.
