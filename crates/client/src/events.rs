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

/// The emergency committee (`set_by`) named `emergency_caller` as the
/// forwarder's emergency caller, as the EVM V1 forwarder's
/// `EmergencyCallerSet`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmergencyCallerSetEvent {
    pub emergency_caller: [u8; 32],
    pub set_by: [u8; 32],
}

/// The emergency caller (`caller`) moved `amount` of `token_mint` from escrow
/// to `to`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmergencyWithdrawEvent {
    pub token_mint: [u8; 32],
    pub to: [u8; 32],
    pub amount: u64,
    pub caller: [u8; 32],
}

/// `initialize` or `reinitialize` set the forwarder's configuration to
/// `version`, as OpenZeppelin Initializable's `Initialized(version)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InitializedEvent {
    pub version: u64,
}

/// One decoded SPL Token Forwarder event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ForwarderEvent {
    Wrapped(WrappedEvent),
    Unwrapped(UnwrappedEvent),
    EmergencyCallerSet(EmergencyCallerSetEvent),
    EmergencyWithdraw(EmergencyWithdrawEvent),
    Initialized(InitializedEvent),
    OwnershipTransferred(OwnershipTransferredEvent),
    Upgraded(UpgradedEvent),
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
    } else if disc == anchor_event_disc("EmergencyCallerSet") {
        ForwarderEvent::EmergencyCallerSet(EmergencyCallerSetEvent {
            emergency_caller: c.array_32("emergency_caller")?,
            set_by: c.array_32("set_by")?,
        })
    } else if disc == anchor_event_disc("EmergencyWithdraw") {
        ForwarderEvent::EmergencyWithdraw(EmergencyWithdrawEvent {
            token_mint: c.array_32("token_mint")?,
            to: c.array_32("to")?,
            amount: c.u64_le("amount")?,
            caller: c.array_32("caller")?,
        })
    } else if disc == anchor_event_disc("Initialized") {
        ForwarderEvent::Initialized(InitializedEvent {
            version: c.u64_le("version")?,
        })
    } else if disc == anchor_event_disc("OwnershipTransferred") {
        ForwarderEvent::OwnershipTransferred(decode_ownership_transferred(c)?)
    } else if disc == anchor_event_disc("Upgraded") {
        ForwarderEvent::Upgraded(decode_upgraded(c)?)
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
                event("EmergencyCallerSet", &[&[6; 32], &[7; 32]]),
                ForwarderEvent::EmergencyCallerSet(EmergencyCallerSetEvent {
                    emergency_caller: [6; 32],
                    set_by: [7; 32],
                }),
            ),
            (
                event(
                    "EmergencyWithdraw",
                    &[&[1; 32], &[4; 32], &8u64.to_le_bytes(), &[6; 32]],
                ),
                ForwarderEvent::EmergencyWithdraw(EmergencyWithdrawEvent {
                    token_mint: [1; 32],
                    to: [4; 32],
                    amount: 8,
                    caller: [6; 32],
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
        ];
        for (data, expected) in cases {
            assert_eq!(decode_forwarder_event_instruction(&data).unwrap(), expected);
        }
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

    /// The decoder reads every event the IDL declares field by field, in the
    /// IDL's order and widths: a body that ends inside a field names that
    /// field, and the whole body decodes.
    #[test]
    fn every_event_the_idl_declares_decodes_its_fields_in_order() {
        let idl: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/idl/spl_token_forwarder.json"
        )))
        .unwrap();
        for e in idl["events"].as_array().unwrap() {
            let name = e["name"].as_str().unwrap();
            let ty = idl["types"]
                .as_array()
                .unwrap()
                .iter()
                .find(|t| t["name"] == name)
                .unwrap_or_else(|| panic!("the IDL declares no type for the event {name}"));
            let mut body = Vec::new();
            for f in ty["type"]["fields"].as_array().unwrap() {
                let field = f["name"].as_str().unwrap();
                let width = idl_width(&f["type"]);
                body.resize(body.len() + width - 1, 0);
                let decoded = decode_forwarder_event_instruction(&event(name, &[&body]));
                assert!(
                    matches!(decoded, Err(EventDecodeError::Truncated { field: f }) if f == field),
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
