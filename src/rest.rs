//! Thin client for the read-only PiShock REST endpoints (user, devices and shared shockers).
//! Operations are not sent through REST, see [`crate::broker`].

use crate::errors::PiShockError;
use crate::Endpoints;
use log::debug;
use reqwest::{Response, StatusCode};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A PiShock hub owned by the account, as returned by `GetUserDevices`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PiShockDevice {
    /// The ID of the hub, used to address its shockers
    pub client_id: u64,
    pub name: String,
    pub user_id: u64,
    pub username: String,
    pub shockers: Vec<OwnShockerInfo>,
}

/// A shocker connected to one of the account's own hubs
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnShockerInfo {
    pub name: String,
    pub shocker_id: u64,
    pub is_paused: bool,
}

/// A shocker that another user shared with this account, as returned by `GetShockersByShareIds`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedShockerInfo {
    /// Username of the shocker's owner
    #[serde(default)]
    pub owner: String,
    pub share_id: u64,
    pub client_id: u64,
    pub shocker_id: u64,
    pub shocker_name: String,
    pub is_paused: bool,
    pub max_intensity: u32,
    pub can_continuous: bool,
    pub can_shock: bool,
    pub can_vibrate: bool,
    pub can_beep: bool,
    pub can_log: bool,
    pub share_code: String,
}

#[derive(Deserialize)]
struct UserInfo {
    #[serde(rename = "UserID", alias = "UserId", alias = "userId")]
    user_id: u64,
}

/// Resolves the user ID belonging to a username and API key
pub(crate) async fn get_user_id(
    http: &reqwest::Client,
    endpoints: &Endpoints,
    username: &str,
    api_key: &str,
) -> Result<u64, PiShockError> {
    debug!("Resolving user ID of {username}");

    let response = http
        .get(format!("{}/Auth/GetUserIfAPIKeyValid", endpoints.auth_base))
        .query(&[("apikey", api_key), ("username", username)])
        .send()
        .await
        .map_err(|e| PiShockError::from(e.without_url()))?;

    // The auth server answers 404 for unknown username/API key combinations
    if response.status() == StatusCode::NOT_FOUND {
        return Err(PiShockError::InvalidCredentials);
    }

    Ok(parse_json::<UserInfo>(response).await?.user_id)
}

/// Lists the account's own hubs and their shockers
pub(crate) async fn get_user_devices(
    http: &reqwest::Client,
    endpoints: &Endpoints,
    user_id: u64,
    api_key: &str,
) -> Result<Vec<PiShockDevice>, PiShockError> {
    let response = http
        .get(format!("{}/PiShock/GetUserDevices", endpoints.ps_base))
        .query(&auth_query(user_id, api_key))
        .send()
        .await
        .map_err(|e| PiShockError::from(e.without_url()))?;

    parse_json(response).await
}

/// Lists all shockers shared with the account.
/// This needs two requests: one for the share IDs, one for the details of those shares.
pub(crate) async fn get_shared_shockers(
    http: &reqwest::Client,
    endpoints: &Endpoints,
    user_id: u64,
    api_key: &str,
) -> Result<Vec<SharedShockerInfo>, PiShockError> {
    let response = http
        .get(format!(
            "{}/PiShock/GetShareCodesByOwner",
            endpoints.ps_base
        ))
        .query(&auth_query(user_id, api_key))
        .send()
        .await
        .map_err(|e| PiShockError::from(e.without_url()))?;

    let share_ids: Vec<u64> = parse_json::<BTreeMap<String, Vec<u64>>>(response)
        .await?
        .into_values()
        .flatten()
        .collect();

    if share_ids.is_empty() {
        return Ok(Vec::new());
    }

    // Every share ID is passed as its own `shareIds` query parameter
    let mut query = auth_query(user_id, api_key);
    query.extend(share_ids.iter().map(|id| ("shareIds", id.to_string())));

    let response = http
        .get(format!(
            "{}/PiShock/GetShockersByShareIds",
            endpoints.ps_base
        ))
        .query(&query)
        .send()
        .await
        .map_err(|e| PiShockError::from(e.without_url()))?;

    let shockers = parse_json::<BTreeMap<String, Vec<SharedShockerInfo>>>(response)
        .await?
        .into_iter()
        .flat_map(|(owner, shockers)| {
            shockers.into_iter().map(move |mut shocker| {
                shocker.owner.clone_from(&owner);
                shocker
            })
        })
        .collect();

    Ok(shockers)
}

