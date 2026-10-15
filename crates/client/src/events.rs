//! Decoders for the SPL Token Forwarder's events.
//!
//! The forwarder emits every event as a self-invocation (Anchor
//! `#[event_cpi]`), framed as the adapter frames its own: an inner instruction
//! whose program is the forwarder and whose data is the 8-byte event tag, the
//! event's 8-byte discriminator (`sha256("event:<Name>")[..8]`), and the
//! Borsh-encoded body. Readers pass each forwarder-addressed inner instruction
//! of a settlement through [`decode_forwarder_event_instruction`].

use anoma_pa_solana_client::cursor::Cursor;
use anoma_pa_solana_client::events::{
    decode_cpi_event, decode_ownership_transferred, decode_upgraded, EventDecodeError,
    OwnershipTransferredEvent, UpgradedEvent,
};
use anoma_pa_solana_client::{anchor_event_disc, ANCHOR_DISCRIMINATOR_LEN};

/// The forwarder escrowed `amount` of the token whose mint is `token` from
/// `from`, as the EVM forwarder's `Wrapped`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WrappedEvent {
    pub token: [u8; 32],
    pub from: [u8; 32],
    pub amount: u64,
}

/// The forwarder released `amount` of the token whose mint is `token` to
/// `to`, as the EVM forwarder's `Unwrapped`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnwrappedEvent {
    pub token: [u8; 32],
    pub to: [u8; 32],
    pub amount: u64,
}

/// `initialize` or `reinitialize` set the forwarder's configuration to
/// `version`, as OpenZeppelin Initializable's `Initialized(version)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InitializedEvent {
    pub version: u64,
}

/// The emergency-migrating build's `reinitialize` replaced the logic ref
/// `previous_logic_ref` with `new_logic_ref`, as the EVM draft's
/// `LogicRefReplaced`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogicRefReplacedEvent {
    pub previous_logic_ref: [u8; 32],
    pub new_logic_ref: [u8; 32],
}

/// The emergency-migrating build migrated the resource with `nullifier`
/// and the logic ref `vulnerable_logic_ref`, whose token's mint is `token`,
/// as the EVM draft's `Migrated`. No tokens moved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MigratedEvent {
    pub token: [u8; 32],
    pub vulnerable_logic_ref: [u8; 32],
    pub nullifier: [u8; 32],
}

/// The emergency-migrating build's `reinitialize` listed `logic_ref` as
/// vulnerable, as the EVM draft's `VulnerableLogicRefListed`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VulnerableLogicRefListedEvent {
    pub logic_ref: [u8; 32],
}

/// One decoded SPL Token Forwarder event, of either build.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ForwarderEvent {
    Wrapped(WrappedEvent),
    Unwrapped(UnwrappedEvent),
    Initialized(InitializedEvent),
    OwnershipTransferred(OwnershipTransferredEvent),
    Upgraded(UpgradedEvent),
    LogicRefReplaced(LogicRefReplacedEvent),
    Migrated(MigratedEvent),
    VulnerableLogicRefListed(VulnerableLogicRefListedEvent),
}

/// Decode the instruction data of one SPL Token Forwarder event self-invocation.
pub fn decode_forwarder_event_instruction(data: &[u8]) -> Result<ForwarderEvent, EventDecodeError> {
    decode_cpi_event(data, forwarder_event_body)
}

