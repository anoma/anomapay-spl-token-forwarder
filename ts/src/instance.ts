// The forwarder deployed at an address, as the EVM bindings return the ERC20
// forwarder: its reads under their EVM names, from its config and the
// emergency-migrating build's accounts, and its instructions, built for its
// address with its build's IDL, as the Rust crate's `instance` reads and
// builds them. The reads and instructions both builds declare are written
// once (`ForwarderInstance`); each build's class adds its own.

import { BN, BorshCoder, type Idl } from "@anchor-lang/core";
import { getAssociatedTokenAddressSync, TOKEN_PROGRAM_ID } from "@solana/spl-token";
import {
  ComputeBudgetProgram,
  Connection,
  PublicKey,
  SYSVAR_CLOCK_PUBKEY,
  SYSVAR_RENT_PUBKEY,
  SystemProgram,
  TransactionInstruction,
  TransactionMessage,
  VersionedTransaction,
  type AccountMeta,
  type TransactionError,
  type TransactionReturnData,
} from "@solana/web3.js";

import { decodeVulnerableLogicRefs } from "./accounts.js";
import { coder, emergencyMigratingCoder } from "./coder.js";
import { UPGRADE_COMPUTE_UNIT_LIMIT } from "./constants.js";
import { chainIdOfGenesisHash, deployment, type Deployment, type DeploymentRecord, type Environment } from "./deployments.js";
import { emergencyMigratingIdl, idl, idlConstant } from "./idl.js";
import {
  BPF_LOADER_UPGRADEABLE,
  deriveEventAuthorityPda,
  deriveForwarderConfigPda,
  deriveForwarderEscrowAuthority,
  deriveMigrationRecordPda,
  deriveNonceBitmapPda,
  derivePaStatePda,
  deriveProgramDataAddress,
  deriveUpgradeAuthorityPda,
  deriveVulnerableLogicRefsPda,
} from "./pda.js";

/** Why an instance could not be made or could not answer. */
export type InstanceErrorDetail =
  /** The environment records no deployed forwarder on the cluster. */
  | { kind: "NoDeployment"; chainId: string }
  /** An account the answer is read from does not exist. */
  | { kind: "AccountMissing"; address: PublicKey }
  /** An RPC request failed: retrying may succeed. */
  | { kind: "Rpc"; what: string; error: string }
  /** What the chain holds does not decode. */
  | { kind: "Decode"; what: string; error: string }
  /** A simulated instruction failed. */
  | { kind: "Simulation"; error: TransactionError; logs: string[] };

function describe(detail: InstanceErrorDetail): string {
  switch (detail.kind) {
    case "NoDeployment":
      return `no deployed forwarder is recorded on ${detail.chainId}`;
    case "AccountMissing":
      return `the account ${detail.address.toBase58()} does not exist`;
    case "Rpc":
      return `failed to fetch ${detail.what}: ${detail.error}`;
    case "Decode":
      return `${detail.what} does not decode: ${detail.error}`;
    case "Simulation":
      return `the simulation failed: ${JSON.stringify(detail.error)}; logs: ${JSON.stringify(detail.logs)}`;
  }
}

export class InstanceError extends Error {
  constructor(
    readonly detail: InstanceErrorDetail,
    options?: ErrorOptions,
  ) {
    super(describe(detail), options);
    this.name = "InstanceError";
  }
}

async function rpc<T>(what: string, request: Promise<T>): Promise<T> {
  try {
    return await request;
  } catch (error) {
    throw new InstanceError({ kind: "Rpc", what, error: String(error) }, { cause: error });
  }
}

/**
 * The forwarder `environment` records on the cluster whose genesis hash is
 * `genesisHash`, in `record` (the package's `deployments.json` by default).
 * Both builds run at the one address: the emergency-migrating build replaces
 * the production build in place.
 */
