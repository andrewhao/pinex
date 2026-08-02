//! Debug web UI. **Not yet implemented (M1).**
//!
//! A blocking `tiny_http` server — no async runtime, matching the thread model.
//! Shows connection status, firmware version, current preset, all 20 preset
//! names, and recent decoded frames with their raw hex.
//!
//! Parse failures are first-class here, not log lines. When a Tonex firmware
//! update breaks our parsing, this page is how that gets discovered — with the
//! offending bytes in hand.

#![allow(unused_imports)]

use pinex_proto as _;
