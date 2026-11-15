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
//! The session loop (prediction, resimulation and the app wiring) and a real
//! UDP transport are follow-up work; this crate defines the pieces they build
//! on and is covered by unit tests.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod input;
pub mod rollback;
pub mod session;
pub mod transport;

pub use input::{InputFrame, INPUT_MESSAGE_LEN};
pub use rollback::{RollbackBuffer, StoredFrame};
pub use session::RollbackSession;
pub use transport::{channel_pair, ChannelTransport, Transport};

/// Stable feature-core id.
pub const CORE_ID: &str = "unko_netplay";
