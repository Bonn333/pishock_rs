# list_shockers

Read-only, lists all own and shared shockers of the account.

```bash
$ PISHOCK_APIKEY=apikey PISHOCK_USERNAME=username cargo run --example list_shockers
```

# env_shock

```bash
$ PISHOCK_SHARECODE=sharecode PISHOCK_APIKEY=apikey PISHOCK_USERNAME=username PISHOCK_INTENSITY=30 PISHOCK_DURATION=1 cargo run --example env_shock
```

# everything_and_the_kitchen_sink

Lists devices and shared shockers, prints the shocker's details, then beeps and vibrates.
Set `PISHOCK_SHOCK_CURVE=1` to also send a low-intensity shock curve.

```bash
$ PISHOCK_SHARECODE=sharecode PISHOCK_APIKEY=apikey PISHOCK_USERNAME=username cargo run --example everything_and_the_kitchen_sink
```
