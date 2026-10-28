import { describe, expect, it } from "vitest";

import forwarderRs from "../../crates/client/src/forwarder.rs?raw";
import { UPGRADE_COMPUTE_UNIT_LIMIT } from "./constants.js";

describe("UPGRADE_COMPUTE_UNIT_LIMIT", () => {
  // The Rust crate's `UPGRADE_COMPUTE_UNIT_LIMIT`, as its source declares it.
  it("is the Rust crate's", () => {
    const declared = /pub const UPGRADE_COMPUTE_UNIT_LIMIT: u32 = ([0-9_]+);/.exec(forwarderRs);
    expect(declared, "crates/client/src/forwarder.rs declares no UPGRADE_COMPUTE_UNIT_LIMIT").not.toBeNull();
    const rust = Number(declared![1]!.replaceAll("_", ""));
    console.log(`Rust ${rust}, TS ${UPGRADE_COMPUTE_UNIT_LIMIT}`);
    expect(UPGRADE_COMPUTE_UNIT_LIMIT).toBe(rust);
  });
});
