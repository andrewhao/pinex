//! Capture raw bytes from a real pedal, for turning into fixtures.
//!
//! Deliberately does no parsing beyond framing-agnostic byte collection: the
//! whole point is to record what the pedal actually said, so the codec can be
//! judged against it rather than the recording being shaped by the codec.
//!
//! ```sh
//! cargo run -p pinex-device --example capture -- /dev/cu.usbmodem... hello out.bin
//! ```
//!
//! Requests: `hello`, `state`, `preset:<n>`, `preset-full:<n>`, `none` (listen only).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use pinex_device::transport::{Transport, TtyTransport};
use pinex_proto::message::{self, PresetDetail};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: capture <tty> <request> [outfile] [quiet-ms]");
        std::process::exit(2);
    }
    let tty = PathBuf::from(&args[1]);
    let request = args[2].clone();
    let outfile = args.get(3).cloned();
    let quiet_ms: u64 = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(1500);

    let payload = match request.as_str() {
        "hello" => Some(message::hello()),
        "state" => Some(message::request_state()),
        "none" => None,
        other => {
            let (kind, n) = other
                .split_once(':')
                .ok_or_else(|| format!("unknown request {other}"))?;
            let n: u8 = n.parse()?;
            let detail = match kind {
                "preset" => PresetDetail::Summary,
                "preset-full" => PresetDetail::Full,
                _ => return Err(format!("unknown request {other}").into()),
            };
            Some(message::request_preset(n, detail)?)
        }
    };

    let mut port = TtyTransport::open(&tty)?;
    eprintln!("opened {}", tty.display());

    if let Some(bytes) = &payload {
        eprintln!("TX {} bytes: {}", bytes.len(), hex(bytes));
        port.write_all(bytes)?;
    }

    // Read until the port has been quiet for `quiet_ms`. The transport's VTIME
    // gives a ~1 s tick even with no data, so this terminates on its own.
    let mut collected = Vec::new();
    let mut last_data = Instant::now();
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut buf = [0u8; 4096];

    while Instant::now() < deadline {
        let n = port.read(&mut buf)?;
        if n > 0 {
            collected.extend_from_slice(&buf[..n]);
            last_data = Instant::now();
        } else if !collected.is_empty() && last_data.elapsed() >= Duration::from_millis(quiet_ms) {
            break;
        } else if collected.is_empty() && last_data.elapsed() >= Duration::from_secs(5) {
            eprintln!("no reply after 5s");
            break;
        }
    }

    eprintln!("RX {} bytes", collected.len());
    println!("{}", hex(&collected));

    if let Some(path) = outfile {
        std::fs::File::create(Path::new(&path))?.write_all(&collected)?;
        eprintln!("wrote {path}");
    }
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}
