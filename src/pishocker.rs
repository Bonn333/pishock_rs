use crate::broker::{CommandBody, CommandLog, PublishCommand};
use crate::errors::PiShockError;
use crate::pishock_account::{find_own_shocker, find_shared_shocker, AccountInner};
use crate::rest::{OwnShockerInfo, PiShockDevice, SharedShockerInfo};
use crate::{errors, MAX_DURATION, MIN_DURATION};
use log::{debug, info};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time::Instant;

// This is imported for the doc comments
#[allow(unused_imports)]
use crate::PiShockAccount;

/// The highest intensity any shocker accepts
const MAX_INTENSITY: u32 = 100;

/// Represents a single [`PiShocker`] device. This struct should not be constructed directly,
/// use [`PiShockAccount::get_shocker`] or [`PiShockAccount::get_own_shocker`] instead.
#[derive(Debug, Clone)]
pub struct PiShocker {
    pub(crate) account: Arc<AccountInner>,
    pub(crate) client_id: u64,
    pub(crate) shocker_id: u64,
    /// Set for shockers shared with the account, `None` for own shockers
    pub(crate) share_code: Option<String>,
    pub(crate) metadata: Option<ShockerMetadata>,
    pub(crate) cooldown: Option<Duration>,
    pub(crate) last_shock: Arc<Mutex<Option<Instant>>>,
}

#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub(crate) struct ShockerMetadata {
    pub name: String,
    pub owner: String,
    pub paused: bool,
    pub max_intensity: u32,
    pub can_shock: bool,
    pub can_vibrate: bool,
    pub can_beep: bool,
    pub can_continuous: bool,
    pub can_log: bool,
}

impl From<&SharedShockerInfo> for ShockerMetadata {
    fn from(shocker: &SharedShockerInfo) -> Self {
        ShockerMetadata {
            name: shocker.shocker_name.clone(),
            owner: shocker.owner.clone(),
            paused: shocker.is_paused,
            max_intensity: shocker.max_intensity,
            can_shock: shocker.can_shock,
            can_vibrate: shocker.can_vibrate,
            can_beep: shocker.can_beep,
            can_continuous: shocker.can_continuous,
            can_log: shocker.can_log,
        }
    }
}

