import { PublicKey } from "@solana/web3.js";

export const BPF_LOADER_UPGRADEABLE = new PublicKey("BPFLoaderUpgradeab1e11111111111111111111111");

/** The adapter's `PA_STATE_SEED`: its state account's seed. */
const PA_STATE_SEED = Buffer.from("pa_state");
/**
 * `UPGRADE_AUTHORITY_SEED`, which the forwarder and the adapter share: the seed
 * of a program's upgrade authority PDA, through which only the program
 * upgrades itself.
 */
const UPGRADE_AUTHORITY_SEED = Buffer.from("upgrade_authority");

/** The adapter's state account, whose pause the forwarder's emergency instructions check. */
export function derivePaStatePda(paProgramId: PublicKey): [PublicKey, number] {
  return PublicKey.findProgramAddressSync([PA_STATE_SEED], paProgramId);
}

/** The upgradeable loader's program-data account of a program. */
export function deriveProgramDataPda(programId: PublicKey): PublicKey {
  return PublicKey.findProgramAddressSync([programId.toBuffer()], BPF_LOADER_UPGRADEABLE)[0];
}

/** A program's upgrade authority once initialized: its PDA at `UPGRADE_AUTHORITY_SEED`, which only it signs for. */
export function deriveUpgradeAuthorityPda(programId: PublicKey): PublicKey {
  return PublicKey.findProgramAddressSync([UPGRADE_AUTHORITY_SEED], programId)[0];
}
