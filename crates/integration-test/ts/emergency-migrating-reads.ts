/**
 * The TypeScript bindings' emergency-migrating instance read against a
 * runtime: prints, as one line of JSON, what `isLogicRefVulnerable` answers
 * for each of LOGIC_REFS and what `isNullifierMigrated` answers for each of
 * NULLIFIERS (comma-separated 32-byte hex), of the forwarder at
 * FORWARDER_PROGRAM_ID over ANCHOR_PROVIDER_URL. The integration tests run it
 * (`emergency_migrating.rs`) and compare the answers with what they set up.
 */
import { createSolanaRpc } from "@solana/kit";

import { requireEnv, requirePubkey } from "../../../scripts/cli-utils";
import { EmergencyMigratingSplTokenForwarderInstance } from "../../../ts/src/contract";

/** The 32-byte hex values of the comma-separated list in `name`. */
function hexList(name: string, what: string): string[] {
  const hexes = requireEnv(name, what).split(",");
  for (const hex of hexes) {
    if (!/^[0-9a-f]{64}$/.test(hex)) throw new Error(`${name}: ${hex} is not 32 bytes of lowercase hex`);
  }
  return hexes;
}

async function main() {
  const rpc = createSolanaRpc(requireEnv("ANCHOR_PROVIDER_URL", "the RPC endpoint"));
  const forwarder = new EmergencyMigratingSplTokenForwarderInstance(
    requirePubkey("FORWARDER_PROGRAM_ID", "the forwarder's address"),
    rpc,
  );
  const answers = async (hexes: string[], read: (bytes: Uint8Array) => Promise<boolean>) =>
    Object.fromEntries(await Promise.all(hexes.map(async (hex) => [hex, await read(Buffer.from(hex, "hex"))])));
  const isLogicRefVulnerable = await answers(hexList("LOGIC_REFS", "the logic refs to read"), (r) =>
    forwarder.isLogicRefVulnerable(r),
  );
  const isNullifierMigrated = await answers(hexList("NULLIFIERS", "the nullifiers to read"), (n) =>
    forwarder.isNullifierMigrated(n),
  );
  console.log(JSON.stringify({ isLogicRefVulnerable, isNullifierMigrated }));
}

main().catch((error: unknown) => {
  console.error(error);
  process.exit(1);
});
