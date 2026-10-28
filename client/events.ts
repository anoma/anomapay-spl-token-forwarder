/**
 * The forwarder's events are Anchor CPI events: inner instructions of the
 * forwarder whose data `decodeForwarderEventInstruction` decodes.
 */
import { getBase58Encoder, type Address, type Rpc, type Signature, type SolanaRpcApiMainnet } from "@solana/kit";
import { decodeForwarderEventInstruction } from "../ts/src/events";

/**
 * The events `forwarder` emitted in the confirmed transaction `signature`, in
 * emission order: each its IDL name and its fields under their IDL names.
 */
export async function forwarderEventsOfSignature(
  rpc: Rpc<SolanaRpcApiMainnet>,
  forwarder: Address,
  signature: Signature,
): Promise<ReturnType<typeof decodeForwarderEventInstruction>[]> {
  const tx = await rpc
    .getTransaction(signature, { commitment: "confirmed", encoding: "json", maxSupportedTransactionVersion: 0 })
    .send();
  if (!tx) throw new Error(`transaction ${signature} is not fetchable at confirmed commitment`);
  // The message's keys, then the keys its lookup tables loaded: writable, then read-only.
  const keys = [
    ...tx.transaction.message.accountKeys,
    ...(tx.meta?.loadedAddresses?.writable ?? []),
    ...(tx.meta?.loadedAddresses?.readonly ?? []),
  ];
  return (tx.meta?.innerInstructions ?? []).flatMap((group) =>
    group.instructions
      .filter((ix) => keys[ix.programIdIndex] === forwarder)
      .map((ix) => decodeForwarderEventInstruction(Uint8Array.from(getBase58Encoder().encode(ix.data)))),
  );
}
