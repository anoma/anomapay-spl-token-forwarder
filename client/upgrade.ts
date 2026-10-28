/**
 * What the loader records about a deployed program: its upgrade authority,
 * and the identity of its code, the executable hash (sha256 of the code with
 * trailing zero bytes removed, as `solana-verify get-executable-hash` and
 * `get-program-hash` compute it and as the programs' `Upgraded` event
 * announces it).
 */
import { createHash } from "crypto";
import { getAddressDecoder, getBase64Encoder, type Address, type Rpc, type SolanaRpcApiMainnet } from "@solana/kit";
import { deriveProgramDataAddress } from "../ts/src/pda";

/** Offset of the authority's option tag in a ProgramData account: after the u32 account kind and the u64 deployment slot. */
const PROGRAM_DATA_AUTHORITY_OFFSET = 12;
/** Bytes before the code in a ProgramData account: the loader's state enum, slot and optional authority. */
const PROGRAM_DATA_METADATA_LEN = PROGRAM_DATA_AUTHORITY_OFFSET + 1 + 32;
/** Bytes before the code in a loader buffer: the loader's state enum and optional authority. */
const BUFFER_METADATA_LEN = 37;

/** sha256 of `code` without its trailing zero bytes. */
export function executableHash(code: Uint8Array): Buffer {
  let end = code.length;
  while (end > 0 && code[end - 1] === 0) end--;
  return createHash("sha256").update(code.subarray(0, end)).digest();
}

/** The executable hash of the code a loader buffer's data holds. */
export function bufferExecutableHash(bufferData: Uint8Array): Buffer {
  return executableHash(bufferData.subarray(BUFFER_METADATA_LEN));
}

/** The data of the account at `address` at `confirmed`, or null when it does not exist. */
export async function accountData(rpc: Rpc<SolanaRpcApiMainnet>, address: Address): Promise<Uint8Array | null> {
  const { value } = await rpc.getAccountInfo(address, { encoding: "base64", commitment: "confirmed" }).send();
  return value === null ? null : Uint8Array.from(getBase64Encoder().encode(value.data[0]));
}

/** The executable hash of the code `programId` runs, read from its ProgramData. */
export async function deployedExecutableHash(rpc: Rpc<SolanaRpcApiMainnet>, programId: Address): Promise<Buffer> {
  const programData = await accountData(rpc, await deriveProgramDataAddress(programId));
  if (programData === null) throw new Error(`${programId} has no ProgramData account`);
  return executableHash(programData.subarray(PROGRAM_DATA_METADATA_LEN));
}

/**
 * A program's upgrade authority, or null when the program is final. The
 * ProgramData layout: u32 account kind (3), u64 deployment slot, then the
 * authority as a Borsh Option<Pubkey> (tag byte, 32 bytes).
 */
export async function upgradeAuthority(rpc: Rpc<SolanaRpcApiMainnet>, programId: Address): Promise<Address | null> {
  const programData = await deriveProgramDataAddress(programId);
  const data = await accountData(rpc, programData);
  if (data === null) {
    throw new Error(`${programId} has no ProgramData (${programData}): not an upgradeable program`);
  }
  const kind = new DataView(data.buffer, data.byteOffset).getUint32(0, true);
  if (kind !== 3) {
    throw new Error(`${programData} is loader account kind ${kind}, not ProgramData (3)`);
  }
  const tag = data[PROGRAM_DATA_AUTHORITY_OFFSET];
  if (tag === 0) return null;
  if (tag !== 1) throw new Error(`${programData}: invalid authority option tag ${tag}`);
  return getAddressDecoder().decode(data.subarray(PROGRAM_DATA_AUTHORITY_OFFSET + 1, PROGRAM_DATA_METADATA_LEN));
}