fn forwarder_event_body(
    disc: [u8; ANCHOR_DISCRIMINATOR_LEN],
    c: &mut Cursor<'_>,
) -> Result<ForwarderEvent, EventDecodeError> {
    Ok(if disc == anchor_event_disc("Wrapped") {
        ForwarderEvent::Wrapped(WrappedEvent {
            token: c.array_32("token")?,
            from: c.array_32("from")?,
            amount: c.u64_le("amount")?,
        })
    } else if disc == anchor_event_disc("Unwrapped") {
        ForwarderEvent::Unwrapped(UnwrappedEvent {
            token: c.array_32("token")?,
            to: c.array_32("to")?,
            amount: c.u64_le("amount")?,
        })
    } else if disc == anchor_event_disc("Initialized") {
        ForwarderEvent::Initialized(InitializedEvent {
            version: c.u64_le("version")?,
        })
    } else if disc == anchor_event_disc("OwnershipTransferred") {
        ForwarderEvent::OwnershipTransferred(decode_ownership_transferred(c)?)
    } else if disc == anchor_event_disc("Upgraded") {
        ForwarderEvent::Upgraded(decode_upgraded(c)?)
    } else if disc == anchor_event_disc("LogicRefReplaced") {
        ForwarderEvent::LogicRefReplaced(LogicRefReplacedEvent {
            previous_logic_ref: c.array_32("previous_logic_ref")?,
            new_logic_ref: c.array_32("new_logic_ref")?,
        })
    } else if disc == anchor_event_disc("Migrated") {
        ForwarderEvent::Migrated(MigratedEvent {
            token: c.array_32("token")?,
            vulnerable_logic_ref: c.array_32("vulnerable_logic_ref")?,
            nullifier: c.array_32("nullifier")?,
        })
    } else if disc == anchor_event_disc("VulnerableLogicRefListed") {
        ForwarderEvent::VulnerableLogicRefListed(VulnerableLogicRefListedEvent {
            logic_ref: c.array_32("logic_ref")?,
        })
    } else {
        return Err(EventDecodeError::UnknownDiscriminator(disc));
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use anoma_pa_solana_client::EVENT_IX_TAG;

    fn event(name: &str, body: &[&[u8]]) -> Vec<u8> {
        let mut data = EVENT_IX_TAG.to_vec();
        data.extend(anchor_event_disc(name));
        body.iter().for_each(|field| data.extend(*field));
        data
    }

    #[test]
    fn each_event_decodes_to_its_fields() {
        let cases = [
            (
                event("Wrapped", &[&[1; 32], &[2; 32], &7u64.to_le_bytes()]),
                ForwarderEvent::Wrapped(WrappedEvent {
                    token: [1; 32],
                    from: [2; 32],
                    amount: 7,
                }),
            ),
            (
                event("Unwrapped", &[&[1; 32], &[4; 32], &5u64.to_le_bytes()]),
                ForwarderEvent::Unwrapped(UnwrappedEvent {
                    token: [1; 32],
                    to: [4; 32],
                    amount: 5,
                }),
            ),
            (
                event("Initialized", &[&3u64.to_le_bytes()]),
                ForwarderEvent::Initialized(InitializedEvent { version: 3 }),
            ),
            (
                event("OwnershipTransferred", &[&[0; 32], &[9; 32]]),
                ForwarderEvent::OwnershipTransferred(OwnershipTransferredEvent {
                    previous_owner: [0; 32],
                    new_owner: [9; 32],
                }),
            ),
            (
                event("Upgraded", &[&[5; 32]]),
                ForwarderEvent::Upgraded(UpgradedEvent {
                    executable_hash: [5; 32],
                }),
            ),
            (
                event("LogicRefReplaced", &[&[1; 32], &[2; 32]]),
                ForwarderEvent::LogicRefReplaced(LogicRefReplacedEvent {
                    previous_logic_ref: [1; 32],
                    new_logic_ref: [2; 32],
                }),
            ),
            (
                event("Migrated", &[&[1; 32], &[2; 32], &[3; 32]]),
                ForwarderEvent::Migrated(MigratedEvent {
                    token: [1; 32],
                    vulnerable_logic_ref: [2; 32],
                    nullifier: [3; 32],
                }),
            ),
            (
                event("VulnerableLogicRefListed", &[&[4; 32]]),
                ForwarderEvent::VulnerableLogicRefListed(VulnerableLogicRefListedEvent {
                    logic_ref: [4; 32],
                }),
            ),
        ];
        for (data, expected) in &cases {
            assert_eq!(&decode_forwarder_event_instruction(data).unwrap(), expected);
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
        assert_eq!(
            decode_forwarder_event_instruction(&event("Paused", &[])),
            Err(EventDecodeError::UnknownDiscriminator(anchor_event_disc(
                "Paused"
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
    /// declares (the production build's and its own) field by field, in the
    /// IDL's order and widths: a body that ends inside a field names that
    /// field, and the whole body decodes.
    #[test]
    fn every_event_the_idl_declares_decodes_its_fields_in_order() {
        let idl = crate::idl::emergency_migrating_idl();
        for e in idl["events"].as_array().unwrap() {
            let name = e["name"].as_str().unwrap();
            let mut body = Vec::new();
            for f in crate::idl::fields(&idl, name) {
                let field = f["name"].as_str().unwrap();
                let width = idl_width(&f["type"]);
                body.resize(body.len() + width - 1, 0);
                let decoded = decode_forwarder_event_instruction(&event(name, &[&body]));
                assert!(
                    matches!(decoded, Err(EventDecodeError::Truncated { field: got }) if got == field),
                    "{name}: a body ending inside {field} decodes to {decoded:?}"
                );
                body.push(0);
            }
            assert!(
                decode_forwarder_event_instruction(&event(name, &[&body])).is_ok(),
                "{name}: the IDL's {}-byte body does not decode",
                body.len()
            );
        }
    }
}
