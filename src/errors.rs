use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Clone, Error)]
pub enum PiShockError {
    /// The share code is not linked to this account.
    /// Share codes have to be added to the account on pishock.com before they can be used.
    #[error("Share code not found on this account")]
    ShareCodeNotFound,
    /// No shocker with this ID exists on this account
    #[error("Shocker {} not found on this account", .0)]
    ShockerNotFound(u64),
    #[error("Username or API key invalid")]
    InvalidCredentials,
    #[error("Shocker is in paused state")]
    ShockerPaused,
    /// The share code does not permit this operation (e.g. `"shock"`)
    #[error("Operation not permitted for this shocker: {}", .0)]
    PermissionDenied(&'static str),
    #[error("Invalid intensity specified, max intensity: {}", .0)]
    /// The maximum intensity the shocker can deploy
    InvalidIntensity(u32),
    #[error("Invalid duration specified, max duration: {}", .0)]
    /// The maximum duration the shocker can deploy (in seconds)
    InvalidDuration(u32),
    #[error("Connection error: {}", .0)]
    ConnectionError(String),
    /// The broker did not answer in time
    #[error("Timed out waiting for the PiShock broker")]
    Timeout,
    /// The broker rejected the command
    #[error("PiShock broker error {}: {}", .code.as_deref().unwrap_or("(no code)"), .message)]
    BrokerError {
        code: Option<String>,
        message: String,
    },
    #[error("Shocker is busy")]
    ShockerBusy,
    #[error("Unknown error: {}", .0)]
    UnknownError(String),
    #[error("Shock cooldown exceeded, {:#?} left", .0)]
    /// If a shock is attempted while the cooldown is not over, this error is returned with the remaining cooldown time
    CooldownExceeded(Duration),
}

impl From<reqwest::Error> for PiShockError {
    fn from(error: reqwest::Error) -> Self {
        if error.is_timeout() {
            PiShockError::Timeout
        } else if error.is_decode() {
            PiShockError::UnknownError(format!("Failed to parse PiShock API response: {error}"))
        } else {
            PiShockError::ConnectionError(error.to_string())
        }
    }
}

impl From<tokio_tungstenite::tungstenite::Error> for PiShockError {
    fn from(error: tokio_tungstenite::tungstenite::Error) -> Self {
        use tokio_tungstenite::tungstenite::Error as WsError;
        match error {
            WsError::Http(response) if matches!(response.status().as_u16(), 401 | 403) => {
                PiShockError::InvalidCredentials
            }
            _ => PiShockError::ConnectionError(error.to_string()),
        }
    }
}
