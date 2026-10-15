import { PublicKey } from "@solana/web3.js";

export const BPF_LOADER_UPGRADEABLE = new PublicKey("BPFLoaderUpgradeab1e11111111111111111111111");

/**
 * `UPGRADE_AUTHORITY_SEED`, which the forwarder and the adapter share: the seed
 * of a program's upgrade authority PDA, through which only the program
 * upgrades itself.
 */
const UPGRADE_AUTHORITY_SEED = Buffer.from("upgrade_authority");

/** The upgradeable loader's program-data account of a program. */
export function deriveProgramDataPda(programId: PublicKey): PublicKey {
  return PublicKey.findProgramAddressSync([programId.toBuffer()], BPF_LOADER_UPGRADEABLE)[0];
}

/** A program's upgrade authority once initialized: its PDA at `UPGRADE_AUTHORITY_SEED`, which only it signs for. */
export function deriveUpgradeAuthorityPda(programId: PublicKey): PublicKey {
  return PublicKey.findProgramAddressSync([UPGRADE_AUTHORITY_SEED], programId)[0];
}
