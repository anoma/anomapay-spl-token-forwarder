import { describe, expect, it } from "vitest";

import forwarderIdl from "../../crates/client/idl/spl_token_forwarder.json";
import { FORWARDER_PROGRAM_ID } from "./programIds.js";

describe("program ids", () => {
  it("FORWARDER_PROGRAM_ID matches the vendored IDL", () => {
    expect(forwarderIdl.address).toBe(FORWARDER_PROGRAM_ID.toBase58());
  });
});