export function recordedDeployment(
  environment: Environment,
  genesisHash: string,
  record?: DeploymentRecord,
): Deployment {
  const id = chainIdOfGenesisHash(genesisHash);
  const recorded = deployment(environment, id, record);
  if (recorded === undefined) throw new InstanceError({ kind: "NoDeployment", chainId: id });
  return recorded;
}

async function recordedProgram(connection: Connection, environment: Environment): Promise<PublicKey> {
  const genesisHash = await rpc("the genesis hash", connection.getGenesisHash());
  return recordedDeployment(environment, genesisHash).programId;
}

/** The forwarder `environment` records on the cluster behind `connection`, as the EVM bindings return the ERC20 forwarder. */
export async function splTokenForwarder(connection: Connection, environment: Environment): Promise<SplTokenForwarder> {
  return SplTokenForwarder.at(connection, await recordedProgram(connection, environment));
}

/** The forwarder `environment` records on the cluster behind `connection`, read as the emergency-migrating build. */
export async function emergencyMigratingSplTokenForwarder(
  connection: Connection,
  environment: Environment,
): Promise<EmergencyMigratingSplTokenForwarder> {
  return EmergencyMigratingSplTokenForwarder.at(connection, await recordedProgram(connection, environment));
}

/** A u64 argument in the form the coder encodes. */
const u64 = (value: bigint): BN => new BN(value.toString());

/** The forwarder at an address, read over a connection at its commitment, as the build whose IDL is `idl`. */
abstract class ForwarderInstance {
  protected constructor(
    protected readonly connection: Connection,
    /** The address the forwarder is deployed at. */
    readonly programId: PublicKey,
    private readonly idl: Idl,
    private readonly coder: BorshCoder,
  ) {}

  /** The forwarder's config, as the coder decodes `Config`: its IDL field names and the coder's values. */
  private async config(): Promise<any> {
    const [address] = deriveForwarderConfigPda(this.programId);
    const what = `the forwarder's config ${address.toBase58()}`;
    const account = await rpc(what, this.connection.getAccountInfo(address));
    if (account === null) throw new InstanceError({ kind: "AccountMissing", address });
    try {
      return this.coder.accounts.decode("Config", account.data);
    } catch (error) {
      throw new InstanceError({ kind: "Decode", what, error: String(error) }, { cause: error });
    }
  }

  /** The data of the account at `address` when it exists and the forwarder owns it: an account the forwarder created. */
  protected async ownedData(address: PublicKey, what: string): Promise<Buffer | null> {
    const account = await rpc(`${what} ${address.toBase58()}`, this.connection.getAccountInfo(address));
    return account !== null && account.owner.equals(this.programId) ? account.data : null;
  }

  /** `getLogicRef()`. */
  async getLogicRef(): Promise<number[]> {
    return (await this.config()).logic_ref;
  }

  /** `getProtocolAdapter()`. */
  async getProtocolAdapter(): Promise<PublicKey> {
    return (await this.config()).protocol_adapter;
  }

  /** `owner()`: the zero key once the ownership is renounced. */
  async owner(): Promise<PublicKey> {
    return (await this.config()).owner;
  }

  /**
   * `VERSION()`: the forwarder's `version` simulated, with `feePayer`, any
   * funded system account, paying the simulation's fee (nothing is signed or
   * sent).
   */
  async version(feePayer: PublicKey): Promise<string> {
    const message = new TransactionMessage({
      payerKey: feePayer,
      // The RPC replaces it with its latest.
      recentBlockhash: PublicKey.default.toBase58(),
      instructions: [this.versionIx()],
    }).compileToLegacyMessage();
    const { value } = await rpc(
      "the simulation of version",
      this.connection.simulateTransaction(new VersionedTransaction(message), {
        sigVerify: false,
        replaceRecentBlockhash: true,
      }),
    );
    if (value.err !== null) {
      throw new InstanceError({ kind: "Simulation", error: value.err, logs: value.logs ?? [] });
    }
    return returnedString(this.programId, value.returnData ?? null);
  }

