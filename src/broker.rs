//! Persistent connection to the PiShock v2 WebSocket broker, which all operations are published to.

use crate::errors::PiShockError;
use futures_util::{SinkExt, StreamExt};
use log::{debug, warn};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

const RESPONSE_TIMEOUT: Duration = Duration::from_secs(10);
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Serialize)]
#[serde(tag = "Operation", rename_all = "UPPERCASE")]
enum BrokerRequest<'a> {
    Ping,
    Publish {
        #[serde(rename = "PublishCommands")]
        publish_commands: &'a [PublishCommand],
    },
}

/// A single command for a shocker, published to the channel of its hub
#[derive(Debug, Clone, Serialize)]
pub(crate) struct PublishCommand {
    /// `c{clientId}-ops` for own shockers, `c{clientId}-sops-{shareCode}` for shared ones
    #[serde(rename = "Target")]
    pub target: String,
    #[serde(rename = "Body")]
    pub body: CommandBody,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct CommandBody {
    /// Shocker ID
    pub id: u64,
    /// Mode: `s`hock, `v`ibrate or `b`eep
    pub m: &'static str,
    /// Intensity
    pub i: u32,
    /// Duration in milliseconds
    pub d: u64,
    /// Repeating, always true according to the API documentation
    pub r: bool,
    pub l: CommandLog,
}

/// Information shown in the shocker's logs
#[derive(Debug, Clone, Serialize)]
pub(crate) struct CommandLog {
    /// User ID of the sender
    pub u: u64,
    /// `sc` for share codes, `api` for own shockers
    pub ty: &'static str,
    /// Whether this is a warning vibration
    pub w: bool,
    /// Whether the button is held / continuous mode
    pub h: bool,
    /// Origin name shown in the logs
    pub o: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct BrokerResponse {
    #[serde(default)]
    error_code: Option<serde_json::Value>,
    #[serde(default)]
    is_error: bool,
    #[serde(default)]
    message: String,
}

impl BrokerResponse {
    /// Error codes are strings like `AUTH_ERROR`, numbers are accepted just in case
    fn error_code(&self) -> Option<String> {
        match &self.error_code {
            None | Some(serde_json::Value::Null) => None,
            Some(serde_json::Value::String(code)) => Some(code.clone()),
            Some(code) => Some(code.to_string()),
        }
    }

    /// The broker sends these right after connecting with invalid credentials, then closes the connection
    fn is_auth_error(&self) -> bool {
        self.is_error
            && matches!(
                self.error_code().as_deref(),
                Some("AUTH_ERROR" | "API_KEY_ERROR")
            )
    }
}

pub(crate) struct Broker {
    /// Broker URL including the credential query parameters
    url: String,
    connection: Mutex<Option<WsStream>>,
    keepalive_started: AtomicBool,
}

impl std::fmt::Debug for Broker {
    // The URL contains the API key, so it is deliberately left out
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Broker").finish_non_exhaustive()
    }
}

impl Broker {
    /// Creates a broker handle. No connection is made until the first command is published.
    pub(crate) fn new(broker_url: &str, username: &str, api_key: &str) -> Self {
        // The broker URL is one of the crate's own constants (or a test server), so parsing cannot fail
        let url = reqwest::Url::parse_with_params(
            broker_url,
            &[("Username", username), ("ApiKey", api_key)],
        )
        .expect("broker URL must be valid");

        Broker {
            url: url.into(),
            connection: Mutex::new(None),
            keepalive_started: AtomicBool::new(false),
        }
    }

    /// Publishes commands and waits for the broker to acknowledge them
    pub(crate) async fn publish(
        self: &Arc<Self>,
        commands: &[PublishCommand],
    ) -> Result<(), PiShockError> {
        let request = serde_json::to_string(&BrokerRequest::Publish {
            publish_commands: commands,
        })
        .map_err(|e| PiShockError::UnknownError(e.to_string()))?;

        debug!("Publishing to PiShock broker: {request}");
        let response = self.request(&request).await?;
        debug!("Response from PiShock broker: {response:?}");

        if response.is_error {
            return Err(broker_error(response));
        }

        Ok(())
    }

