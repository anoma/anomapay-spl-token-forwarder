//! Tests for the tokens the emergency-migrating build holds.

use crate::emergency_migrating::holds_the_tokens_of;
use anchor_lang::prelude::Pubkey;

/// Mirrors test_migrate_accepts_a_resource_with_the_legacy_forwarder_in_its_label
/// and test_migrate_reverts_on_the_zero_forwarder_on_a_chain_without_a_legacy_forwarder:
/// the forwarder holds the tokens of its own resources and of the legacy
/// forwarder's, and on a cluster without one the zero key names none. The
/// local build names no legacy forwarder, so the integration tests reach only
/// the second.
#[test]
fn the_forwarder_holds_its_own_and_the_legacy_forwarders_tokens() {
    let (legacy, other) = (Pubkey::new_unique(), Pubkey::new_unique());
    assert!(holds_the_tokens_of(&crate::ID, &legacy));
    assert!(holds_the_tokens_of(&legacy, &legacy));
    assert!(!holds_the_tokens_of(&other, &legacy));

    let none = Pubkey::default();
    assert!(holds_the_tokens_of(&crate::ID, &none));
    assert!(!holds_the_tokens_of(&none, &none));
    assert!(!holds_the_tokens_of(&other, &none));
}