  /**
   * The instruction `name` at the forwarder's address: `accounts` by their IDL
   * names, every one the IDL lists, in its order and with its flags, then
   * `remaining`, and `args` encoded by the build's coder.
   */
  protected instruction(
    name: string,
    accounts: Record<string, PublicKey>,
    args: object,
    remaining: AccountMeta[] = [],
  ): TransactionInstruction {
    const ix = this.idl.instructions.find((i) => i.name === name);
    if (ix === undefined) throw new Error(`the IDL of ${this.idl.metadata.name} declares no instruction ${name}`);
    const names = ix.accounts.map((a) => a.name);
    const unknown = Object.keys(accounts).filter((n) => !names.includes(n));
    if (unknown.length > 0) throw new Error(`${name} takes no account ${unknown.join(", ")}`);
    const keys = ix.accounts.map((a) => {
      if ("accounts" in a) throw new Error(`${name}: the account group ${a.name} is not supported`);
      const pubkey = accounts[a.name];
      if (pubkey === undefined) throw new Error(`${name} takes an account ${a.name} not given`);
      return { pubkey, isSigner: a.signer ?? false, isWritable: a.writable ?? false };
    });
    return new TransactionInstruction({
      programId: this.programId,
      keys: [...keys, ...remaining],
      data: this.coder.instruction.encode(name, args),
    });
  }

  /** The accounts Anchor's `#[event_cpi]` adds to an instruction that emits events. */
  protected events(): Record<string, PublicKey> {
    return { event_authority: deriveEventAuthorityPda(this.programId), program: this.programId };
  }

  protected configPda(): PublicKey {
    return deriveForwarderConfigPda(this.programId)[0];
  }

  /** `initialize`, by `authority`, the program's upgrade authority. */
  initializeIx(
    authority: PublicKey,
    protocolAdapter: PublicKey,
    logicRef: Uint8Array,
    initialOwner: PublicKey,
  ): TransactionInstruction {
    return this.instruction(
      "initialize",
      {
        authority,
        config: this.configPda(),
        program: this.programId,
        program_data: deriveProgramDataAddress(this.programId),
        system_program: SystemProgram.programId,
        event_authority: deriveEventAuthorityPda(this.programId),
        upgrade_authority: deriveUpgradeAuthorityPda(this.programId),
        bpf_loader_upgradeable: BPF_LOADER_UPGRADEABLE,
      },
      { protocol_adapter: protocolAdapter, logic_ref: Array.from(logicRef), initial_owner: initialOwner },
    );
  }

  /** `init_nonce_bitmap` of `user`'s word `wordIndex`, `payer` funding the rent. */
  initNonceBitmapIx(payer: PublicKey, user: PublicKey, wordIndex: bigint): TransactionInstruction {
    return this.instruction(
      "init_nonce_bitmap",
      {
        payer,
        nonce_bitmap: deriveNonceBitmapPda(this.programId, user, wordIndex)[0],
        system_program: SystemProgram.programId,
      },
      { _user: user, _word_index: u64(wordIndex) },
    );
  }

  /** `transfer_ownership` to `newOwner`, by the owner. */
  transferOwnershipIx(owner: PublicKey, newOwner: PublicKey): TransactionInstruction {
    return this.instruction(
      "transfer_ownership",
      { authority: owner, config: this.configPda(), ...this.events() },
      { new_owner: newOwner },
    );
  }

  /** `renounce_ownership`, by the owner. */
  renounceOwnershipIx(owner: PublicKey): TransactionInstruction {
    return this.instruction("renounce_ownership", { authority: owner, config: this.configPda(), ...this.events() }, {});
  }