    async fn request(self: &Arc<Self>, request: &str) -> Result<BrokerResponse, PiShockError> {
        let mut connection = self.connection.lock().await;
        let reused_connection = connection.is_some();

        match self.request_on(&mut connection, request).await {
            // An idle connection may have been dropped by the server, retry once on a fresh one.
            // Timeouts are not retried, as the command may already have been executed.
            Err(PiShockError::ConnectionError(error)) if reused_connection => {
                debug!("Broker connection lost ({error}), reconnecting");
                self.request_on(&mut connection, request).await
            }
            result => result,
        }
    }

    /// Sends a request on the given connection (connecting first if needed) and waits for the response.
    /// The connection is discarded if anything goes wrong, so the next request reconnects.
    async fn request_on(
        self: &Arc<Self>,
        connection: &mut Option<WsStream>,
        request: &str,
    ) -> Result<BrokerResponse, PiShockError> {
        let stream = match connection {
            Some(stream) => stream,
            None => {
                debug!("Connecting to PiShock broker");
                let (stream, _) = connect_async(self.url.as_str()).await?;
                self.start_keepalive();
                connection.insert(stream)
            }
        };

        let result = tokio::time::timeout(RESPONSE_TIMEOUT, exchange(stream, request))
            .await
            .unwrap_or(Err(PiShockError::Timeout));

        let connection_usable = matches!(&result, Ok(response) if !response.is_auth_error());
        if !connection_usable {
            *connection = None;
        }

        result
    }

    /// Pings the broker periodically so idle connections are not dropped.
    /// The task stops once the broker (and with it the account) is dropped.
    fn start_keepalive(self: &Arc<Self>) {
        if self.keepalive_started.swap(true, Ordering::SeqCst) {
            return;
        }

        let broker = Arc::downgrade(self);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(KEEPALIVE_INTERVAL);
            // The first tick completes immediately
            interval.tick().await;

            loop {
                interval.tick().await;
                let Some(broker) = broker.upgrade() else {
                    break;
                };

                let mut connection = broker.connection.lock().await;
                // Only keep open connections alive, closed ones are reopened by the next command
                if connection.is_none() {
                    continue;
                }

                let ping = serde_json::to_string(&BrokerRequest::Ping).unwrap();
                if let Err(e) = broker.request_on(&mut connection, &ping).await {
                    warn!("PiShock broker keepalive failed: {e}");
                }
            }
        });
    }
}

/// Sends one request and returns the first broker response
async fn exchange(stream: &mut WsStream, request: &str) -> Result<BrokerResponse, PiShockError> {
    stream.send(Message::text(request)).await?;

    loop {
        match stream.next().await {
            Some(Ok(Message::Text(text))) => match serde_json::from_str::<BrokerResponse>(&text) {
                Ok(response) => return Ok(response),
                Err(_) => debug!("Ignoring unexpected broker message: {text}"),
            },
            Some(Ok(Message::Close(frame))) => {
                return Err(PiShockError::ConnectionError(format!(
                    "Broker closed the connection: {frame:?}"
                )))
            }
            // Ping/pong frames are answered by tungstenite itself
            Some(Ok(_)) => {}
            Some(Err(e)) => return Err(e.into()),
            None => {
                return Err(PiShockError::ConnectionError(
                    "Broker closed the connection".to_string(),
                ))
            }
        }
    }
}

fn broker_error(response: BrokerResponse) -> PiShockError {
    let message = response.message.to_lowercase();

    if response.is_auth_error() {
        PiShockError::InvalidCredentials
    } else if message.contains("paused") {
        PiShockError::ShockerPaused
    } else if message.contains("busy") || message.contains("in use") {
        PiShockError::ShockerBusy
    } else if message.contains("not authorized") || message.contains("unauthorized") {
        PiShockError::InvalidCredentials
    } else {
        PiShockError::BrokerError {
            code: response.error_code(),
            message: response.message,
        }
    }
}
