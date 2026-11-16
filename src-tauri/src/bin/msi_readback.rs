//! MSI Mystic Light read-back probe (read-only, never writes the board).
//!
//! Question it answers: can the host read the controller's current setup
//! (report 0x50) back, so the app can detect "the board left direct mode"
//! without having to re-send the setup blindly? Prints what each GET_FEATURE
//! returns and how it compares to the direct-mode template the app sends.
//!
//! Usage: cargo run --bin msi_readback

use hidapi::HidApi;

const VID: u16 = 0x0DB0;
const PID: u16 = 0x0076;
const USAGE_PAGE: u16 = 0xFF00;

fn main() {
    let api = HidApi::new().expect("hidapi init");
    let Some(info) = api
        .device_list()
        .find(|d| d.vendor_id() == VID && d.product_id() == PID && d.usage_page() == USAGE_PAGE)
    else {
        eprintln!("MSI Mystic Light controller not found");
        std::process::exit(1);
    };
    println!("path: {:?}", info.path());
    let dev = api.open_path(info.path()).expect("open");

    for (id, len) in [(0x50u8, 290usize), (0x50, 727), (0x51, 727), (0x52, 727)] {
        let mut buf = vec![0u8; len];
        buf[0] = id;
        match dev.get_feature_report(&mut buf) {
            Ok(n) => {
                println!("\nreport 0x{id:02x} len {len}: OK, {n} bytes");
                for (row, chunk) in buf[..n.min(buf.len())].chunks(16).take(24).enumerate() {
                    let hex: Vec<String> = chunk.iter().map(|b| format!("{b:02x}")).collect();
                    println!("  {:04x}: {}", row * 16, hex.join(" "));
                }
            }
            Err(e) => println!("\nreport 0x{id:02x} len {len}: ERR {e}"),
        }
    }
}
