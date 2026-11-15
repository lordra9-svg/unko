//! Miracle Piano probe: run a ROM and print the MIDI bytes the NES software
//! sends. Useful for checking the transport and the documented SysEx replies
//! against real software.
use std::fs;
use unko_nes::{PortDevice, System};

fn main() {
    let path = std::env::args().nth(1).expect("rom");
    let frames: u32 = std::env::args()
        .nth(2)
        .and_then(|v| v.parse().ok())
        .unwrap_or(1800);
    let rom = fs::read(path).expect("read");
    let mut system = System::from_ines(&rom).expect("load");
    system.set_port_device(0, PortDevice::MiraclePiano);
    let mut last = 0usize;
    for frame in 0..frames {
        // The game uses the controller in port 2; press Start periodically.
        system.set_controller(1, if frame % 120 == 60 { 0x08 } else { 0 });
        system.run_frame();
        let tx = system.piano_tx_log();
        if tx.len() != last {
            println!("frame {frame}: tx {:02X?}", &tx[last..]);
            last = tx.len();
        }
    }
    println!("final tx: {:02X?}", system.piano_tx_log());
}
