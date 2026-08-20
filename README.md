# crisclip

Clipboard sync between two computers on the same LAN. Text and images. No server, no account.

## Install

```sh
cargo install --path .
crisclip init      # writes ~/.config/crisclip/config.toml with a fresh key
crisclip run
```

Copy that config to both machines, point each `peer` at the other, keep `key` identical.
Run it as a service with `systemctl --user enable --now crisclip`.

```toml
peer = "192.168.15.5:47777"
listen = "0.0.0.0:47777"
key = "..."             # 64 hex chars, same on both
poll_ms = 2000
max_bytes = 33554432
```

## Tests

```sh
cargo test                # protocol, framing, PNG
cargo test -- --ignored   # needs a graphical session
```
