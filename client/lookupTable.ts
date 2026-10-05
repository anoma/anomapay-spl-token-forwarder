import {
  AddressLookupTableAccount,
  AddressLookupTableProgram,
  Connection,
  Keypair,
  PublicKey,
  Transaction,
  sendAndConfirmTransaction,
} from "@solana/web3.js";

/** The lookup table at `address`; a missing table is an error. */
export async function fetchLookupTable(connection: Connection, address: PublicKey): Promise<AddressLookupTableAccount> {
  const { value } = await connection.getAddressLookupTable(address);
  if (!value) throw new Error(`lookup table ${address.toBase58()} does not exist`);
  return value;
}

/**
 * Extend the table at `address` with the `keys` it lacks; `payer` pays and
 * must be the table's authority. Returns the usable table, so this waits for
 * the transaction to be finalized: until the slot that extended the table is
 * finalized, a v0 transaction naming the new keys passes simulation but does
 * not land. `signature` is set only when a transaction was sent.
 */
export async function extendLookupTable(
  connection: Connection,
  payer: Keypair,
  keys: PublicKey[],
  address: PublicKey,
): Promise<{ table: AddressLookupTableAccount; added: PublicKey[]; signature?: string }> {
  const table = await fetchLookupTable(connection, address);
  const added: PublicKey[] = [];
  for (const key of keys) {
    if (!table.state.addresses.some((p) => p.equals(key)) && !added.some((a) => a.equals(key))) added.push(key);
  }
  if (added.length === 0) return { table, added };
  const extend = AddressLookupTableProgram.extendLookupTable({
    lookupTable: address,
    authority: payer.publicKey,
    payer: payer.publicKey,
    addresses: added,
  });
  const signature = await sendAndConfirmTransaction(connection, new Transaction().add(extend), [payer], {
    preflightCommitment: "confirmed",
    commitment: "finalized",
  });
  return { table: await fetchLookupTable(connection, address), added, signature };
}
