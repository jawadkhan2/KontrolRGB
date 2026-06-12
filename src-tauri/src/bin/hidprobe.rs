//! Diagnostic v3: exercise the GMMK protocol against the vendor collection
//! (usage page 0xFF1C) with per-packet ack reads, mirroring
//! dokutan/rgb_keyboard's interrupt EP3-out / EP2-in transport.
//!
//! Tries output reports (hid_write) first; if the first write fails it
//! retries the whole sequence as feature reports on the same collection.
//! Success looks like: keyboard switches to custom mode and Esc/F1/F2 turn
//! red/green/blue. Run with: cargo run --bin hidprobe

use hidapi::{HidApi, HidDevice};

const VID: u16 = 0x0C45;
const PID: u16 = 0x652F;
const VENDOR_USAGE_PAGE: u16 = 0xFF1C;

#[derive(Clone, Copy, PartialEq)]
enum Transport {
    /// hid_write → interrupt OUT EP 0x03 (what libusb references use).
    OutputReport,
    /// HidD_SetOutputReport → SET_REPORT(Output) on EP0, like
    /// rgb_keyboard's "ajazz compatibility" control-transfer mode.
    SetOutputReport,
    FeatureReport,
}

/// Returns false on write failure so the caller can switch transports.
fn cmd(dev: &HidDevice, t: Transport, payload: &[u8]) -> bool {
    let mut buf = [0u8; 64];
    buf[..payload.len()].copy_from_slice(payload);
    let res = match t {
        Transport::OutputReport => dev.write(&buf).map(|n| Some(n)),
        Transport::SetOutputReport => dev.send_output_report(&buf).map(|()| None),
        Transport::FeatureReport => dev.send_feature_report(&buf).map(|()| None),
    };
    match res {
        Ok(n) => print!(
            "  sent{}  {:02x?}",
            n.map_or(String::new(), |n| format!(" {n:2}")),
            &payload[..payload.len().min(11)]
        ),
        Err(e) => {
            println!("  WRITE ERR {:02x?}: {e}", &payload[..payload.len().min(11)]);
            return false;
        }
    }
    let mut ack = [0u8; 64];
    match dev.read_timeout(&mut ack, 250) {
        Ok(0) => println!("  ack: timeout"),
        Ok(n) => println!("  ack {n:2}: {:02x?}", &ack[..n.min(8)]),
        Err(e) => println!("  ack ERR: {e}"),
    }
    true
}

fn key(dev: &HidDevice, t: Transport, b5: u8, b6: u8, r: u8, g: u8, b: u8) -> bool {
    let ck = b5.wrapping_add(b6).wrapping_add(0x54);
    cmd(dev, t, &[0x04, ck, 0x02, 0x11, 0x03, b5, b6, 0x00, r, g, b])
}

/// Full test sequence; returns false if the first packet can't be written.
fn sequence(dev: &HidDevice, t: Transport) -> bool {
    println!("init: custom mode + max hw brightness");
    if !cmd(dev, t, &[0x04, 0x01, 0x00, 0x01]) {
        return false; // transport dead, let caller try the other one
    }
    cmd(dev, t, &[0x04, 0x1b, 0x00, 0x06, 0x01, 0x00, 0x00, 0x00, 0x14]); // custom mode
    cmd(dev, t, &[0x04, 0x11, 0x00, 0x06, 0x01, 0x01, 0x00, 0x00, 0x09]); // brightness 9
    cmd(dev, t, &[0x04, 0x02, 0x00, 0x02]); // end

    println!("colors: Esc=red F1=green F2=blue");
    cmd(dev, t, &[0x04, 0x01, 0x00, 0x01]); // start
    key(dev, t, 0x03, 0x00, 255, 0, 0); // Esc red
    key(dev, t, 0x06, 0x00, 0, 255, 0); // F1 green
    key(dev, t, 0x09, 0x00, 0, 0, 255); // F2 blue
    cmd(dev, t, &[0x04, 0x02, 0x00, 0x02]); // end
    true
}

fn main() {
    let api = HidApi::new().expect("hidapi init");
    for d in api.device_list().filter(|d| d.vendor_id() == VID && d.product_id() == PID) {
        println!(
            "found: iface {} usage_page {:#06x} usage {:#04x}  {}",
            d.interface_number(),
            d.usage_page(),
            d.usage(),
            d.path().to_string_lossy()
        );
    }

    let info = api
        .device_list()
        .find(|d| {
            d.vendor_id() == VID && d.product_id() == PID && d.usage_page() == VENDOR_USAGE_PAGE
        })
        .expect("GMMK vendor collection (usage page 0xFF1C) not found");
    println!("opening {}", info.path().to_string_lossy());
    let dev = api.open_path(info.path()).expect("open");

    println!("== transport: output reports (interrupt OUT) ==");
    if sequence(&dev, Transport::OutputReport) {
        println!("done — check the keyboard (output-report transport)");
        return;
    }

    println!("== transport: HidD_SetOutputReport (SET_REPORT via EP0) ==");
    if sequence(&dev, Transport::SetOutputReport) {
        println!("done — check the keyboard (set-output-report transport)");
        return;
    }

    println!("== transport: feature reports ==");
    sequence(&dev, Transport::FeatureReport);
    println!("done — check the keyboard (feature-report transport)");
}
