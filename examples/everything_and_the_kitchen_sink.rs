extern crate pishock_rs;

use log::error;
use pishock_rs::interpolation::ShockPoint;
use pishock_rs::PiShocker;
use simplelog::{Config, LevelFilter, TerminalMode};
use std::process::exit;
use std::time::Duration;

#[tokio::main]
async fn main() {
    simplelog::TermLogger::init(
        LevelFilter::Debug,
        Config::default(),
        TerminalMode::Mixed,
        simplelog::ColorChoice::Auto,
    )
    .unwrap();

    println!("Example using most of the PiShock API - env variable control");

    let shocker_share_code = std::env::var("PISHOCK_SHARECODE").unwrap_or_default();
    let shocker_api_key = std::env::var("PISHOCK_APIKEY").unwrap_or_default();
    let shocker_api_username = std::env::var("PISHOCK_USERNAME").unwrap_or_default();
    // The shock curve is only sent when explicitly requested
    let send_shock_curve = std::env::var("PISHOCK_SHOCK_CURVE").is_ok_and(|value| value == "1");

    println!("Shocker share code (PISHOCK_SHARECODE): {shocker_share_code}");
    println!("Shocker API username (PISHOCK_USERNAME): {shocker_api_username}");

    if shocker_share_code.is_empty()
        || shocker_api_key.is_empty()
        || shocker_api_username.is_empty()
    {
        error!("PISHOCK_SHARECODE, PISHOCK_APIKEY and PISHOCK_USERNAME must be set");
        exit(1);
    }

    // Create a new PiShockAccount instance
    let pishock_account = pishock_rs::PiShockAccount::new(
        "pishock_rs example".to_string(),
        shocker_api_username,
        shocker_api_key,
    );

    println!("User ID: {}", pishock_account.get_user_id().await.unwrap());

    // List the account's own hubs and shockers
    for device in pishock_account.get_devices().await.unwrap() {
        println!("Hub {} (client ID {})", device.name, device.client_id);
        for shocker in device.shockers {
            println!(
                "  Shocker {} (ID {}, paused: {})",
                shocker.name, shocker.shocker_id, shocker.is_paused
            );
        }
    }

    // List the shockers shared with the account
    for shocker in pishock_account.get_shared_shockers().await.unwrap() {
        println!(
            "Shared shocker {} from {} (share code {})",
            shocker.shocker_name, shocker.owner, shocker.share_code
        );
    }

    // Get a PiShocker instance
    let pishocker_instance: PiShocker = match pishock_account.get_shocker(shocker_share_code).await
    {
        Ok(pishock_instance) => pishock_instance,
        Err(e) => {
            error!("Failed to get PiShocker instance: {e}");
            exit(1);
        }
    };

    // Print all the PiShocker's details
    println!("PiShocker details:");
    println!("  Name: {}", pishocker_instance.get_shocker_name().unwrap());
    println!("  Owner: {}", pishocker_instance.get_owner().unwrap());
    println!(
        "  Max intensity: {}",
        pishocker_instance.get_max_intensity().unwrap()
    );
    println!(
        "  Max duration: {:#?}",
        pishocker_instance.get_max_duration()
    );
    println!("  Client ID: {}", pishocker_instance.get_client_id());
    println!("  Shocker ID: {}", pishocker_instance.get_shocker_id());
    println!(
        "  Paused: {}",
        pishocker_instance.get_shocker_paused().unwrap()
    );
    println!(
        "  Permissions: shock {}, vibrate {}, beep {}",
        pishocker_instance.can_shock().unwrap(),
        pishocker_instance.can_vibrate().unwrap(),
        pishocker_instance.can_beep().unwrap()
    );

    pishocker_instance
        .beep(Duration::from_millis(500))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_secs(1)).await;
    pishocker_instance
        .vibrate(20, Duration::from_secs(1))
        .await
        .unwrap();

    if send_shock_curve {
        tokio::time::sleep(Duration::from_secs(2)).await;
        pishocker_instance
            .shock_curve(vec![
                ShockPoint::new(Duration::from_secs(2), 10),
                ShockPoint::new(Duration::from_secs(3), 5),
                ShockPoint::new(Duration::from_secs(1), 1),
            ])
            .await
            .unwrap();
    }
}