  /** `upgrade` to `buffer`'s code, by the owner, after the compute budget it needs; the buffer's rent goes to `spill`. */
  upgradeIxs(owner: PublicKey, buffer: PublicKey, spill: PublicKey): [TransactionInstruction, TransactionInstruction] {
    return [
      ComputeBudgetProgram.setComputeUnitLimit({ units: UPGRADE_COMPUTE_UNIT_LIMIT }),
      this.instruction(
        "upgrade",
        {
          authority: owner,
          config: this.configPda(),
          program_data: deriveProgramDataAddress(this.programId),
          upgraded_program: this.programId,
          buffer,
          spill,
          upgrade_authority: deriveUpgradeAuthorityPda(this.programId),
          rent: SYSVAR_RENT_PUBKEY,
          clock: SYSVAR_CLOCK_PUBKEY,
          bpf_loader_upgradeable: BPF_LOADER_UPGRADEABLE,
          ...this.events(),
        },
        {},
      ),
    ];
  }

  /** `version`. */
  versionIx(): TransactionInstruction {
    return this.instruction("version", {}, {});
  }

  /** `close_escrow` of `mint`'s escrow into `recipientAta`, by the owner, while the adapter whose state is `paState` is paused. */
  closeEscrowIx(owner: PublicKey, paState: PublicKey, mint: PublicKey, recipientAta: PublicKey): TransactionInstruction {
    const [escrowAuthority] = deriveForwarderEscrowAuthority(this.programId);
    return this.instruction(
      "close_escrow",
      {
        authority: owner,
        config: this.configPda(),
        escrow_ata: getAssociatedTokenAddressSync(mint, escrowAuthority, true),
        escrow_authority: escrowAuthority,
        recipient_ata: recipientAta,
        token_mint: mint,
        token_program: TOKEN_PROGRAM_ID,
        pa_state: paState,
      },
      {},
    );
  }

  /** `close_nonce_bitmaps_batch` of `bitmaps`, by the owner, while the adapter whose state is `paState` is paused. */
  closeNonceBitmapsBatchIx(owner: PublicKey, paState: PublicKey, bitmaps: PublicKey[]): TransactionInstruction {
    return this.instruction(
      "close_nonce_bitmaps_batch",
      { authority: owner, config: this.configPda(), pa_state: paState },
      {},
      bitmaps.map((pubkey) => ({ pubkey, isSigner: false, isWritable: true })),
    );
  }

  /** `close_config`, by the owner, while the adapter whose state is `paState` is paused. */
  closeConfigIx(owner: PublicKey, paState: PublicKey): TransactionInstruction {
    return this.instruction("close_config", { authority: owner, config: this.configPda(), pa_state: paState }, {});
  }
}

/** The production build at an address. */
export class SplTokenForwarder extends ForwarderInstance {
  private constructor(connection: Connection, programId: PublicKey) {
    super(connection, programId, idl, coder);
  }

  /** The forwarder at `programId`, as alloy's `new(address, provider)`. */
  static at(connection: Connection, programId: PublicKey): SplTokenForwarder {
    return new SplTokenForwarder(connection, programId);
  }

  /** `reinitialize` to the resource logic `logicRef`, by the owner. */
  reinitializeIx(owner: PublicKey, logicRef: Uint8Array): TransactionInstruction {
    return this.instruction(
      "reinitialize",
      { authority: owner, config: this.configPda(), ...this.events() },
      { logic_ref: Array.from(logicRef) },
    );
  }
}

/** The emergency-migrating build at an address. */
export class EmergencyMigratingSplTokenForwarder extends ForwarderInstance {
  private constructor(connection: Connection, programId: PublicKey) {
    super(connection, programId, emergencyMigratingIdl, emergencyMigratingCoder);
  }

  /** The forwarder at `programId`, as alloy's `new(address, provider)`. */
  static at(connection: Connection, programId: PublicKey): EmergencyMigratingSplTokenForwarder {
    return new EmergencyMigratingSplTokenForwarder(connection, programId);
  }

