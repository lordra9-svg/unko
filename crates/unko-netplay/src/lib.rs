//! Netplay foundation: frame transport and rollback state storage.
//!
//! This crate is deliberately independent of the frontend and of any specific
//! emulator core. It provides the two pieces that rollback netplay needs
//! before a session loop can exist:
//!
//! - [`Transport`] for exchanging encoded [`InputFrame`] messages, with an
//!   in-process channel pair for tests;
//! - [`RollbackBuffer`] for storing a serialized core state plus the input for
//!   the frame that follows it, so a late remote input can restore an older
//!   frame and resimulate.
//!
//! A [`UdpTransport`] exchanges the same messages over the network. The app
//! wiring and prediction tuning are follow-up work; this crate defines the
//! pieces they build on and is covered by unit tests.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod holepunch;
pub mod input;
pub mod relay;
pub mod rollback;
pub mod session;
pub mod stun;
pub mod sync;
pub mod transport;
pub mod udp;
pub mod upnp;

pub use holepunch::{HolePunchTransport, PunchPeer};
pub use input::{InputFrame, INPUT_MESSAGE_LEN};
pub use relay::{room_token, RelayServer, RelayTransport};
pub use rollback::{RollbackBuffer, StoredFrame};
pub use session::{PredictionMode, RollbackSession, SessionConfig, SessionStats};
pub use stun::{public_address as stun_public_address, StunError};
pub use sync::{DesyncInfo, DesyncMonitor, StateHash};
pub use transport::{channel_pair, ChannelTransport, MeshTransport, Transport};
pub use udp::{UdpMesh, UdpTransport};
pub use upnp::{
    add_port_mapping, delete_port_mapping, discover, endpoint, external_ip, local_address_toward,
    IgdDevice, MappingRequest, Protocol as MappingProtocol, UpnpError,
};

/// Stable feature-core id.
pub const CORE_ID: &str = "unko_netplay";
