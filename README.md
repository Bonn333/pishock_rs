# pishock_rs

A Rust implementation of the [pishock API](https://pishock.com).

Uses the current PiShock API: the REST endpoints on `auth.pishock.com` / `ps.pishock.com` to look up shockers,
and the v2 WebSocket broker (`broker.pishock.com/v2`) to send operations.

## Usage

```rust
use pishock_rs::PiShockAccount;
use std::time::Duration;

let account = PiShockAccount::new("my app", "username", "apikey");

// A shocker shared with your account
let shocker = account.get_shocker("SHARECODE").await?;
shocker.vibrate(30, Duration::from_millis(1500)).await?;

// One of your own shockers
let own_shocker = account.get_own_shocker(1234).await?;
own_shocker.beep(Duration::from_secs(1)).await?;
```

The API key must have been generated after 2024-10-15, older keys are rejected by the broker.

Check out the [examples](examples) directory for more usage examples.

## Migrating from 2.x

- The legacy `do.pishock.com/api` endpoints are no longer used.
- Share codes must be added to your account on pishock.com before `get_shocker` can find them.
- Own shockers can be used without share codes: `get_devices`, `get_own_shocker` and `get_all_own_shockers`.
- `get_shocker_without_verification` was replaced by `shocker_from_ids`.
- `get_shocker_online` was removed, the API no longer reports it.
- `get_client_id` and `get_shocker_id` return `u64`, `get_max_intensity` returns `Option<u32>`
  and `get_max_duration` always returns the 15 second limit.
- Durations are sent in milliseconds, they are no longer rounded down to whole seconds above one second.
- The shock cooldown only applies to shocks, vibrations and beeps are not affected.
- Errors: `ShareCodeInUse`, `InvalidOpCode` and `ShockerOffline` were removed;
  `ShockerNotFound`, `PermissionDenied`, `BrokerError` and `Timeout` were added.

## License
See [LICENSE](LICENSE.md) for details.
