/**
 * The provider every operator script and spec file uses: the endpoint and
 * wallet of the Anchor environment (ANCHOR_PROVIDER_URL, ANCHOR_WALLET), with
 * every transaction confirmed, and every read made, at `confirmed`. A cluster's
 * RPC endpoint answers from several nodes; at Anchor's default, `processed`
 * (one node's unvoted view), a blockhash or a write one node has seen may be
 * unknown to the node that answers next, and a transaction then fails its
 * simulation with "Blockhash not found".
 *
 * Confirmations arrive over the endpoint's websocket, which web3.js expects at
 * the RPC port plus one; ANCHOR_WS_URL names it when it is elsewhere, as on a
 * local runtime that picks its ports.
 */
import { AnchorProvider } from "@anchor-lang/core";
import { Connection } from "@solana/web3.js";

export function confirmedProvider(): AnchorProvider {
  const env = AnchorProvider.env();
  const connection = new Connection(env.connection.rpcEndpoint, {
    commitment: "confirmed",
    wsEndpoint: process.env.ANCHOR_WS_URL,
  });
  return new AnchorProvider(connection, env.wallet, {
    commitment: "confirmed",
    preflightCommitment: "confirmed",
  });
}
