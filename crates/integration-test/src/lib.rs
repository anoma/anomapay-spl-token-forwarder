//! The SPL token forwarder's tests on the adapter's harness: the forwarder
//! deployed and serving a mint on the harness's environments, the AnomaPay
//! wrap and unwrap actions, and how a submitter passes the forwarder its
//! calls' accounts.

pub mod fixtures;
pub mod logic;
pub mod refusal;
pub mod setup;
pub mod submitter;
