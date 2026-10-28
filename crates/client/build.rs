//! Writes the size of the forwarder's `NonceBitmap` account, as its IDL lays
//! it out: the account's discriminator, then the Borsh encoding of its type,
//! which has a fixed size. `constants::NONCE_BITMAP_ACCOUNT_SIZE` includes it.

use std::path::Path;

use serde_json::Value;

const IDL: &str = "idls/spl_token_forwarder.json";

fn main() {
    println!("cargo:rerun-if-changed={IDL}");
    let idl: Value = serde_json::from_str(
        &std::fs::read_to_string(IDL).unwrap_or_else(|e| panic!("failed to read {IDL}: {e}")),
    )
    .unwrap_or_else(|e| panic!("{IDL} is not JSON: {e}"));
    let size = account_size(&idl, "NonceBitmap");
    let out = Path::new(&std::env::var("OUT_DIR").expect("cargo sets OUT_DIR"))
        .join("nonce_bitmap_account_size.rs");
    std::fs::write(&out, size.to_string())
        .unwrap_or_else(|e| panic!("failed to write {}: {e}", out.display()));
}

/// The bytes of the account `name` of `idl`: its discriminator and its type.
fn account_size(idl: &Value, name: &str) -> usize {
    let account = idl["accounts"]
        .as_array()
        .and_then(|accounts| accounts.iter().find(|account| account["name"] == name))
        .unwrap_or_else(|| panic!("{IDL} declares no account {name}"));
    let discriminator = account["discriminator"]
        .as_array()
        .unwrap_or_else(|| panic!("{IDL}: the account {name} has no discriminator"))
        .len();
    discriminator + borsh_size(idl, &serde_json::json!({ "defined": { "name": name } }))
}

/// The bytes of the Borsh encoding of the IDL type `ty`, which must have a
/// fixed size.
fn borsh_size(idl: &Value, ty: &Value) -> usize {
    if let Some(primitive) = ty.as_str() {
        return match primitive {
            "bool" | "u8" | "i8" => 1,
            "u16" | "i16" => 2,
            "u32" | "i32" | "f32" => 4,
            "u64" | "i64" | "f64" => 8,
            "u128" | "i128" => 16,
            "u256" | "i256" | "pubkey" => 32,
            other => panic!("{IDL}: the type {other} has no fixed size"),
        };
    }
    if let Some([element, length]) = ty["array"].as_array().map(Vec::as_slice) {
        let length = length
            .as_u64()
            .unwrap_or_else(|| panic!("{IDL}: the array {ty} has no literal length"));
        return usize::try_from(length).unwrap() * borsh_size(idl, element);
    }
    if let Some(name) = ty["defined"]["name"].as_str() {
        let declared = idl["types"]
            .as_array()
            .and_then(|types| types.iter().find(|t| t["name"] == name))
            .unwrap_or_else(|| panic!("{IDL} declares no type {name}"));
        let fields = declared["type"]["fields"]
            .as_array()
            .filter(|_| declared["type"]["kind"] == "struct")
            .unwrap_or_else(|| panic!("{IDL}: {name} is no struct with fields: {declared}"));
        return fields
            .iter()
            .map(|field| borsh_size(idl, &field["type"]))
            .sum();
    }
    panic!("{IDL}: the type {ty} has no fixed size")
}
