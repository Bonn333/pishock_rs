extern crate pishock_rs;

use log::error;
use simplelog::{Config, LevelFilter, TerminalMode};
use std::process::exit;

/// Lists all own and shared shockers of an account. Read-only, no operations are sent.
#[tokio::main]
async fn main() {
    simplelog::TermLogger::init(
        LevelFilter::Info,
        Config::default(),
        TerminalMode::Mixed,
        simplelog::ColorChoice::Auto,
    )
    .unwrap();

    let api_key = std::env::var("PISHOCK_APIKEY").unwrap_or_default();
    let username = std::env::var("PISHOCK_USERNAME").unwrap_or_default();

    if api_key.is_empty() || username.is_empty() {
        error!("PISHOCK_APIKEY and PISHOCK_USERNAME must be set");
        exit(1);
    }

    let pishock_account =
        pishock_rs::PiShockAccount::new("pishock_rs example", &username, &api_key);

    match pishock_account.get_user_id().await {
        Ok(user_id) => println!("Logged in as {username} (user ID {user_id})"),
        Err(e) => {
            error!("Failed to log in: {e}");
            exit(1);
        }
    }

    println!("Own shockers:");
    for shocker in pishock_account.get_all_own_shockers().await.unwrap() {
        println!(
            "  {} (client ID {}, shocker ID {}, paused: {})",
            shocker.get_shocker_name().unwrap(),
            shocker.get_client_id(),
            shocker.get_shocker_id(),
            shocker.get_shocker_paused().unwrap()
        );
    }

    println!("Shared shockers:");
    for shocker in pishock_account.get_shared_shockers().await.unwrap() {
        println!(
            "  {} from {} (share code {}, max intensity {}, shock {}, vibrate {}, beep {}, paused: {})",
            shocker.shocker_name,
            shocker.owner,
            shocker.share_code,
            shocker.max_intensity,
            shocker.can_shock,
            shocker.can_vibrate,
            shocker.can_beep,
            shocker.is_paused
        );
    }
}
