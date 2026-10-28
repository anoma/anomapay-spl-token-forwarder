//! The decoder of the one forwarder account the IDLs do not declare: the
//! emergency-migrating build's `VulnerableLogicRefs`, which `reinitialize`
//! grows in place and reads as a raw account, so no instruction takes it as
//! a typed account and Anchor leaves it out of the IDL. Every other account
//! is a generated type (`generated::*::accounts`).

use anchor_lang::AnchorDeserialize;
use sha2::{Digest, Sha256};

/// Errors produced by [`decode_vulnerable_logic_refs`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AccountDecodeError {
    /// The data does not start with the `VulnerableLogicRefs` discriminator.
    WrongDiscriminator,
    /// The data ends inside the list of logic refs.
    Truncated,
}

impl core::fmt::Display for AccountDecodeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            AccountDecodeError::WrongDiscriminator => {
                write!(f, "the account is not a VulnerableLogicRefs")
            }
            AccountDecodeError::Truncated => {
                write!(f, "the account ends inside its list of logic refs")
            }
        }
    }
}

impl std::error::Error for AccountDecodeError {}

/// Anchor's discriminator of the `#[account]` `VulnerableLogicRefs`:
/// `sha256("account:VulnerableLogicRefs")[..8]`.
fn vulnerable_logic_refs_discriminator() -> [u8; 8] {
    Sha256::digest(b"account:VulnerableLogicRefs")[..8]
        .try_into()
        .expect("a SHA-256 digest is longer than 8 bytes")
}

/// Decode the emergency-migrating build's `VulnerableLogicRefs` account:
/// the logic refs whose resources can emergency-migrate, as the EVM draft's
/// `isLogicRefVulnerable` answers for each. As the program's own read
/// (`try_deserialize`), bytes after the list are not read.
pub fn decode_vulnerable_logic_refs(data: &[u8]) -> Result<Vec<[u8; 32]>, AccountDecodeError> {
    let mut body = data
        .strip_prefix(&vulnerable_logic_refs_discriminator())
        .ok_or(AccountDecodeError::WrongDiscriminator)?;
    // A length then fixed-size arrays: running out of bytes is the only way
    // the Borsh read can fail.
    Vec::<[u8; 32]>::deserialize(&mut body).map_err(|_| AccountDecodeError::Truncated)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The account bytes the program's `VulnerableLogicRefs { logic_refs:
    /// [[7; 32], [8; 32]] }` serializes to: its discriminator, the list's
    /// length and the refs.
    const EXPECTED_VULNERABLE_LOGIC_REFS: &str = "8a4e8457e793330702000000\
        0707070707070707070707070707070707070707070707070707070707070707\
        0808080808080808080808080808080808080808080808080808080808080808";

    fn bytes(hex: &str) -> Vec<u8> {
        (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn the_vulnerable_logic_refs_decode_to_their_list() {
        let listed = bytes(EXPECTED_VULNERABLE_LOGIC_REFS);
        let decoded = decode_vulnerable_logic_refs(&listed);
        println!("{listed:02x?} decodes to {decoded:?}");
        assert_eq!(decoded, Ok(vec![[7; 32], [8; 32]]));
        assert_eq!(
            decode_vulnerable_logic_refs(&listed[..listed.len() - 1]),
            Err(AccountDecodeError::Truncated),
            "one byte short of the second ref"
        );
        let mut other = listed.clone();
        other[0] ^= 1;
        assert_eq!(
            decode_vulnerable_logic_refs(&other),
            Err(AccountDecodeError::WrongDiscriminator)
        );
    }
}
