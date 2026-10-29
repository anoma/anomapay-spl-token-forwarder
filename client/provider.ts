/**
 * The provider every operator script uses: the endpoint `url` and the wallet
 * of the Anchor environment (ANCHOR_WALLET), with every transaction
 * confirmed, and every read made, at `confirmed`, Anchor's default. A
 * cluster's RPC endpoint answers from several nodes; at `processed` (one
 * node's unvoted view), a blockhash or a write one node has seen may be
 * unknown to the node that answers next, and a transaction then fails its
 * simulation with "Blockhash not found".
 *
 * Confirmations arrive over the endpoint's websocket, which Anchor expects at
 * the RPC port plus one; `websocketUrl` names it when it is elsewhere, as on
 * a local runtime that picks its ports.
 */
import { AnchorProvider, createLocalWallet } from "@anchor-lang/core";

export function confirmedProvider(url: string, websocketUrl: string | undefined): AnchorProvider {
  return new AnchorProvider({ url, websocketUrl }, createLocalWallet());
}
