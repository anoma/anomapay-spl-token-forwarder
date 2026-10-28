import { describe, expect, it } from "vitest";
import { hexToBytes } from "@noble/hashes/utils";

import { AccountDecodeError, decodeVulnerableLogicRefs } from "./accounts.js";

/**
 * The account bytes the program's `VulnerableLogicRefs { logic_refs: [[7; 32],
 * [8; 32]] }` serializes to: its discriminator, the list's length and the
 * refs, as the Rust crate's `accounts` test pins them.
 */
const LISTED = hexToBytes(
  "8a4e8457e793330702000000" + "07".repeat(32) + "08".repeat(32),
);

describe("decodeVulnerableLogicRefs", () => {
  it("decodes the program's account to its list", () => {
    const decoded = decodeVulnerableLogicRefs(LISTED);
    console.log(decoded);
    expect(decoded).toEqual([new Uint8Array(32).fill(7), new Uint8Array(32).fill(8)]);
  });

  it("refuses an account that ends inside its list", () => {
    expect(() => decodeVulnerableLogicRefs(LISTED.slice(0, -1))).toThrow(AccountDecodeError);
    expect(() => decodeVulnerableLogicRefs(LISTED.slice(0, -1))).toThrow("the account ends inside its list of logic refs");
    expect(() => decodeVulnerableLogicRefs(LISTED.slice(0, 10))).toThrow("the account ends inside its list of logic refs");
  });

  it("refuses another account's discriminator", () => {
    const other = LISTED.slice();
    other[0] = other[0]! ^ 1;
    expect(() => decodeVulnerableLogicRefs(other)).toThrow("the account is not a VulnerableLogicRefs");
    expect(() => decodeVulnerableLogicRefs(LISTED.slice(0, 7))).toThrow("the account is not a VulnerableLogicRefs");
  });
});
