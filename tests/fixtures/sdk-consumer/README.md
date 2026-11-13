# Path-based SDK consumer preparation

This standalone Cargo workspace currently uses local path dependencies. It checks the lean codec and in-memory plan, and its `native` feature exercises the selected setup path against temporary local fixtures. It does not prove independent consumption; P05b must pin both producer packages to the same immutable commit and validate outside this checkout.

Run the lean executable with `cargo run --manifest-path tests/fixtures/sdk-consumer/Cargo.toml`. Run native fixture tests with `cargo test --manifest-path tests/fixtures/sdk-consumer/Cargo.toml --features native`. Use an isolated target directory for validation.
