//! Cross-package fixture: the Rust crate serializes, hashes and encodes the
//! wrap message of `fixtures/wrap_message_fixture.json` to the values the
//! fixture records. The TypeScript package checks the same fixture
//! (`ts/src/crossPackageFixture.test.ts`), so both produce byte-identical
//! output for the same input; any divergence is a wire incompatibility with
//! the forwarder.

use anomapay_spl_token_forwarder_client::wrap_message::WrapMessage;
use serde_json::Value;

fn fixture() -> Value {
    serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/wrap_message_fixture.json"
    )))
    .expect("fixtures/wrap_message_fixture.json is JSON")
}

fn field<'a>(value: &'a Value, path: &str) -> &'a str {
    value
        .pointer(path)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("the fixture has no string at {path}"))
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn bytes32(value: &Value, path: &str) -> [u8; 32] {
    let hex = field(value, path);
    assert_eq!(hex.len(), 64, "{path} is not 32 bytes of hex: {hex}");
    std::array::from_fn(|i| {
        u8::from_str_radix(&hex[2 * i..2 * i + 2], 16)
            .unwrap_or_else(|e| panic!("{path} is not hex ({e}): {hex}"))
    })
}

fn number<T: std::str::FromStr>(value: &Value, path: &str) -> T
where
    T::Err: std::fmt::Debug,
{
    field(value, path)
        .parse()
        .unwrap_or_else(|e| panic!("{path} is not a number: {e:?}"))
}

fn fixture_message(fixture: &Value) -> WrapMessage {
    WrapMessage {
        forwarder_id: bytes32(fixture, "/input/forwarder_id_hex"),
        token_mint: bytes32(fixture, "/input/token_mint_hex"),
        amount: number(fixture, "/input/amount"),
        nonce: number(fixture, "/input/nonce"),
        deadline: number(fixture, "/input/deadline"),
        action_tree_root: bytes32(fixture, "/input/action_tree_root_hex"),
    }
}

#[test]
fn serialization_is_the_fixtures() {
    let fixture = fixture();
    assert_eq!(
        to_hex(&fixture_message(&fixture).serialize()),
        field(&fixture, "/expected/serialized_hex")
    );
}

#[test]
fn sha256_digest_is_the_fixtures() {
    let fixture = fixture();
    assert_eq!(
        to_hex(&fixture_message(&fixture).sha256_digest()),
        field(&fixture, "/expected/sha256_digest_hex")
    );
}

#[test]
fn base64_digest_is_the_fixtures() {
    let fixture = fixture();
    assert_eq!(
        fixture_message(&fixture).base64_digest(),
        field(&fixture, "/expected/sha256_digest_base64")
    );
}
