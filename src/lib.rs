mod broker;
pub mod errors;
mod pishocker;
pub use self::pishocker::*;
mod pishock_account;
pub use self::pishock_account::*;
pub mod interpolation;
mod rest;
pub use self::rest::{OwnShockerInfo, PiShockDevice, SharedShockerInfo};

use std::time::Duration;

/// The maximum duration of a single operation.
/// The v2 API does not report a per-shocker duration limit, so this library enforces the historical PiShock limit.
pub const MAX_DURATION: Duration = Duration::from_secs(15);

/// The minimum duration of a single operation.
pub const MIN_DURATION: Duration = Duration::from_millis(100);

/// Base URLs of the PiShock services (without trailing slash).
/// Only overridden in tests, where they point at local mock servers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Endpoints {
    /// Used to resolve the user ID from a username and API key
    pub auth_base: String,
    /// Used to list devices and shared shockers
    pub ps_base: String,
    /// The v2 WebSocket broker that operations are published to
    pub broker_url: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Endpoints {
            auth_base: "https://auth.pishock.com".to_string(),
            ps_base: "https://ps.pishock.com".to_string(),
            broker_url: "wss://broker.pishock.com/v2".to_string(),
        }
    }
}

#[cfg(test)]
pub(crate) mod test_utils;
