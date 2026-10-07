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

/// The config version this build of the forwarder initializes to and
/// reinitializes to (the `n` of an OpenZeppelin `reinitializer(n)`).
pub const CONFIG_VERSION: u64 = 3;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_constants_the_idl_declares_are_the_idls() {
        let idl: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/idl/spl_token_forwarder.json"
        )))
        .unwrap();
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