impl ShockerMetadata {
    /// Own shockers have no share restrictions
    pub(crate) fn from_own(device: &PiShockDevice, shocker: &OwnShockerInfo) -> Self {
        ShockerMetadata {
            name: shocker.name.clone(),
            owner: device.username.clone(),
            paused: shocker.is_paused,
            max_intensity: MAX_INTENSITY,
            can_shock: true,
            can_vibrate: true,
            can_beep: true,
            can_continuous: true,
            can_log: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Operation {
    Shock,
    Vibrate,
    Beep,
}

impl Operation {
    fn mode(self) -> &'static str {
        match self {
            Operation::Shock => "s",
            Operation::Vibrate => "v",
            Operation::Beep => "b",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Operation::Shock => "shock",
            Operation::Vibrate => "vibrate",
            Operation::Beep => "beep",
        }
    }
}

impl PiShocker {
    pub(crate) fn new(
        account: Arc<AccountInner>,
        client_id: u64,
        shocker_id: u64,
        share_code: Option<String>,
    ) -> PiShocker {
        PiShocker {
            account,
            client_id,
            shocker_id,
            share_code,
            metadata: None,
            cooldown: None,
            last_shock: Arc::new(Mutex::new(None)),
        }
    }

    /// Returns the share code, or `None` for the account's own shockers
    #[must_use]
    pub fn get_share_code(&self) -> Option<String> {
        self.share_code.clone()
    }

    /// Triggers a beep with the specified duration
    ///
    /// ```no_run
    /// # tokio_test::block_on(async {
    /// use std::time::Duration;
    /// use pishock_rs::PiShocker;
    /// use pishock_rs::PiShockAccount;
    ///
    /// let pishock_account = PiShockAccount::new("pishock_rs", "username", "apikey");
    ///
    /// let pishocker_instance = pishock_account.get_shocker("sharecode".to_string()).await.unwrap();
    ///
    /// // Beeps for 10 seconds
    /// pishocker_instance.beep(Duration::from_secs(10)).await.expect("Failed to beep");
    /// # });
    pub async fn beep(&self, duration: Duration) -> Result<(), PiShockError> {
        debug!("Beeping user for {:#?}", duration);
        self.send_operation(Operation::Beep, 0, duration, false)
            .await
    }

    /// Vibrates the shocker with the specified intensity and duration
    /// Intensity is a value between 1 and 100
    ///
    /// ```no_run
    /// # tokio_test::block_on(async {
    /// use std::time::Duration;
    /// use pishock_rs::PiShocker;
    /// use pishock_rs::PiShockAccount;
    ///
    /// let pishock_account = PiShockAccount::new("pishock_rs", "username", "apikey");
    ///
    /// let pishocker_instance = pishock_account.get_shocker("sharecode".to_string()).await.unwrap();
    ///
    /// // Vibrate with an intensity of 50 and a duration of 10 seconds
    /// pishocker_instance.vibrate(50, Duration::from_secs(10)).await.expect("Failed to vibrate");
    /// # });
    pub async fn vibrate(&self, intensity: u32, duration: Duration) -> Result<(), PiShockError> {
        info!(
            "Vibrating user with intensity {} and duration {:#?}",
            intensity, duration
        );
        self.send_operation(Operation::Vibrate, intensity, duration, false)
            .await
    }

    /// Delivers a 300ms shock with the specified intensity
    /// Intensity is a value between 1 and the maximum intensity of the shocker (max 100)
    pub async fn mini_shock(&self, intensity: u32) -> Result<(), errors::PiShockError> {
        info!(
            "Mini shocking user with intensity {} and duration 300ms",
            intensity
        );
        self.shock(intensity, Duration::from_millis(300)).await
    }

    /// <p style="background:rgba(255,181,77,0.16);padding:0.75em;">
    /// <strong>Warning:</strong> Shocks the user without any warning. Brat mode.
    /// </p>
    ///
    /// Refer to documentation of `shock_with_warning` for more information.
    pub async fn shock(&self, intensity: u32, duration: Duration) -> Result<(), PiShockError> {
        info!(
            "Shocking user with intensity {} and duration {:#?}",
            intensity, duration
        );
        self.send_operation(Operation::Shock, intensity, duration, false)
            .await
    }

    /// Shocks the user with a short soft warning vibration beforehand.
    /// This is the recommended way to shock someone.
    ///
    /// ```no_run
    /// # tokio_test::block_on(async {
    /// use std::time::Duration;
    /// use pishock_rs::PiShocker;
    /// use pishock_rs::PiShockAccount;
    ///
    /// let pishock_account = PiShockAccount::new("pishock_rs", "username", "apikey");
    /// let pishocker_instance = pishock_account.get_shocker("sharecode".to_string()).await.unwrap();
    ///
    /// // Shock the user with an intensity of 50 and a duration of 2 seconds
    /// pishocker_instance.shock_with_warning(50, Duration::from_secs(2)).await.expect("Failed to shock user");
    /// # })
    /// ```
    /// # Errors
    /// The maximum intensity may be below 100, depending on user settings. Make **sure** that you handle `PiShockError::InvalidIntensity` errors properly.
    ///
    /// The maximum duration is 15 seconds. Make **sure** that you handle `PiShockError::InvalidDuration` errors properly.
    ///
    pub async fn shock_with_warning(
        &self,
        intensity: u32,
        duration: Duration,
    ) -> Result<(), PiShockError> {
        debug!("Sending warning vibration");
        self.send_operation(Operation::Vibrate, 20, Duration::from_secs(1), true)
            .await?;

        tokio::time::sleep(Duration::from_millis(200)).await; // The firmware requires some delay between commands
        debug!("Sending shock");
        self.shock(intensity, duration).await
    }

    /// Set a cooldown for shocks
    ///
    /// The cooldown is a duration that must pass after a shock before the shocker can shock again.
    /// This is useful if you want to prevent the user from spamming the shocker.
    /// Vibrations and beeps are not affected.
    ///
    /// Excessive shocks result in a [`PiShockError::CooldownExceeded`] error.
    /// ```no_run
    /// # use std::time::Duration;
    /// # use pishock_rs::PiShocker;
    /// # use pishock_rs::PiShockAccount;
    /// # tokio_test::block_on(async {
    /// # let pishock_account = PiShockAccount::new("pishock_rs", "username", "apikey");
    /// # let mut pishocker_instance = pishock_account.get_shocker("sharecode".to_string()).await.unwrap();
    /// pishocker_instance.set_shocker_cooldown(Duration::from_millis(500));
    /// # });
    pub fn set_shocker_cooldown(&mut self, cooldown: Duration) {
        self.cooldown = Some(cooldown);
    }

    /// Refreshes the metadata (name, paused state, limits and permissions) of the shocker.
    /// This is done automatically by [`PiShockAccount::get_shocker`] and [`PiShockAccount::get_own_shocker`].
    pub async fn refresh_metadata(&mut self) -> Result<(), errors::PiShockError> {
        if let Some(share_code) = &self.share_code {
            let shared_shockers = self.account.shared_shockers().await?;
            let shocker = find_shared_shocker(&shared_shockers, share_code)?;

            self.client_id = shocker.client_id;
            self.shocker_id = shocker.shocker_id;
            self.metadata = Some(ShockerMetadata::from(shocker));
        } else {
            let devices = self.account.devices().await?;
            let (device, shocker) = find_own_shocker(&devices, self.shocker_id)?;

            self.client_id = device.client_id;
            self.metadata = Some(ShockerMetadata::from_own(device, shocker));
        }

        debug!("Refreshed shocker metadata: {:?}", self.metadata);
        Ok(())
    }

    /// Returns the name of the shocker
    #[must_use]
    pub fn get_shocker_name(&self) -> Option<String> {
        self.metadata.as_ref().map(|metadata| metadata.name.clone())
    }

    /// Returns the username of the shocker's owner
    #[must_use]
    pub fn get_owner(&self) -> Option<String> {
        self.metadata
            .as_ref()
            .map(|metadata| metadata.owner.clone())
    }

    /// Returns the client ID (the ID of the PiShock hub the shocker is connected to)
    #[must_use]
    pub fn get_client_id(&self) -> u64 {
        self.client_id
    }

    /// Returns the shocker ID
    #[must_use]
    pub fn get_shocker_id(&self) -> u64 {
        self.shocker_id
    }

    /// Returns the maximum shock intensity
    #[must_use]
    pub fn get_max_intensity(&self) -> Option<u32> {
        self.metadata
            .as_ref()
            .map(|metadata| metadata.max_intensity)
    }

    /// Returns the maximum duration of a single operation.
    /// The v2 API does not report per-shocker limits, so this is always [`MAX_DURATION`].
    #[must_use]
    pub fn get_max_duration(&self) -> Duration {
        MAX_DURATION
    }

    /// Returns whether the shocker is paused or not
    #[must_use]
    pub fn get_shocker_paused(&self) -> Option<bool> {
        self.metadata.as_ref().map(|metadata| metadata.paused)
    }

    /// Returns whether shocking is permitted
    #[must_use]
    pub fn can_shock(&self) -> Option<bool> {
        self.metadata.as_ref().map(|metadata| metadata.can_shock)
    }

    /// Returns whether vibrating is permitted
    #[must_use]
    pub fn can_vibrate(&self) -> Option<bool> {
        self.metadata.as_ref().map(|metadata| metadata.can_vibrate)
    }

    /// Returns whether beeping is permitted
    #[must_use]
    pub fn can_beep(&self) -> Option<bool> {
        self.metadata.as_ref().map(|metadata| metadata.can_beep)
    }

    /// Returns the cooldown of the shocker
    #[must_use]
    pub fn get_shocker_cooldown(&self) -> Option<Duration> {
        self.cooldown
    }

    // Metadata-supported intensity and duration checks that are used by the internal operation function
    pub(crate) fn max_intensity_error_triggered(&self, intensity: u32) -> Option<PiShockError> {
        let max_intensity = self.get_max_intensity().unwrap_or(MAX_INTENSITY);

        if intensity > max_intensity {
            Some(PiShockError::InvalidIntensity(max_intensity))
        } else {
            None
        }
    }

    pub(crate) fn max_duration_error_triggered(&self, duration: Duration) -> Option<PiShockError> {
        if duration > MAX_DURATION {
            Some(PiShockError::InvalidDuration(MAX_DURATION.as_secs() as u32))
        } else {
            None
        }
    }

    fn operation_permitted(&self, operation: Operation) -> bool {
        match (&self.metadata, operation) {
            (None, _) => true,
            (Some(metadata), Operation::Shock) => metadata.can_shock,
            (Some(metadata), Operation::Vibrate) => metadata.can_vibrate,
            (Some(metadata), Operation::Beep) => metadata.can_beep,
        }
    }

    /// The broker channel of the shocker's hub
    fn target(&self) -> String {
        match &self.share_code {
            Some(share_code) => format!("c{}-sops-{}", self.client_id, share_code),
            None => format!("c{}-ops", self.client_id),
        }
    }

    pub(crate) async fn send_operation(
        &self,
        operation: Operation,
        intensity: u32,
        duration: Duration,
        warning: bool,
    ) -> Result<(), errors::PiShockError> {
        if self.get_shocker_paused() == Some(true) {
            return Err(errors::PiShockError::ShockerPaused);
        }

        if !self.operation_permitted(operation) {
            return Err(errors::PiShockError::PermissionDenied(operation.name()));
        }

        // Check if the duration is outside the permitted range
        if duration < MIN_DURATION {
            return Err(errors::PiShockError::InvalidDuration(
                MAX_DURATION.as_secs() as u32,
            ));
        }
        if let Some(error) = self.max_duration_error_triggered(duration) {
            return Err(error);
        }

        // Check if the intensity is outside the permitted range (beeps have no intensity)
        if operation != Operation::Beep {
            if let Some(error) = self.max_intensity_error_triggered(intensity) {
                return Err(error);
            }

            if intensity < 1 {
                return Err(errors::PiShockError::InvalidIntensity(
                    self.get_max_intensity().unwrap_or(MAX_INTENSITY),
                ));
            }
        }

        let user_id = self.account.user_id().await?;

        // Check shock cooldown and return error if it is not over yet
        if operation == Operation::Shock {
            self.verify_shocker_cooldown()?;
        }

        let command = PublishCommand {
            target: self.target(),
            body: CommandBody {
                id: self.shocker_id,
                m: operation.mode(),
                i: intensity,
                d: duration.as_millis() as u64,
                r: true,
                l: CommandLog {
                    u: user_id,
                    ty: if self.share_code.is_some() {
                        "sc"
                    } else {
                        "api"
                    },
                    w: warning,
                    h: false,
                    o: self.account.app_name.clone(),
                },
            },
        };

        self.account.broker.publish(&[command]).await
    }

    fn verify_shocker_cooldown(&self) -> Result<(), errors::PiShockError> {
        let mut last_shock = self.last_shock.lock().unwrap();

        if let (Some(last_shock), Some(cooldown)) = (last_shock.as_ref(), self.cooldown) {
            if last_shock.elapsed() < cooldown {
                return Err(errors::PiShockError::CooldownExceeded(
                    cooldown - last_shock.elapsed(),
                ));
            }
        }

        *last_shock = Some(Instant::now());

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::Operation;
    use crate::errors::PiShockError;
    use crate::test_utils::{MockBroker, TestServer};
    use serde_json::json;
    use std::time::Duration;
    use test_log::test;

    #[test(tokio::test)]
    async fn metadata_is_loaded_from_share_code() {
        let server = TestServer::start().await;
        let shocker = server.account().get_shocker("sharecode").await.unwrap();

        assert_eq!(shocker.get_shocker_name().unwrap(), "Shocker 1");
        assert_eq!(shocker.get_owner().unwrap(), "owner1");
        assert_eq!(shocker.get_max_intensity().unwrap(), 100);
        assert_eq!(shocker.get_max_duration(), Duration::from_secs(15));
        assert!(!shocker.get_shocker_paused().unwrap());
        assert_eq!(shocker.get_client_id(), 6969);
        assert_eq!(shocker.get_shocker_id(), 789);
        assert_eq!(shocker.get_share_code().unwrap(), "SHARECODE");
    }

    #[test(tokio::test)]
    async fn unknown_share_code_is_rejected() {
        let server = TestServer::start().await;

        let result = server.account().get_shocker("unknown").await;

        assert!(matches!(result, Err(PiShockError::ShareCodeNotFound)));
    }

    #[test(tokio::test)]
    async fn own_shocker_is_found() {
        let server = TestServer::start().await;
        let account = server.account();

        let shocker = account.get_own_shocker(456).await.unwrap();
        assert_eq!(shocker.get_client_id(), 420);
        assert_eq!(shocker.get_shocker_name().unwrap(), "Shocker 2");
        assert_eq!(shocker.get_share_code(), None);

        assert!(matches!(
            account.get_own_shocker(1).await,
            Err(PiShockError::ShockerNotFound(1))
        ));
        assert_eq!(account.get_all_own_shockers().await.unwrap().len(), 3);
    }

    #[test(tokio::test)]
    async fn metadata_is_refreshed() {
        let server = TestServer::start().await;
        let mut shocker = server
            .account()
            .shocker_from_ids(0, 0, Some("sharecode".to_string()));
        assert_eq!(shocker.get_shocker_name(), None);

        shocker.refresh_metadata().await.unwrap();

        assert_eq!(shocker.get_shocker_name().unwrap(), "Shocker 1");
        assert_eq!(shocker.get_client_id(), 6969);
        assert_eq!(shocker.get_shocker_id(), 789);
    }

    macro_rules! successful_operation_tests {
        ($($name:ident: $operation:expr, $mode:expr, $intensity:expr,)*) => {
            $(
                #[test(tokio::test)]
                async fn $name() {
                    let server = TestServer::start().await;
                    let shocker = server.account().get_shocker("sharecode").await.unwrap();

                    shocker
                        .send_operation($operation, $intensity, Duration::from_secs(2), false)
                        .await
                        .expect("Failed to send operation");

                    assert_eq!(server.broker.received(), vec![json!({
                        "Operation": "PUBLISH",
                        "PublishCommands": [{
                            "Target": "c6969-sops-SHARECODE",
                            "Body": {
                                "id": 789,
                                "m": $mode,
                                "i": $intensity,
                                "d": 2000,
                                "r": true,
                                "l": { "u": 1234, "ty": "sc", "w": false, "h": false, "o": "pishock_rs" }
                            }
                        }]
                    })]);
                }
            )*
        }
    }

    successful_operation_tests! {
        test_vibrate: Operation::Vibrate, "v", 50,
        test_shock: Operation::Shock, "s", 50,
        test_beep: Operation::Beep, "b", 0,
    }

    #[test(tokio::test)]
    async fn own_shocker_uses_ops_channel() {
        let server = TestServer::start().await;
        let shocker = server.account().get_own_shocker(456).await.unwrap();

        shocker.mini_shock(10).await.unwrap();

        let command = &server.broker.received()[0]["PublishCommands"][0];
        assert_eq!(command["Target"], "c420-ops");
        assert_eq!(command["Body"]["id"], 456);
        assert_eq!(command["Body"]["d"], 300);
        assert_eq!(command["Body"]["l"]["ty"], "api");
    }

    #[test(tokio::test)]
    async fn shock_with_warning_marks_warning_vibration() {
        let server = TestServer::start().await;
        let shocker = server.account().get_shocker("sharecode").await.unwrap();

        shocker
            .shock_with_warning(30, Duration::from_millis(1500))
            .await
            .unwrap();

        let received = server.broker.received();
        assert_eq!(received.len(), 2);
        let warning = &received[0]["PublishCommands"][0]["Body"];
        assert_eq!(warning["m"], "v");
        assert_eq!(warning["l"]["w"], true);
        let shock = &received[1]["PublishCommands"][0]["Body"];
        assert_eq!(shock["m"], "s");
        assert_eq!(shock["i"], 30);
        assert_eq!(shock["d"], 1500);
        assert_eq!(shock["l"]["w"], false);
        // Both commands share one connection
        assert_eq!(server.broker.connections(), 1);
    }

    #[test(tokio::test)]
    async fn broker_errors_are_returned() {
        let broker = MockBroker::start(|_| {
            Some(json!({"ErrorCode": "SOME_ERROR", "IsError": true, "Message": "Something failed.", "OriginalCommand": ""}))
        })
        .await;
        let server = TestServer::start_with_broker(broker).await;
        let shocker = server.account().get_shocker("sharecode").await.unwrap();

        match shocker.vibrate(10, Duration::from_secs(1)).await {
            Err(PiShockError::BrokerError { code, message }) => {
                assert_eq!(code.as_deref(), Some("SOME_ERROR"));
                assert_eq!(message, "Something failed.");
            }
            other => panic!("Expected BrokerError, got {other:?}"),
        }
    }

    #[test(tokio::test)]
    async fn broker_auth_errors_are_invalid_credentials() {
        // The real broker answers invalid API keys like this and then closes the connection
        let broker = MockBroker::start(|_| {
            Some(json!({
                "ErrorCode": "API_KEY_ERROR",
                "IsError": true,
                "Message": "Api key could not be validated, please ensure it has been generated after 2024-10-15.",
                "OriginalCommand": null,
                "Source": null
            }))
        })
        .await;
        let server = TestServer::start_with_broker(broker).await;
        let shocker = server.account().get_shocker("sharecode").await.unwrap();

        for _ in 0..2 {
            assert!(matches!(
                shocker.beep(Duration::from_secs(1)).await,
                Err(PiShockError::InvalidCredentials)
            ));
        }
        // The rejected connection is not reused
        assert_eq!(server.broker.connections(), 2);
    }

    #[test(tokio::test)]
    async fn lost_connection_is_reopened() {
        let server = TestServer::start().await;
        let shocker = server.account().get_shocker("sharecode").await.unwrap();

        shocker.beep(Duration::from_secs(1)).await.unwrap();
        server.broker.drop_connections();
        shocker.beep(Duration::from_secs(1)).await.unwrap();

        assert_eq!(server.broker.connections(), 2);
    }

    #[test(tokio::test)]
    async fn broker_is_authenticated_with_username_and_api_key() {
        let server = TestServer::start().await;
        let shocker = server.account().get_shocker("sharecode").await.unwrap();

        shocker.beep(Duration::from_secs(1)).await.unwrap();

        assert_eq!(
            server.broker.last_query().unwrap(),
            "Username=username&ApiKey=apikey"
        );
    }

    #[test(tokio::test)]
    async fn invalid_operations_are_rejected_locally() {
        let server = TestServer::start().await;
        let account = server.account();

        // Shared shocker with max intensity 50, shocking not permitted
        let restricted = account.get_shocker("restricted").await.unwrap();
        assert!(matches!(
            restricted.shock(10, Duration::from_secs(1)).await,
            Err(PiShockError::PermissionDenied("shock"))
        ));
        assert!(matches!(
            restricted.vibrate(51, Duration::from_secs(1)).await,
            Err(PiShockError::InvalidIntensity(50))
        ));

        let shocker = account.get_shocker("sharecode").await.unwrap();
        assert!(matches!(
            shocker.vibrate(0, Duration::from_secs(1)).await,
            Err(PiShockError::InvalidIntensity(100))
        ));
        assert!(matches!(
            shocker.vibrate(10, Duration::from_millis(99)).await,
            Err(PiShockError::InvalidDuration(15))
        ));
        assert!(matches!(
            shocker.vibrate(10, Duration::from_millis(15001)).await,
            Err(PiShockError::InvalidDuration(15))
        ));

        let mut paused = shocker.clone();
        paused.metadata.as_mut().unwrap().paused = true;
        assert!(matches!(
            paused.beep(Duration::from_secs(1)).await,
            Err(PiShockError::ShockerPaused)
        ));

        assert!(server.broker.received().is_empty());
    }

    #[test(tokio::test)]
    async fn shock_cooldown_is_enforced() {
        let server = TestServer::start().await;
        let mut shocker = server.account().get_shocker("sharecode").await.unwrap();
        shocker.set_shocker_cooldown(Duration::from_secs(60));

        shocker.shock(10, Duration::from_secs(1)).await.unwrap();
        // Vibrations are not affected by the cooldown
        shocker.vibrate(10, Duration::from_secs(1)).await.unwrap();

        assert!(matches!(
            shocker.shock(10, Duration::from_secs(1)).await,
            Err(PiShockError::CooldownExceeded(_))
        ));
        assert_eq!(server.broker.received().len(), 2);
    }
}
