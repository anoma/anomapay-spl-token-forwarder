import { PublicKey } from "@solana/web3.js";
import { PA_STATE_SEED, UPGRADE_AUTHORITY_SEED } from "./constants";

export const BPF_LOADER_UPGRADEABLE = new PublicKey("BPFLoaderUpgradeab1e11111111111111111111111");

// Protocol adapter PDAs

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
