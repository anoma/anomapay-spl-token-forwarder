//! Decoders for the SPL Token Forwarder's events.
//!
//! The forwarder emits every event as a self-invocation (Anchor
//! `#[event_cpi]`), framed as the adapter frames its own: an inner instruction
//! whose program is the forwarder and whose data is the 8-byte event tag, the
//! event's 8-byte discriminator (`sha256("event:" ++ signature)[..8]`, the
//! signature being the event's name and its field types in order), and the
//! Borsh-encoded body. The event types and their discriminators are the
//! generated ones ([`crate::generated`]). Readers pass each forwarder-addressed
//! inner instruction of a settlement through
//! [`decode_forwarder_event_instruction`].

use anchor_lang::{AnchorDeserialize, Discriminator, Event};
use anoma_pa_solana_client::cursor::Cursor;
use anoma_pa_solana_client::events::{decode_cpi_event, EventDecodeError};
use anoma_pa_solana_client::ANCHOR_DISCRIMINATOR_LEN;

use crate::generated::emergency_migrating_spl_token_forwarder::events::{
    LogicRefReplaced, Migrated, VulnerableLogicRefListed,
};
use crate::generated::spl_token_forwarder::events::{
    Initialized, OwnershipTransferred, Unwrapped, Upgraded, Wrapped,
};

/// One decoded SPL Token Forwarder event, of either build: the production
/// build's events and the emergency-migrating build's own. Two events are
/// equal when their discriminators and bodies are.
#[derive(Clone, Debug)]
pub enum ForwarderEvent {
    Wrapped(Wrapped),
    Unwrapped(Unwrapped),
    Initialized(Initialized),
    OwnershipTransferred(OwnershipTransferred),
    Upgraded(Upgraded),
    LogicRefReplaced(LogicRefReplaced),
    Migrated(Migrated),
    VulnerableLogicRefListed(VulnerableLogicRefListed),
}

impl ForwarderEvent {
    /// The event's discriminator and Borsh body, as the program emits them.
    fn data(&self) -> Vec<u8> {
        match self {
            ForwarderEvent::Wrapped(e) => e.data(),
            ForwarderEvent::Unwrapped(e) => e.data(),
            ForwarderEvent::Initialized(e) => e.data(),
            ForwarderEvent::OwnershipTransferred(e) => e.data(),
            ForwarderEvent::Upgraded(e) => e.data(),
            ForwarderEvent::LogicRefReplaced(e) => e.data(),
            ForwarderEvent::Migrated(e) => e.data(),
            ForwarderEvent::VulnerableLogicRefListed(e) => e.data(),
        }
    }
}

impl PartialEq for ForwarderEvent {
    fn eq(&self, other: &Self) -> bool {
        self.data() == other.data()
    }
}

impl Eq for ForwarderEvent {}

/// Decode the instruction data of one SPL Token Forwarder event self-invocation.
pub fn decode_forwarder_event_instruction(data: &[u8]) -> Result<ForwarderEvent, EventDecodeError> {
    decode_cpi_event(data, forwarder_event_body)
}

/// The Borsh body of the event `name`: the rest of the instruction data,
/// which must hold exactly one `E`.
fn body<E: AnchorDeserialize>(
    c: &mut Cursor<'_>,
    name: &'static str,
) -> Result<E, EventDecodeError> {
    let mut rest = c.take(c.remaining(), name)?;
    let event =
        E::deserialize(&mut rest).map_err(|_| EventDecodeError::Truncated { field: name })?;
    match rest.len() {
        0 => Ok(event),
        n => Err(EventDecodeError::TrailingBytes(n)),
    }
}

