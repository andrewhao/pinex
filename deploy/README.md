# Deploying to a Raspberry Pi

```sh
cargo build --release --target aarch64-unknown-linux-gnu
scp target/aarch64-unknown-linux-gnu/release/pinex pi:/usr/local/bin/
scp deploy/99-tonex.rules pi:/etc/udev/rules.d/
scp deploy/pinex.service pi:/etc/systemd/system/
ssh pi 'sudo udevadm control --reload && sudo systemctl enable --now pinex'
```

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
