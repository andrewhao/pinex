//! Pinex binary.
//!
//! The device, display, input and web layers are not built yet (M1–M3). Until
//! they are, this dumps the frames `pinex-proto` generates, so the bytes we
//! intend to put on the wire can be eyeballed — and diffed against a USB capture
//! — before anything is ever transmitted to a pedal.

use pinex_proto::message::PresetDetail;
use pinex_proto::{hello, request_preset, request_state, USB_PID, USB_VID};

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn main() {
    println!("pinex {} — protocol scaffolding", env!("CARGO_PKG_VERSION"));
    println!("target device: USB {USB_VID:#06x}:{USB_PID:#06x} (Tonex ONE, CDC-ACM)");
    println!();
    println!("Outbound frames (nothing is transmitted; no device is opened):");
    println!();

    println!("  Hello           {}", hex(&hello()));
    println!("  RequestState    {}", hex(&request_state()));

    let preset = request_preset(0, PresetDetail::Summary).expect("preset 0 is in range");
    println!("  RequestPreset 0 {}", hex(&preset));

    println!();
    println!("Read-only until M3. See PLAN.md for milestones.");
}
