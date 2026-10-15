//! Constants of the forwarder's interface with the adapter.

/// Number of accounts in a wrap forwarder CPI segment
/// (`build_wrap_forwarder_accounts`).
pub const FORWARDER_WRAP_NUM_ACCOUNTS: u8 = 10;

/// Number of accounts in an unwrap forwarder CPI segment
/// (`build_unwrap_forwarder_accounts`).
pub const FORWARDER_UNWRAP_NUM_ACCOUNTS: u8 = 9;

/// Nonces per nonce-bitmap account. The bitmap for `nonce` is the one for
/// word `nonce / NONCES_PER_WORD`.
pub const NONCES_PER_WORD: u64 = 256;

/// Size of a nonce-bitmap account: the Anchor discriminator, the 256-bit word
/// and the bump. A settlement's payer funds its rent when a wrap's word has no
/// bitmap yet.
pub const NONCE_BITMAP_ACCOUNT_SIZE: usize = 8 + 32 + 1;

/// Size of the config account: the Anchor discriminator, the adapter, the
/// logic ref, the version and the owner.
pub const CONFIG_ACCOUNT_SIZE: usize = 8 + 32 + 32 + 8 + 32;

/// The config version this build of the forwarder initializes to and
/// reinitializes to (the `n` of an OpenZeppelin `reinitializer(n)`).
pub const CONFIG_VERSION: u64 = 5;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_constants_the_idl_declares_are_the_idls() {
        let idl = crate::idl::idl();
        let declared = |name: &str| {
            idl["constants"]
                .as_array()
                .unwrap()
                .iter()
                .find(|constant| constant["name"] == name)
                .unwrap_or_else(|| panic!("the IDL declares no {name}"))["value"]
                .as_str()
                .unwrap()
                .to_string()
        };
        assert_eq!(declared("CONFIG_VERSION"), CONFIG_VERSION.to_string());
        assert_eq!(declared("NONCES_PER_WORD"), NONCES_PER_WORD.to_string());
    }
}
