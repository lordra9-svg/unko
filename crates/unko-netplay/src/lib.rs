//! Netplay: GGRS rollback plus legacy transport and state primitives.
//!
//! This crate is independent of the frontend and of any specific emulator
//! core. Direct UDP, hole-punch and relay sessions run
//! [`ggrs_session::GgrsNetplay`] through `NonBlockingSocket` adapters, which
//! map GGRS save/load/advance requests onto [`unko_core::Core`]. The transport,
//! rollback-buffer and session modules remain as a library fallback and for
//! the legacy harness path.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod ggrs_session;
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

pub use ggrs_session::{GgrsConfig, GgrsNetplay};
pub use holepunch::{HolePunchTransport, PunchPeer};
pub use input::{InputFrame, INPUT_MESSAGE_LEN};
pub use relay::{room_token, RelayServer, RelayTransport};
pub use rollback::{RollbackBuffer, StoredFrame};
pub use session::{PredictionMode, RollbackSession, SessionConfig, SessionStats};
pub use stun::{public_address as stun_public_address, StunError};
pub use sync::{
    DesyncInfo, DesyncMonitor, StateAck, StateChunk, StateHash, StateReassembler, StateSender,
    STATE_CHUNK_BYTES,
};
pub use transport::{channel_pair, ChannelTransport, MeshTransport, Transport};
pub use udp::{UdpMesh, UdpTransport};
pub use upnp::{
    add_port_mapping, delete_port_mapping, discover, endpoint, external_ip, local_address_toward,
    IgdDevice, MappingRequest, Protocol as MappingProtocol, UpnpError,
};

/// Stable feature-core id.
pub const CORE_ID: &str = "unko_netplay";