  /**
   * `isLogicRefVulnerable(logicRef)`: the forwarder lists `logicRef`. False
   * before `reinitialize` has listed any, as the EVM draft's empty storage
   * answers.
   */
  async isLogicRefVulnerable(logicRef: Uint8Array): Promise<boolean> {
    const [address] = deriveVulnerableLogicRefsPda(this.programId);
    const what = "the vulnerable logic refs";
    const data = await this.ownedData(address, what);
    if (data === null) return false;
    let listed: Uint8Array[];
    try {
      listed = decodeVulnerableLogicRefs(data);
    } catch (error) {
      throw new InstanceError({ kind: "Decode", what: `${what} ${address.toBase58()}`, error: String(error) }, { cause: error });
    }
    return listed.some((r) => r.length === logicRef.length && r.every((b, i) => b === logicRef[i]));
  }

  /**
   * `isNullifierMigrated(nullifier)`: the resource with `nullifier` has a
   * migration record that says so. False without a record, as the EVM draft's
   * empty storage answers.
   */
  async isNullifierMigrated(nullifier: Uint8Array): Promise<boolean> {
    const [address] = deriveMigrationRecordPda(this.programId, nullifier);
    const what = "the migration record";
    const data = await this.ownedData(address, what);
    if (data === null) return false;
    try {
      return emergencyMigratingCoder.accounts.decode("MigrationRecord", data).migrated;
    } catch (error) {
      throw new InstanceError({ kind: "Decode", what: `${what} ${address.toBase58()}`, error: String(error) }, { cause: error });
    }
  }

  /** `LEGACY_FORWARDER()`: the legacy forwarder whose resources migrate too, as the build's IDL records it. */
  legacyForwarder(): PublicKey {
    return new PublicKey(idlConstant(emergencyMigratingIdl, "LEGACY_FORWARDER"));
  }

  /**
   * `reinitialize`, by the owner: lists `vulnerableLogicRefs`, which the
   * adapter `protocolAdapter` denies, and sets the logic ref to `newLogicRef`.
   */
  reinitializeIx(
    owner: PublicKey,
    protocolAdapter: PublicKey,
    newLogicRef: Uint8Array,
    vulnerableLogicRefs: Uint8Array[],
  ): TransactionInstruction {
    return this.instruction(
      "reinitialize",
      {
        authority: owner,
        config: this.configPda(),
        vulnerable_logic_refs: deriveVulnerableLogicRefsPda(this.programId)[0],
        pa_state: derivePaStatePda(protocolAdapter)[0],
        system_program: SystemProgram.programId,
        ...this.events(),
      },
      { new_logic_ref: Array.from(newLogicRef), vulnerable_logic_refs: vulnerableLogicRefs.map((r) => Array.from(r)) },
    );
  }

  /** `init_migration_record` of the resource with `nullifier`, `payer` funding the rent. */
  initMigrationRecordIx(payer: PublicKey, nullifier: Uint8Array): TransactionInstruction {
    return this.instruction(
      "init_migration_record",
      {
        payer,
        migration_record: deriveMigrationRecordPda(this.programId, nullifier)[0],
        system_program: SystemProgram.programId,
      },
      { _nullifier: Array.from(nullifier) },
    );
  }
}

/** The string `program` returned, as a simulation reports it: a Borsh `String`, all of the return data. */
export function returnedString(program: PublicKey, returnData: TransactionReturnData | null): string {
  const what = `the return data of ${program.toBase58()}`;
  const fail = (error: string) => new InstanceError({ kind: "Decode", what, error });
  if (returnData === null) throw fail("the simulation returned nothing");
  if (returnData.programId !== program.toBase58()) throw fail(`${returnData.programId} returned it`);
  const bytes = Uint8Array.from(atob(returnData.data[0]), (c) => c.charCodeAt(0));
  if (bytes.length < 4) throw fail(`${bytes.length} byte(s), shorter than a string's length`);
  const length = new DataView(bytes.buffer).getUint32(0, true);
  if (bytes.length !== 4 + length) throw fail(`${bytes.length} byte(s), not one string of ${length}`);
  try {
    return new TextDecoder("utf-8", { fatal: true }).decode(bytes.subarray(4));
  } catch (error) {
    throw fail(`not UTF-8: ${error}`);
  }
}