fn forwarder_event_body(
    disc: [u8; ANCHOR_DISCRIMINATOR_LEN],
    c: &mut Cursor<'_>,
) -> Result<ForwarderEvent, EventDecodeError> {
    let d = disc.as_slice();
    Ok(if d == Wrapped::DISCRIMINATOR {
        ForwarderEvent::Wrapped(body(c, "Wrapped")?)
    } else if d == Unwrapped::DISCRIMINATOR {
        ForwarderEvent::Unwrapped(body(c, "Unwrapped")?)
    } else if d == Initialized::DISCRIMINATOR {
        ForwarderEvent::Initialized(body(c, "Initialized")?)
    } else if d == OwnershipTransferred::DISCRIMINATOR {
        ForwarderEvent::OwnershipTransferred(body(c, "OwnershipTransferred")?)
    } else if d == Upgraded::DISCRIMINATOR {
        ForwarderEvent::Upgraded(body(c, "Upgraded")?)
    } else if d == LogicRefReplaced::DISCRIMINATOR {
        ForwarderEvent::LogicRefReplaced(body(c, "LogicRefReplaced")?)
    } else if d == Migrated::DISCRIMINATOR {
        ForwarderEvent::Migrated(body(c, "Migrated")?)
    } else if d == VulnerableLogicRefListed::DISCRIMINATOR {
        ForwarderEvent::VulnerableLogicRefListed(body(c, "VulnerableLogicRefListed")?)
    } else {
        return Err(EventDecodeError::UnknownDiscriminator(disc));
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use anchor_lang::prelude::Pubkey;
    use anoma_pa_solana_client::{anchor_event_disc, EVENT_IX_TAG};

    /// The event tag, the emergency-migrating build's IDL's discriminator of
    /// `name` (its events are the production build's and its own), then the
    /// body.
    fn event(name: &str, body: &[&[u8]]) -> Vec<u8> {
        let idl = crate::idl::emergency_migrating_idl();
        let disc = idl["events"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["name"] == name)
            .unwrap_or_else(|| panic!("the IDL declares no event {name}"))["discriminator"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b.as_u64().unwrap() as u8)
            .collect::<Vec<u8>>();
        let mut data = EVENT_IX_TAG.to_vec();
        data.extend(disc);
        body.iter().for_each(|field| data.extend(*field));
        data
    }

    #[test]
    fn each_event_decodes_to_its_fields() {
        let key = |b: u8| Pubkey::new_from_array([b; 32]);
        let cases = [
            (
                event("Wrapped", &[&[1; 32], &[2; 32], &7u64.to_le_bytes()]),
                ForwarderEvent::Wrapped(Wrapped {
                    token: key(1),
                    from: key(2),
                    amount: 7,
                }),
            ),
            (
                event("Unwrapped", &[&[1; 32], &[4; 32], &5u64.to_le_bytes()]),
                ForwarderEvent::Unwrapped(Unwrapped {
                    token: key(1),
                    to: key(4),
                    amount: 5,
                }),
            ),
            (
                event("Initialized", &[&3u64.to_le_bytes()]),
                ForwarderEvent::Initialized(Initialized { version: 3 }),
            ),
            (
                event("OwnershipTransferred", &[&[0; 32], &[9; 32]]),
                ForwarderEvent::OwnershipTransferred(OwnershipTransferred {
                    previous_owner: key(0),
                    new_owner: key(9),
                }),
            ),
            (
                event("Upgraded", &[&[5; 32]]),
                ForwarderEvent::Upgraded(Upgraded {
                    implementation_hash: [5; 32],
                }),
            ),
            (
                event("LogicRefReplaced", &[&[1; 32], &[2; 32]]),
                ForwarderEvent::LogicRefReplaced(LogicRefReplaced {
                    previous_logic_ref: [1; 32],
                    new_logic_ref: [2; 32],
                }),
            ),
            (
                event("Migrated", &[&[1; 32], &[2; 32], &[3; 32]]),
                ForwarderEvent::Migrated(Migrated {
                    token: key(1),
                    vulnerable_logic_ref: [2; 32],
                    nullifier: [3; 32],
                }),
            ),
            (
                event("VulnerableLogicRefListed", &[&[4; 32]]),
                ForwarderEvent::VulnerableLogicRefListed(VulnerableLogicRefListed {
                    logic_ref: [4; 32],
                }),
            ),
        ];
        for (data, expected) in &cases {
            let decoded = decode_forwarder_event_instruction(data);
            println!("{data:?} decodes to {decoded:?}");
            assert_eq!(
                decoded.as_ref(),
                Ok(expected),
                "{data:?} does not decode to {expected:?}"
            );
        }
        // The cases are the events of the emergency-migrating build, which
        // emits the production build's and its own: a variant for an event
        // neither build emits fails here.
        let decoded: std::collections::BTreeSet<String> = cases
            .iter()
            .map(|(_, e)| format!("{e:?}").split('(').next().unwrap().to_string())
            .collect();
        let idl = crate::idl::emergency_migrating_idl();
        let declared: std::collections::BTreeSet<String> = idl["events"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["name"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(decoded, declared, "the decoded events are not the IDL's");
    }

    /// The discriminator over the name alone is not the event's: a reader
    /// cannot take another layout of the event for this one.
    #[test]
    fn a_discriminator_over_the_name_alone_is_unknown() {
        let mut data = EVENT_IX_TAG.to_vec();
        data.extend(anchor_event_disc("Wrapped"));
        data.extend([0; 72]);
        assert_eq!(
            decode_forwarder_event_instruction(&data),
            Err(EventDecodeError::UnknownDiscriminator(anchor_event_disc(
                "Wrapped"
            )))
        );
    }

    /// The Borsh width of an IDL field type the forwarder's events use.
    fn idl_width(ty: &serde_json::Value) -> usize {
        match ty {
            serde_json::Value::String(t) if t == "pubkey" => 32,
            serde_json::Value::String(t) if t == "u64" => 8,
            _ if ty["array"] == serde_json::json!(["u8", 32]) => 32,
            _ => panic!("no width for the IDL type {ty}"),
        }
    }

    /// The decoder reads every event the emergency-migrating build's IDL
    /// declares (the production build's and its own) as a body of exactly
    /// the IDL's fields' widths: one byte short is truncated, naming the
    /// event, and one byte more is a trailing byte.
    #[test]
    fn every_event_the_idl_declares_decodes_a_body_of_its_fields_widths() {
        let idl = crate::idl::emergency_migrating_idl();
        for e in idl["events"].as_array().unwrap() {
            let name = e["name"].as_str().unwrap();
            let width: usize = crate::idl::fields(&idl, name)
                .iter()
                .map(|f| idl_width(&f["type"]))
                .sum();
            let body = vec![0; width];
            let short = decode_forwarder_event_instruction(&event(name, &[&body[..width - 1]]));
            assert!(
                matches!(short, Err(EventDecodeError::Truncated { field }) if field == name),
                "{name}: a {}-byte body decodes to {short:?}",
                width - 1
            );
            let exact = decode_forwarder_event_instruction(&event(name, &[&body]));
            assert!(
                exact.is_ok(),
                "{name}: the IDL's {width}-byte body decodes to {exact:?}"
            );
            let long = decode_forwarder_event_instruction(&event(name, &[&body, &[0]]));
            assert_eq!(
                long,
                Err(EventDecodeError::TrailingBytes(1)),
                "{name}: a {}-byte body",
                width + 1
            );
        }
    }
}
