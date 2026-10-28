/**
 * The provider every operator script uses: the endpoint and wallet of the
 * Anchor environment (ANCHOR_PROVIDER_URL, ANCHOR_WALLET), with every
 * transaction confirmed, and every read made, at `confirmed`, Anchor's
 * default. A cluster's RPC endpoint answers from several nodes; at
 * `processed` (one node's unvoted view), a blockhash or a write one node has
 * seen may be unknown to the node that answers next, and a transaction then
 * fails its simulation with "Blockhash not found".
 *
 * Confirmations arrive over the endpoint's websocket, which Anchor expects at
 * the RPC port plus one; ANCHOR_WS_URL names it when it is elsewhere, as on a
 * local runtime that picks its ports.
 */
import { AnchorProvider, createLocalWallet } from "@anchor-lang/core";
import { requireEnv } from "../scripts/cli-utils";

export function confirmedProvider(): AnchorProvider {
  return new AnchorProvider(
    {
      url: requireEnv("ANCHOR_PROVIDER_URL", "the RPC endpoint"),
      websocketUrl: process.env.ANCHOR_WS_URL,
    },
    createLocalWallet(),
    { commitment: "confirmed", preflightCommitment: "confirmed" },
  );
}
