//! Minimal UDP relay for Unko NetPlay.
//!
//! Usage: `unko-relay [--bind ADDR]` (default `0.0.0.0:3478`).
//! All players must join the same room name in the NetPlay window.

use std::env;
use std::process::ExitCode;
use std::thread;
use std::time::Duration;

use unko_netplay::RelayServer;

fn main() -> ExitCode {
    let mut bind = "0.0.0.0:3478".to_string();
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--bind" => {
                let Some(value) = args.next() else {
                    eprintln!("--bind needs an address");
                    return ExitCode::FAILURE;
                };
                bind = value;
            }
            "-h" | "--help" => {
                println!("usage: unko-relay [--bind 0.0.0.0:3478]");
                return ExitCode::SUCCESS;
            }
            other => {
                eprintln!("unknown argument {other}");
                return ExitCode::FAILURE;
            }
        }
    }
    match RelayServer::spawn(&bind) {
        Ok(server) => {
            println!("unko-relay listening on {}", server.local_addr());
            loop {
                thread::sleep(Duration::from_secs(3600));
            }
        }
        Err(err) => {
            eprintln!("relay failed: {err}");
            ExitCode::FAILURE
        }
    }
}
