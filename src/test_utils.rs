//! Mock PiShock REST API and WebSocket broker for tests

use crate::{Endpoints, PiShockAccount};
use futures_util::{SinkExt, StreamExt};
use httpmock::Method::GET;
use httpmock::MockServer;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
use tokio_tungstenite::tungstenite::Message;

pub(crate) const SHARED_SHOCKERS_BODY: &str = r#"{
    "owner1": [
        {
            "shareId": 123, "clientId": 6969, "shockerId": 789, "shockerName": "Shocker 1",
            "isPaused": false, "maxIntensity": 100, "canContinuous": true, "canShock": true,
            "canVibrate": true, "canBeep": true, "canLog": true, "shareCode": "SHARECODE"
        },
        {
            "shareId": 456, "clientId": 6969, "shockerId": 790, "shockerName": "Shocker 2",
            "isPaused": false, "maxIntensity": 50, "canContinuous": false, "canShock": false,
            "canVibrate": true, "canBeep": true, "canLog": true, "shareCode": "RESTRICTED"
        }
    ]
}"#;

const USER_DEVICES_BODY: &str = r#"[
    {
        "clientId": 6969, "name": "PiShock 1", "userId": 1234, "username": "username",
        "shockers": [{ "name": "Shocker 1", "shockerId": 321, "isPaused": false }]
    },
    {
        "clientId": 420, "name": "PiShock 2", "userId": 1234, "username": "username",
        "shockers": [
            { "name": "Shocker 2", "shockerId": 456, "isPaused": false },
            { "name": "Shocker 3", "shockerId": 789, "isPaused": false }
        ]
    }
]"#;

/// Decides the broker's answer to a request, `None` closes the connection
type Responder = fn(&Value) -> Option<Value>;

fn successful_response(request: &Value) -> Option<Value> {
    let message = if request["Operation"] == "PING" {
        "PONG"
    } else {
        "Publish successful."
    };

    Some(json!({
        "ErrorCode": null,
        "IsError": false,
        "Message": message,
        "OriginalCommand": request.to_string()
    }))
}

#[derive(Default)]
struct BrokerState {
    received: Mutex<Vec<Value>>,
    connections: AtomicUsize,
    last_query: Mutex<Option<String>>,
}

/// A local WebSocket server that records published commands
pub(crate) struct MockBroker {
    pub url: String,
    state: Arc<BrokerState>,
    drop_connections: broadcast::Sender<()>,
}

impl MockBroker {
    pub(crate) async fn start(responder: Responder) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}/v2", listener.local_addr().unwrap());
        let state = Arc::new(BrokerState::default());
        let (drop_connections, _) = broadcast::channel(1);

        let accept_state = state.clone();
        let accept_drop = drop_connections.clone();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(handle_connection(
                    stream,
                    accept_state.clone(),
                    accept_drop.subscribe(),
                    responder,
                ));
            }
        });

        MockBroker {
            url,
            state,
            drop_connections,
        }
    }

    /// All received PUBLISH requests
    pub(crate) fn received(&self) -> Vec<Value> {
        self.state.received.lock().unwrap().clone()
    }

    /// Number of accepted connections
    pub(crate) fn connections(&self) -> usize {
        self.state.connections.load(Ordering::SeqCst)
    }

    /// Query string of the last connection request
    pub(crate) fn last_query(&self) -> Option<String> {
        self.state.last_query.lock().unwrap().clone()
    }

    /// Closes all open connections without a close frame, like a lost connection
    pub(crate) fn drop_connections(&self) {
        let _ = self.drop_connections.send(());
    }
}

// The handshake callback's error type is defined by tungstenite
#[allow(clippy::result_large_err)]
async fn handle_connection(
    stream: TcpStream,
    state: Arc<BrokerState>,
    mut drop_connection: broadcast::Receiver<()>,
    responder: Responder,
) {
    let handshake_state = state.clone();
    let mut websocket = tokio_tungstenite::accept_hdr_async(
        stream,
        move |request: &Request, response: Response| {
            *handshake_state.last_query.lock().unwrap() = request.uri().query().map(String::from);
            Ok(response)
        },
    )
    .await
    .unwrap();
    state.connections.fetch_add(1, Ordering::SeqCst);

    loop {
        tokio::select! {
            biased;
            _ = drop_connection.recv() => return,
            message = websocket.next() => match message {
                Some(Ok(Message::Text(text))) => {
                    let request: Value = serde_json::from_str(&text).unwrap();
                    if request["Operation"] == "PUBLISH" {
                        state.received.lock().unwrap().push(request.clone());
                    }

                    match responder(&request) {
                        Some(response) => {
                            websocket.send(Message::text(response.to_string())).await.unwrap();
                        }
                        None => return,
                    }
                }
                Some(Ok(_)) => {}
                _ => return,
            }
        }
    }
}

/// Mock REST API with one account (user ID 1234) plus a mock broker
pub(crate) struct TestServer {
    pub rest: MockServer,
    pub broker: MockBroker,
}

impl TestServer {
    pub(crate) async fn start() -> Self {
        Self::start_with_broker(MockBroker::start(successful_response).await).await
    }

    pub(crate) async fn start_with_broker(broker: MockBroker) -> Self {
        let rest = MockServer::start_async().await;

        rest.mock_async(|when, then| {
            when.method(GET)
                .path("/Auth/GetUserIfAPIKeyValid")
                .query_param("apikey", "apikey")
                .query_param("username", "username");
            then.status(200).body(r#"{"UserID": 1234}"#);
        })
        .await;
        rest.mock_async(|when, then| {
            when.method(GET)
                .path("/PiShock/GetUserDevices")
                .query_param("UserId", "1234");
            then.status(200).body(USER_DEVICES_BODY);
        })
        .await;
        rest.mock_async(|when, then| {
            when.method(GET)
                .path("/PiShock/GetShareCodesByOwner")
                .query_param("UserId", "1234");
            then.status(200).body(r#"{"owner1": [123, 456]}"#);
        })
        .await;
        rest.mock_async(|when, then| {
            when.method(GET)
                .path("/PiShock/GetShockersByShareIds")
                .query_param("UserId", "1234");
            then.status(200).body(SHARED_SHOCKERS_BODY);
        })
        .await;

        TestServer { rest, broker }
    }

    pub(crate) fn account(&self) -> PiShockAccount {
        PiShockAccount::with_endpoints(
            "pishock_rs",
            "username",
            "apikey",
            Endpoints {
                auth_base: self.rest.url(""),
                ps_base: self.rest.url(""),
                broker_url: self.broker.url.clone(),
            },
        )
    }
}
