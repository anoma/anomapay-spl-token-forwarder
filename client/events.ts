/**
 * The forwarder's events are Anchor CPI events: inner instructions of the
 * forwarder whose data `decodeForwarderEventInstruction` decodes.
 */
import * as anchor from "@anchor-lang/core";
import { Connection, PublicKey } from "@solana/web3.js";
import { type ForwarderEvent, decodeForwarderEventInstruction } from "../ts/src/events";

/** The events `forwarder` emitted in the confirmed transaction `signature`, in emission order. */
export async function forwarderEventsOfSignature(
  connection: Connection,
  forwarder: PublicKey,
  signature: string,
): Promise<ForwarderEvent[]> {
  const tx = await connection.getTransaction(signature, { commitment: "confirmed", maxSupportedTransactionVersion: 0 });
  if (!tx) throw new Error(`transaction ${signature} is not fetchable at confirmed commitment`);
  const keys = tx.transaction.message.getAccountKeys({ accountKeysFromLookups: tx.meta?.loadedAddresses });
  return (tx.meta?.innerInstructions ?? []).flatMap((group) =>
    group.instructions
      .filter((ix) => keys.get(ix.programIdIndex)?.equals(forwarder))
      .map((ix) => decodeForwarderEventInstruction(anchor.utils.bytes.bs58.decode(ix.data))),
  );
}
