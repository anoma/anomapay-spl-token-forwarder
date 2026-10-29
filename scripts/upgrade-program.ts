/**
 * Upgrade the forwarder, which owns its upgrades: the owner calls its
 * `upgrade` with a loader buffer it wrote, as the EVM forwarder's owner calls
 * `upgradeToAndCall`. Run through ops.sh (`upgrade`), which writes the buffer
 * and sets the cluster and wallet; the wallet must be the owner.
 *
 *   npx ts-node -P tsconfig.json scripts/upgrade-program.ts path
 *   npx ts-node -P tsconfig.json scripts/upgrade-program.ts upgrade <buffer>
 *
 * `path` prints which upgrade path the program's upgrade authority leaves:
 * `program` when the authority is the program's own PDA (upgrade through
 * the program), `loader` when it is the wallet (the loader's own upgrade,
 * before `initialize` hands the authority over). Any
 * other authority, or none, is an error. `upgrade` verifies that the
 * program then runs the buffer's code and announced its hash.
 */
import * as anchor from "@anchor-lang/core";
import { confirmedProvider } from "../client/provider";
import {
  address,
  assertIsTransactionWithBlockhashLifetime,
  fetchEncodedAccount,
  getSignatureFromTransaction,
  sendAndConfirmTransactionFactory,
  signTransactionWithSigners,
} from "@solana/kit";
import { SplTokenForwarder } from "../ts/src/instance";
import { deriveUpgradeAuthorityPda } from "../ts/src/pda";
import { compileLegacyTransaction } from "../ts/src/transaction";
import { bufferExecutableHash, deployedExecutableHash, upgradeAuthority } from "../client/upgrade";
import { forwarderEventsOfSignature } from "../client/events";
import { fail, parsePubkey, requireEnv } from "./cli-utils";

async function main() {
  const [command, bufferArg] = process.argv.slice(2);
  const provider = confirmedProvider(requireEnv("ANCHOR_PROVIDER_URL", "the RPC endpoint"), process.env.ANCHOR_WS_URL);
  anchor.setProvider(provider);
  // The address of this cluster's build (target/idl).
  const forwarder = SplTokenForwarder.at(provider.rpc, address(anchor.workspace.SplTokenForwarder.address));
  const wallet = provider.wallet.address;

  if (command === "path") {
    const authority = await upgradeAuthority(provider.rpc, forwarder.programId);
    if (authority === (await deriveUpgradeAuthorityPda(forwarder.programId))) console.log("program");
    else if (authority === wallet) console.log("loader");
    else fail(`the forwarder's upgrade authority is ${authority ?? "none (final)"}, neither its PDA nor ${wallet}`);
    return;
  }
  if (command !== "upgrade") fail(`usage: upgrade-program.ts <path|upgrade> [buffer], got ${command}`);

  const buffer = parsePubkey("buffer", bufferArg);
  const bufferAccount = await fetchEncodedAccount(provider.rpc, buffer, { commitment: "confirmed" });
  if (!bufferAccount.exists) fail(`buffer ${buffer} does not exist`);
  const expected = bufferExecutableHash(bufferAccount.data);

  console.log(`Upgrading the forwarder (${forwarder.programId}) from buffer ${buffer}`);
  // Anchor's sender compiles with kit, which refuses the upgrade's writable
  // invoked program (ts/src/transaction.ts).
  const upgrade = await forwarder.upgradeIxs(wallet, buffer, wallet);
  const { value: lifetime } = await provider.rpc.getLatestBlockhash({ commitment: "confirmed" }).send();
  const signed = await signTransactionWithSigners(
    [provider.wallet],
    compileLegacyTransaction(wallet, upgrade, lifetime),
  );
  assertIsTransactionWithBlockhashLifetime(signed);
  await sendAndConfirmTransactionFactory({ rpc: provider.rpc, rpcSubscriptions: provider.rpcSubscriptions })(signed, {
    commitment: "confirmed",
  });
  const signature = getSignatureFromTransaction(signed);

  const events = await forwarderEventsOfSignature(provider.rpc, forwarder.programId, signature);
  const upgraded = events.find((e) => e.name === "Upgraded");
  if (!upgraded) throw new Error(`transaction ${signature} landed without Upgraded`);
  const announced = Buffer.from(upgraded.data.implementation_hash);
  const deployed = await deployedExecutableHash(provider.rpc, forwarder.programId);
  if (!announced.equals(expected) || !deployed.equals(expected)) {
    throw new Error(
      `transaction ${signature}: the buffer's code hashes to ${expected.toString("hex")}, ` +
        `the event announced ${announced.toString("hex")}, the program runs ${deployed.toString("hex")}`,
    );
  }
  console.log(`✅ The forwarder is upgraded to ${expected.toString("hex")}. Signature: ${signature}`);
}

main().catch((err) => {
  console.error(`❌ upgrade-program failed:`, err);
  process.exit(1);
});