/// Query parameters to authenticate with a user ID and API key
fn auth_query(user_id: u64, api_key: &str) -> Vec<(&'static str, String)> {
    vec![
        ("UserId", user_id.to_string()),
        ("Token", api_key.to_string()),
        ("api", "true".to_string()),
    ]
}

async fn parse_json<T: DeserializeOwned>(response: Response) -> Result<T, PiShockError> {
    let status = response.status();
    debug!("Response from PiShock API: {status}");

    match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Err(PiShockError::InvalidCredentials),
        status if !status.is_success() => Err(PiShockError::ConnectionError(format!(
            "PiShock API responded with {status}"
        ))),
        _ => response
            .json::<T>()
            .await
            .map_err(|e| PiShockError::from(e.without_url())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::Method::GET;
    use httpmock::MockServer;
    use test_log::test;

    fn endpoints(server: &MockServer) -> Endpoints {
        Endpoints {
            auth_base: server.url(""),
            ps_base: server.url(""),
            broker_url: String::new(),
        }
    }

    #[test(tokio::test)]
    async fn user_id_is_parsed() {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(GET)
                .path("/Auth/GetUserIfAPIKeyValid")
                .query_param("apikey", "apikey")
                .query_param("username", "username");
            then.status(200)
                .body(r#"{"UserID": 1234, "Username": "username", "Extra": true}"#);
        });

        let user_id = get_user_id(
            &reqwest::Client::new(),
            &endpoints(&server),
            "username",
            "apikey",
        )
        .await
        .unwrap();

        assert_eq!(user_id, 1234);
        mock.assert();
    }

    #[test(tokio::test)]
    async fn unknown_credentials_are_rejected() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/Auth/GetUserIfAPIKeyValid");
            then.status(404);
        });

        let result = get_user_id(
            &reqwest::Client::new(),
            &endpoints(&server),
            "username",
            "apikey",
        )
        .await;

        assert!(matches!(result, Err(PiShockError::InvalidCredentials)));
    }

    #[test(tokio::test)]
    async fn unauthorized_device_request_is_rejected() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/PiShock/GetUserDevices");
            then.status(401);
        });

        let result =
            get_user_devices(&reqwest::Client::new(), &endpoints(&server), 1234, "apikey").await;

        assert!(matches!(result, Err(PiShockError::InvalidCredentials)));
    }

    #[test(tokio::test)]
    async fn shared_shockers_are_fetched_by_share_ids() {
        let server = MockServer::start();
        let codes_mock = server.mock(|when, then| {
            when.method(GET)
                .path("/PiShock/GetShareCodesByOwner")
                .query_param("UserId", "1234")
                .query_param("Token", "apikey")
                .query_param("api", "true");
            then.status(200).body(r#"{"owner1": [123, 456]}"#);
        });
        let shockers_mock = server.mock(|when, then| {
            when.method(GET)
                .path("/PiShock/GetShockersByShareIds")
                .query_param("UserId", "1234")
                .query_param("Token", "apikey")
                .query_param("api", "true")
                .query_param("shareIds", "123")
                .query_param("shareIds", "456");
            then.status(200)
                .body(crate::test_utils::SHARED_SHOCKERS_BODY);
        });

        let shockers =
            get_shared_shockers(&reqwest::Client::new(), &endpoints(&server), 1234, "apikey")
                .await
                .unwrap();

        assert_eq!(shockers.len(), 2);
        assert_eq!(shockers[0].owner, "owner1");
        assert_eq!(shockers[0].share_code, "SHARECODE");
        assert_eq!(shockers[1].max_intensity, 50);
        assert!(!shockers[1].can_shock);
        codes_mock.assert();
        shockers_mock.assert();
    }

    #[test(tokio::test)]
    async fn no_share_ids_skips_second_request() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/PiShock/GetShareCodesByOwner");
            then.status(200).body("{}");
        });
        let shockers_mock = server.mock(|when, then| {
            when.method(GET).path("/PiShock/GetShockersByShareIds");
            then.status(200).body("{}");
        });

        let shockers =
            get_shared_shockers(&reqwest::Client::new(), &endpoints(&server), 1234, "apikey")
                .await
                .unwrap();

        assert!(shockers.is_empty());
        shockers_mock.assert_hits(0);
    }
}
