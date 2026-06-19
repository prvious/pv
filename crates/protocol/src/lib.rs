mod error;
mod event;
mod request;
mod response;
#[cfg(test)]
mod tests;
mod transport;
mod update_check;

pub use error::ProtocolError;
pub use event::DaemonEvent;
pub use request::{DaemonCommand, DaemonRequest};
pub use response::{DaemonResponse, ResponseStatus};
pub use transport::{DaemonTransport, transport, write_line};
pub use update_check::{
    ManagedResourceUpdateBlocker, ManagedResourceUpdateCheck, ManagedResourceUpdateCheckTrack,
    ManagedResourceUpdateRevocation, ManagedResourceUpdateStatus,
};

pub const PROTOCOL_VERSION: u16 = 2;
