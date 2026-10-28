// The staging forwarder on devnet, read over a Helius endpoint: run with
// `npm run test:deployed` at each deployment and upgrade.

import { Connection, PublicKey } from "@solana/web3.js";
import { describe, expect, it } from "vitest";

import { emergencyMigratingIdl, idl } from "./idl.js";
import { emergencyMigratingSplTokenForwarder, splTokenForwarder } from "./instance.js";

function devnetUrl(): string {
  const url = process.env.HELIUS_DEVNET_URL;
  if (url === undefined || url === "") {
    throw new Error(
      "HELIUS_DEVNET_URL is unset: set HELIUS_DEVNET_URL to the Helius devnet endpoint " +
        "(scripts/helius.env in the umbrella repository) and run npm run test:deployed",
    );
  }
  return url;
}

describe("the staging forwarder on devnet", () => {
  it("answers every read", async () => {
    const connection = new Connection(devnetUrl(), "confirmed");
    const forwarder = await splTokenForwarder(connection, "staging");
    const owner = await forwarder.owner();
    const logicRef = await forwarder.getLogicRef();
    const adapter = await forwarder.getProtocolAdapter();
    // The owner, an operator wallet, pays the simulation's fee.
    const version = await forwarder.version(owner);
    console.log({
      programId: forwarder.programId.toBase58(),
      owner: owner.toBase58(),
      logicRef,
      adapter: adapter.toBase58(),
      version,
    });

    expect(owner.equals(PublicKey.default)).toBe(false);
    expect(logicRef.length).toBe(32);
    expect(adapter.equals(PublicKey.default)).toBe(false);
    // The release devnet runs is one of the two builds this package carries.
    expect([idl.metadata.version, emergencyMigratingIdl.metadata.version]).toContain(version);

    // Its emergency-migrating reads answer false wherever nothing is listed or
    // recorded, whichever build runs.
    const emergency = await emergencyMigratingSplTokenForwarder(connection, "staging");
    expect(emergency.programId.equals(forwarder.programId)).toBe(true);
    expect(await emergency.isNullifierMigrated(new Uint8Array(32))).toBe(false);
    expect(await emergency.isLogicRefVulnerable(new Uint8Array(32))).toBe(false);
  });
});
