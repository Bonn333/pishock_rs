use crate::broker::Broker;
use crate::pishocker::ShockerMetadata;
use crate::rest::{self, PiShockDevice, SharedShockerInfo};
use crate::{errors, Endpoints, PiShocker};
use log::debug;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::OnceCell;

/// A struct representing PiShock account credentials.
/// Should be used to create [`PiShocker`] instances.
///
/// Construct a new instance with [`PiShockAccount::new`].
/// Cloning is cheap: all clones (and the [`PiShocker`]s created from them) share one broker connection.
///
/// The API key must have been generated after 2024-10-15 to be accepted by the v2 broker.
#[derive(Debug, Clone)]
pub struct PiShockAccount {
    inner: Arc<AccountInner>,
}

pub(crate) struct AccountInner {
    pub app_name: String,
    pub username: String,
    pub api_key: String,
    pub http_client: reqwest::Client,
    pub endpoints: Endpoints,
    pub broker: Arc<Broker>,
    user_id: OnceCell<u64>,
}

impl std::fmt::Debug for AccountInner {
    // The API key is deliberately left out
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccountInner")
            .field("app_name", &self.app_name)
            .field("username", &self.username)
            .field("user_id", &self.user_id.get())
            .finish_non_exhaustive()
    }
}

impl AccountInner {
    /// Returns the user ID, resolving it on first use
    pub(crate) async fn user_id(&self) -> Result<u64, errors::PiShockError> {
        self.user_id
            .get_or_try_init(|| {
                rest::get_user_id(
                    &self.http_client,
                    &self.endpoints,
                    &self.username,
                    &self.api_key,
                )
            })
            .await
            .copied()
    }

    pub(crate) async fn devices(&self) -> Result<Vec<PiShockDevice>, errors::PiShockError> {
        let user_id = self.user_id().await?;
        rest::get_user_devices(&self.http_client, &self.endpoints, user_id, &self.api_key).await
    }

    pub(crate) async fn shared_shockers(
        &self,
    ) -> Result<Vec<SharedShockerInfo>, errors::PiShockError> {
        let user_id = self.user_id().await?;
        rest::get_shared_shockers(&self.http_client, &self.endpoints, user_id, &self.api_key).await
    }
}

impl PartialEq for PiShockAccount {
    fn eq(&self, other: &Self) -> bool {
        self.inner.app_name == other.inner.app_name
            && self.inner.username == other.inner.username
            && self.inner.api_key == other.inner.api_key
    }
}

impl Eq for PiShockAccount {}

impl PiShockAccount {
    /// Creates a new account handle. No request is made until it is used.
    ///
    /// `app_name` is shown as the origin of all operations in the shocker logs.
    #[must_use]
    pub fn new<S: Into<String>>(app_name: S, username: S, api_key: S) -> PiShockAccount {
        Self::with_endpoints(app_name, username, api_key, Endpoints::default())
    }

    pub(crate) fn with_endpoints<S: Into<String>>(
        app_name: S,
        username: S,
        api_key: S,
        endpoints: Endpoints,
    ) -> PiShockAccount {
        let username = username.into();
        let api_key = api_key.into();

        PiShockAccount {
            inner: Arc::new(AccountInner {
                app_name: app_name.into(),
                broker: Arc::new(Broker::new(&endpoints.broker_url, &username, &api_key)),
                username,
                api_key,
                http_client: reqwest::Client::builder()
                    .timeout(Duration::from_secs(10))
                    .build()
                    .expect("failed to build HTTP client"),
                endpoints,
                user_id: OnceCell::new(),
            }),
        }
    }

    /// Returns the PiShock user ID of this account.
    /// The first call also verifies the username and API key.
    ///
    /// # Errors
    /// Returns [`errors::PiShockError::InvalidCredentials`] if the username or API key is wrong.
    pub async fn get_user_id(&self) -> Result<u64, errors::PiShockError> {
        self.inner.user_id().await
    }

    /// Lists the account's own PiShock hubs and their shockers
    pub async fn get_devices(&self) -> Result<Vec<PiShockDevice>, errors::PiShockError> {
        self.inner.devices().await
    }

    /// Lists all shockers that other users shared with this account
    pub async fn get_shared_shockers(
        &self,
    ) -> Result<Vec<SharedShockerInfo>, errors::PiShockError> {
        self.inner.shared_shockers().await
    }

