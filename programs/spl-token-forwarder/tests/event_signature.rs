//! Every event's discriminator is the hash of its signature, read from the
//! committed IDLs of both builds, so a change to an event's name or field
//! types changes its discriminator.

use anchor_lang::idl::types::Idl;
use protocol_adapter::events::{event_discriminator, idl_event_signature};

fn committed_idl(name: &str, json: &str) -> Idl {
    serde_json::from_str(json).unwrap_or_else(|e| panic!("{name} is not an Anchor IDL: {e}"))
}

fn assert_every_event_discriminator_is_its_signatures(name: &str, idl: &Idl) {
    assert!(!idl.events.is_empty(), "{name} declares no events");
    for event in &idl.events {
        let signature = idl_event_signature(idl, &event.name);
        let expected = event_discriminator(&signature);
        println!(
            "{name} {}: signature {signature}, IDL discriminator {:?}, sha256(\"event:\" ++ signature)[..8] {expected:?}",
            event.name, event.discriminator
        );
        assert_eq!(
            event.discriminator, expected,
            "{name} event {}: the IDL's discriminator {:?} is not that of its signature {signature}, {expected:?}",
            event.name, event.discriminator
        );
    }
}

#[test]
fn every_event_discriminator_is_its_idl_signatures() {
    for (name, json) in [
        (
            "spl_token_forwarder.json",
            include_str!("../../../crates/client/idls/spl_token_forwarder.json"),
        ),
        (
            "emergency_migrating_spl_token_forwarder.json",
            include_str!(
                "../../../crates/client/idls/emergency_migrating_spl_token_forwarder.json"
            ),
        ),
    ] {
        assert_every_event_discriminator_is_its_signatures(name, &committed_idl(name, json));
    }
}
