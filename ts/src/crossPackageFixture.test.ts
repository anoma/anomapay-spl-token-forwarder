// Cross-package fixture: the TS package serializes, hashes and encodes the
// wrap message of `fixtures/wrap_message_fixture.json` to the values the
// fixture records. The Rust crate checks the same fixture
// (`crates/client/tests/cross_package_fixture.rs`), so both produce
// byte-identical output for the same input; any divergence is a wire
// incompatibility with the forwarder.

import { describe, expect, it } from "vitest";
import { bytesToHex, hexToBytes } from "@noble/hashes/utils";

import fixture from "../../fixtures/wrap_message_fixture.json";
import { base64WrapDigest, buildWrapMessage, hashWrapMessage, type WrapMessageInput } from "./wrapMessage.js";

const input: WrapMessageInput = {
  forwarderId: hexToBytes(fixture.input.forwarder_id_hex),
  tokenMint: hexToBytes(fixture.input.token_mint_hex),
  amount: BigInt(fixture.input.amount),
  nonce: BigInt(fixture.input.nonce),
  deadline: BigInt(fixture.input.deadline),
  actionTreeRoot: hexToBytes(fixture.input.action_tree_root_hex),
};

describe("cross-package fixture", () => {
  it("serialization is the fixture's", () => {
    expect(bytesToHex(buildWrapMessage(input))).toBe(fixture.expected.serialized_hex);
  });

  it("sha256 digest is the fixture's", () => {
    expect(bytesToHex(hashWrapMessage(input))).toBe(fixture.expected.sha256_digest_hex);
  });

  it("base64 digest is the fixture's", () => {
    expect(base64WrapDigest(input)).toBe(fixture.expected.sha256_digest_base64);
  });
});
