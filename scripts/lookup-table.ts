/**
 * Add the forwarder's accounts to the deployment's settlement lookup table,
 * which the adapter's operator created. Run through ops.sh, which sets the
 * cluster and wallet:
 *
 *   ./scripts/dev.sh lookup-table --cluster <c> [--wallet <path>]
 *
 * The key set is `forwarderSettlementLookupKeys` (ts/src/forwarder.ts); see
 * docs/OPERATIONS.md, "The settlement lookup table". The wallet pays and must
 * be the table's authority.
 *
 * Environment:
 *   PA_LOOKUP_TABLE  base58 address of the deployment's table, to extend it
 *                    with the keys it lacks
 *   STF_TOKEN_MINTS  comma-separated base58 mints whose escrow accounts to add
 *                    (optional)
 */
import * as anchor from "@anchor-lang/core";
import { confirmedProvider } from "../client/provider";
import { Program } from "@anchor-lang/core";
import { SplTokenForwarder } from "../target/types/spl_token_forwarder";
import { extendLookupTable } from "../client/lookupTable";
import { forwarderSettlementLookupKeys } from "../ts/src/forwarder";
import { pubkeyList, requirePubkey } from "./cli-utils";

async function main() {
  const provider = confirmedProvider();
  anchor.setProvider(provider);
  const wallet = provider.wallet as anchor.Wallet;
  const forwarder = anchor.workspace.SplTokenForwarder as Program<SplTokenForwarder>;

  const mints = pubkeyList("STF_TOKEN_MINTS");
  const address = requirePubkey("PA_LOOKUP_TABLE", "the deployment's settlement lookup table, as a base58 pubkey");

  const keys = forwarderSettlementLookupKeys(forwarder.programId, mints);
  const { table, added, signature } = await extendLookupTable(provider.connection, wallet.payer, keys, address);

  console.log(`settlement lookup table: ${table.key.toBase58()} (authority ${wallet.publicKey.toBase58()})`);
  for (const key of keys) {
    console.log(`  ${added.some((a) => a.equals(key)) ? "+" : "="} ${key.toBase58()}`);
  }
  console.log(signature ? `${added.length} key(s) added in ${signature}` : "0 keys added, nothing sent");
}

main().catch((err) => {
  console.error("❌ lookup-table failed:", err.message || err);
  process.exit(1);
});
