import {
  AddressLookupTableAccount,
  AddressLookupTableProgram,
  Connection,
  Keypair,
  PublicKey,
  SYSVAR_INSTRUCTIONS_PUBKEY,
  Transaction,
  sendAndConfirmTransaction,
} from "@solana/web3.js";
import { TOKEN_PROGRAM_ID } from "@solana/spl-token";
import { escrowAccounts } from "./instructions";
import { deriveConfigPda, deriveEscrowAuthority, deriveEventAuthorityPda } from "./pda";

/**
 * The accounts every settlement that calls the forwarder carries for it and
 * that are the same for every call: the ones the deployment's settlement
 * lookup table holds. The user's and the recipient's token accounts and the
 * user's nonce bitmap differ per settlement; each mint in `mints` adds its
 * escrow account.
 */
export function forwarderLookupKeys(splTokenForwarder: PublicKey, mints: PublicKey[]): PublicKey[] {
  const [config] = deriveConfigPda(splTokenForwarder);
  const [eventAuthority] = deriveEventAuthorityPda(splTokenForwarder);
  return [
    splTokenForwarder,
    config,
    SYSVAR_INSTRUCTIONS_PUBKEY,
    eventAuthority,
    deriveEscrowAuthority(splTokenForwarder),
    TOKEN_PROGRAM_ID,
    ...mints.map((mint) => escrowAccounts(splTokenForwarder, mint).escrowAta),
  ];
}

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
  const present = (await fetchLookupTable(connection, address)).state.addresses;
  const added: PublicKey[] = [];
  for (const key of keys) {
    if (!present.some((p) => p.equals(key)) && !added.some((a) => a.equals(key))) added.push(key);
  }
  let signature: string | undefined;
  if (added.length > 0) {
    const extend = AddressLookupTableProgram.extendLookupTable({
      lookupTable: address,
      authority: payer.publicKey,
      payer: payer.publicKey,
      addresses: added,
    });
    signature = await sendAndConfirmTransaction(connection, new Transaction().add(extend), [payer], {
      preflightCommitment: "confirmed",
      commitment: "finalized",
    });
  }
  return { table: await fetchLookupTable(connection, address), added, signature };
}