    /// Returns a [`PiShocker`] instance for the specified share code
    ///
    /// The share code has to be added to this account on pishock.com first,
    /// the v2 API cannot use share codes that are not linked to the account.
    ///
    /// ```no_run
    /// # tokio_test::block_on(async {
    /// # use pishock_rs::PiShockAccount;
    /// let pishock_account = PiShockAccount::new("pishock_rs", "username", "apikey");
    /// let pishocker_instance = pishock_account.get_shocker("sharecode").await;
    /// # });
    /// ```
    /// # Errors
    /// Returns [`errors::PiShockError::ShareCodeNotFound`] if the share code is not linked to this account
    /// (does NOT fail for paused shockers).
    pub async fn get_shocker<S: Into<String>>(
        &self,
        share_code: S,
    ) -> Result<PiShocker, errors::PiShockError> {
        let share_code = share_code.into();
        let shared_shockers = self.inner.shared_shockers().await?;
        let shocker = find_shared_shocker(&shared_shockers, &share_code)?;

        let mut pishocker = self.shocker_from_ids(
            shocker.client_id,
            shocker.shocker_id,
            Some(shocker.share_code.clone()),
        );
        pishocker.metadata = Some(ShockerMetadata::from(shocker));

        Ok(pishocker)
    }

    /// Returns a [`PiShocker`] instance for one of the account's own shockers
    ///
    /// # Errors
    /// Returns [`errors::PiShockError::ShockerNotFound`] if none of the account's hubs has this shocker.
    pub async fn get_own_shocker(
        &self,
        shocker_id: u64,
    ) -> Result<PiShocker, errors::PiShockError> {
        let devices = self.inner.devices().await?;
        let (device, shocker) = find_own_shocker(&devices, shocker_id)?;

        let mut pishocker = self.shocker_from_ids(device.client_id, shocker.shocker_id, None);
        pishocker.metadata = Some(ShockerMetadata::from_own(device, shocker));

        Ok(pishocker)
    }

    /// Returns [`PiShocker`] instances for all of the account's own shockers
    pub async fn get_all_own_shockers(&self) -> Result<Vec<PiShocker>, errors::PiShockError> {
        let devices = self.inner.devices().await?;

        Ok(devices
            .iter()
            .flat_map(|device| {
                device.shockers.iter().map(move |shocker| {
                    let mut pishocker =
                        self.shocker_from_ids(device.client_id, shocker.shocker_id, None);
                    pishocker.metadata = Some(ShockerMetadata::from_own(device, shocker));
                    pishocker
                })
            })
            .collect())
    }

    /// Returns a [`PiShocker`] instance for known IDs without making any request.
    ///
    /// Pass the share code for shockers shared with this account, or `None` for own shockers.
    /// No metadata is available until [`PiShocker::refresh_metadata`] is called,
    /// so limits like the maximum intensity are only enforced by the PiShock servers.
    #[must_use]
    pub fn shocker_from_ids(
        &self,
        client_id: u64,
        shocker_id: u64,
        share_code: Option<String>,
    ) -> PiShocker {
        debug!("Creating PiShocker for client {client_id}, shocker {shocker_id}");
        PiShocker::new(self.inner.clone(), client_id, shocker_id, share_code)
    }
}

pub(crate) fn find_shared_shocker<'a>(
    shared_shockers: &'a [SharedShockerInfo],
    share_code: &str,
) -> Result<&'a SharedShockerInfo, errors::PiShockError> {
    shared_shockers
        .iter()
        .find(|shocker| shocker.share_code.eq_ignore_ascii_case(share_code))
        .ok_or(errors::PiShockError::ShareCodeNotFound)
}

pub(crate) fn find_own_shocker(
    devices: &[PiShockDevice],
    shocker_id: u64,
) -> Result<(&PiShockDevice, &rest::OwnShockerInfo), errors::PiShockError> {
    devices
        .iter()
        .find_map(|device| {
            device
                .shockers
                .iter()
                .find(|shocker| shocker.shocker_id == shocker_id)
                .map(|shocker| (device, shocker))
        })
        .ok_or(errors::PiShockError::ShockerNotFound(shocker_id))
}
