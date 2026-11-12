# Contributing

Keep the library presentation-only and composable. Window/process lifecycle and
adapter selection belong in an explicit application composition root. Source,
history, persistence, discovery, annotation, and clipboard effects must remain
behind Vellum contracts.

Required local gate:

```sh
cargo fmt --check
cargo test --all-targets --locked --offline
cargo clippy --all-targets --locked --offline -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo doc --no-deps --locked --offline
```

The `desktop-qualification` feature opens a real window and is a separate,
deliberate gate. Changes to `vendor/` require exact upstream provenance,
license retention, a narrow diff, adversarial tests, and supply-chain review.
Do not add crates.io publication or remote effects without an approved release
change.
