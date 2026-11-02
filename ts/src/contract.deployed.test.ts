// The staging forwarder on devnet, read over a Helius endpoint: run with
// `npm run test:deployed` at each deployment and upgrade. Each read is checked
// against the account it reads, fetched and decoded here.

import {
  createSolanaRpc,
  fetchEncodedAccount,
  type Address,
  type Rpc,
  type SolanaRpcApiMainnet,
} from "@solana/kit";
import { describe, expect, it } from "vitest";

import { decodeVulnerableLogicRefs } from "./accounts.js";
import { coder, emergencyMigratingCoder } from "./coder.js";
import { EmergencyMigratingSplTokenForwarderInstance, splTokenForwarder } from "./contract.js";
import { emergencyMigratingIdl, idl, idlConstant } from "./idl.js";
import { deriveForwarderConfigPda, deriveMigrationRecordPda, deriveVulnerableLogicRefsPda } from "./pda.js";

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

/** The data of the account at `address` when it exists and `owner` owns it, else `null`. */
async function ownedData(rpc: Rpc<SolanaRpcApiMainnet>, address: Address, owner: Address): Promise<Uint8Array | null> {
  const account = await fetchEncodedAccount(rpc, address);
  return account.exists && account.programAddress === owner ? account.data : null;
}

describe("the staging forwarder on devnet", () => {
  it("answers every read with what its accounts hold", async () => {
    const rpc = createSolanaRpc(devnetUrl());
    const forwarder = await splTokenForwarder(rpc, "staging");
    expect(forwarder.address).toBe(idl.address);

    const [configAddress] = await deriveForwarderConfigPda(forwarder.address);
    const configData = await ownedData(rpc, configAddress, forwarder.address);
    if (configData === null) throw new Error(`the forwarder's config ${configAddress} does not exist`);
    const config = coder.accounts.decode("Config", configData);
    console.log({ address: forwarder.address, config });

    expect(await forwarder.getLogicRef()).toEqual(config.logic_ref);
    expect(await forwarder.getProtocolAdapter()).toBe(config.protocol_adapter);
    expect(await forwarder.owner()).toBe(config.owner);
    // The owner, an operator wallet, pays the simulation's fee. The release
    // devnet runs is one of the two builds this package carries.
    const version = await forwarder.version(config.owner);
    console.log({ version });
    expect([idl.metadata.version, emergencyMigratingIdl.metadata.version]).toContain(version);

    // The emergency-migrating build's reads at the same address, whichever
    // build runs there: each answers what its account holds, false where the
    // forwarder holds none.
    const emergency = new EmergencyMigratingSplTokenForwarderInstance(forwarder.address, rpc);
    expect(emergency.legacyForwarder()).toBe(idlConstant(emergencyMigratingIdl, "LEGACY_FORWARDER"));
    const [listAddress] = await deriveVulnerableLogicRefsPda(forwarder.address);
    const listData = await ownedData(rpc, listAddress, forwarder.address);
    const listed = listData === null ? [] : decodeVulnerableLogicRefs(listData);
    console.log({ listed });
    for (const logicRef of listed) {
      expect(await emergency.isLogicRefVulnerable(logicRef), `${logicRef}`).toBe(true);
    }
    const zero = new Uint8Array(32);
    const zeroListed = listed.some((r) => r.every((b) => b === 0));
    expect(await emergency.isLogicRefVulnerable(zero)).toBe(zeroListed);
    const [recordAddress] = await deriveMigrationRecordPda(forwarder.address, zero);
    const recordData = await ownedData(rpc, recordAddress, forwarder.address);
    const migrated =
      recordData !== null && (emergencyMigratingCoder.accounts.decode("MigrationRecord", recordData).migrated as boolean);
    expect(await emergency.isNullifierMigrated(zero)).toBe(migrated);
  });
});
