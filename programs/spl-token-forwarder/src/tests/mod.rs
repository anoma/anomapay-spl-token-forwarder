//! Unit tests for the pure parts of the forwarder. Everything that needs a
//! runtime is in the integration tests (crates/integration-test).

mod ed25519_tests;
#[cfg(feature = "emergency-migrating")]
mod emergency_migrating_tests;
mod state_tests;
