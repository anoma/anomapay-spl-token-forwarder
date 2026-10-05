//! Decoders for the forwarder's accounts: its configuration and its users'
//! nonce bitmaps. Cursor-based like the adapter's state decoder: the Borsh
//! fields in order, no hardcoded offsets, each account refused unless it
//! carries its type's Anchor discriminator.

use anoma_pa_solana_client::cursor::{Cursor, Truncated};
use anoma_pa_solana_client::{anchor_account_disc, ANCHOR_DISCRIMINATOR_LEN};

use crate::constants::NONCES_PER_WORD;

/// The forwarder's configuration (`Config`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigAccount {
    /// The adapter whose calls the forwarder accepts.
    pub protocol_adapter: [u8; 32],
    /// The logic ref of the resource whose calls the forwarder accepts.
    pub logic_ref: [u8; 32],
    pub emergency_committee: [u8; 32],
    /// All zeros until the committee names one.
    pub emergency_caller: [u8; 32],
    /// The version `initialize` or `reinitialize` last recorded.
    pub version: u64,
    /// All zeros once renounced.
    pub owner: [u8; 32],
}

/// A user's nonce bitmap for one word of 256 wrap nonces (`NonceBitmap`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NonceBitmapAccount {
    pub bits: [u8; 32],
    /// The canonical bump of the bitmap's address.
    pub bump: u8,
}

impl NonceBitmapAccount {
    /// Whether a wrap has used `nonce`, which must fall in this bitmap's word.
    pub fn is_used(&self, nonce: u64) -> bool {
        let bit = nonce % NONCES_PER_WORD;
        self.bits[(bit / 8) as usize] & (1 << (bit % 8)) != 0
    }
}

/// Errors produced by the account decoders.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AccountDecodeError {
    /// The data does not start with the account type's discriminator.
    WrongDiscriminator { expected: &'static str },
    /// The data ran out while reading the named field.
    Truncated { field: &'static str },
}

impl core::fmt::Display for AccountDecodeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            AccountDecodeError::WrongDiscriminator { expected } => {
                write!(f, "the account is not a {expected}")
            }
            AccountDecodeError::Truncated { field } => {
                write!(f, "account truncated while reading {field}")
            }
        }
    }
}

impl std::error::Error for AccountDecodeError {}

impl From<Truncated> for AccountDecodeError {
    fn from(t: Truncated) -> Self {
        AccountDecodeError::Truncated { field: t.field }
    }
}

/// The cursor over `data`'s fields, once its discriminator is `name`'s.
fn fields<'a>(data: &'a [u8], name: &'static str) -> Result<Cursor<'a>, AccountDecodeError> {
    if data.get(..ANCHOR_DISCRIMINATOR_LEN) != Some(&anchor_account_disc(name)[..]) {
        return Err(AccountDecodeError::WrongDiscriminator { expected: name });
    }
    Ok(Cursor::new(data, ANCHOR_DISCRIMINATOR_LEN))
}

/// Decode the forwarder's `Config` account.
pub fn decode_config(data: &[u8]) -> Result<ConfigAccount, AccountDecodeError> {
    let mut c = fields(data, "Config")?;
    Ok(ConfigAccount {
        protocol_adapter: c.array_32("protocol_adapter")?,
        logic_ref: c.array_32("logic_ref")?,
        emergency_committee: c.array_32("emergency_committee")?,
        emergency_caller: c.array_32("emergency_caller")?,
        version: c.u64_le("version")?,
        owner: c.array_32("owner")?,
    })
}

/// Decode a `NonceBitmap` account.
pub fn decode_nonce_bitmap(data: &[u8]) -> Result<NonceBitmapAccount, AccountDecodeError> {
    let mut c = fields(data, "NonceBitmap")?;
    Ok(NonceBitmapAccount {
        bits: c.array_32("bits")?,
        bump: c.u8("bump")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_idl_declares_the_accounts_the_decoders_read() {
        let idl: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/idl/spl_token_forwarder.json"
        )))
        .unwrap();
        let fields_of = |name: &str| -> Vec<String> {
            let ty = idl["types"]
                .as_array()
                .unwrap()
                .iter()
                .find(|t| t["name"] == name)
                .unwrap();
            ty["type"]["fields"]
                .as_array()
                .unwrap()
                .iter()
                .map(|f| f["name"].as_str().unwrap().to_string())
                .collect()
        };
        assert_eq!(
            fields_of("Config"),
            [
                "protocol_adapter",
                "logic_ref",
                "emergency_committee",
                "emergency_caller",
                "version",
                "owner"
            ]
        );
        assert_eq!(fields_of("NonceBitmap"), ["bits", "bump"]);
        for name in ["Config", "NonceBitmap"] {
            let account = idl["accounts"]
                .as_array()
                .unwrap()
                .iter()
                .find(|a| a["name"] == name)
                .unwrap();
            let expected: Vec<u8> =
                serde_json::from_value(account["discriminator"].clone()).unwrap();
            assert_eq!(anchor_account_disc(name).to_vec(), expected, "{name}");
        }
    }

    #[test]
    fn each_account_decodes_to_its_fields() {
        let mut config = anchor_account_disc("Config").to_vec();
        for field in [[1; 32], [2; 32], [3; 32], [4; 32]] {
            config.extend(field);
        }
        config.extend(3u64.to_le_bytes());
        config.extend([5; 32]);
        assert_eq!(
            decode_config(&config),
            Ok(ConfigAccount {
                protocol_adapter: [1; 32],
                logic_ref: [2; 32],
                emergency_committee: [3; 32],
                emergency_caller: [4; 32],
                version: 3,
                owner: [5; 32],
            })
        );
        assert_eq!(
            decode_config(&config[..config.len() - 1]),
            Err(AccountDecodeError::Truncated { field: "owner" })
        );

        let mut bitmap = anchor_account_disc("NonceBitmap").to_vec();
        let mut bits = [0u8; 32];
        bits[1] = 0b0000_0100; // nonce 10 of the word
        bitmap.extend(bits);
        bitmap.push(254);
        let decoded = decode_nonce_bitmap(&bitmap).unwrap();
        assert_eq!(decoded.bump, 254);
        assert!(decoded.is_used(10) && decoded.is_used(256 + 10));
        assert!(!decoded.is_used(9) && !decoded.is_used(11));
        assert_eq!(
            decode_nonce_bitmap(&config),
            Err(AccountDecodeError::WrongDiscriminator {
                expected: "NonceBitmap"
            })
        );
    }
}
