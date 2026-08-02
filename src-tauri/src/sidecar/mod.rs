//! The bridge to the Swift sidecar.
//!
//! Split three ways on purpose:
//! - `protocol` is pure serde, so the wire contract is testable without processes
//! - `policy` is pure restart logic, so backoff and give-up are testable without crashes
//! - `supervisor` is the only part that touches `std::process`

pub mod policy;
pub mod protocol;
pub mod supervisor;

pub use protocol::{
    AudioSource, ModelState, PermissionState, SidecarCommand, SidecarEvent, PROTOCOL_VERSION,
};
pub use supervisor::{resolve_binary, SidecarError, Supervisor, SupervisorEvent};
