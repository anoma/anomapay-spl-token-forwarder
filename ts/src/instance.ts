// The forwarder deployed at an address, as the EVM bindings return the ERC20
// forwarder: its reads under their EVM names, from its config and the
// emergency-migrating build's accounts, and its instructions, built for its
// address with its build's IDL, as the Rust crate's `instance` reads and
// builds them. The reads and instructions both builds declare are written
// once (`ForwarderInstance`); each build's class adds its own.

import { BorshCoder, type Idl } from "@anchor-lang/core";
import { getSetComputeUnitLimitInstruction } from "@solana-program/compute-budget";
import { TOKEN_PROGRAM_ADDRESS } from "@solana-program/token";
import {
  AccountRole,
  address,
  appendTransactionMessageInstruction,
  blockhash,
  compileTransaction,
  createTransactionMessage,
  getBase64EncodedWireTransaction,
  getBase64Encoder,
  pipe,
  setTransactionMessageFeePayer,
  setTransactionMessageLifetimeUsingBlockhash,
  type AccountMeta,
  type Address,
  type Instruction,
  type PendingRpcRequest,
  type Rpc,
  type SolanaRpcApiMainnet,
  type TransactionError,
} from "@solana/kit";
import { SYSVAR_CLOCK_ADDRESS, SYSVAR_RENT_ADDRESS } from "@solana/sysvars";

import { decodeVulnerableLogicRefs } from "./accounts.js";
import { coder, emergencyMigratingCoder } from "./coder.js";
import { UPGRADE_COMPUTE_UNIT_LIMIT } from "./constants.js";
import { chainIdOfGenesisHash, deployment, type Deployment, type DeploymentRecord, type Environment } from "./deployments.js";
import { emergencyMigratingIdl, idl, idlConstant } from "./idl.js";
import { toJson } from "./json.js";
import {
  BPF_LOADER_UPGRADEABLE,
  deriveEventAuthorityPda,
  deriveForwarderConfigPda,
  deriveForwarderEscrowAuthority,
  deriveMigrationRecordPda,
  deriveNonceBitmapPda,
  derivePaStatePda,
  deriveProgramDataAddress,
  deriveTokenAccount,
  deriveUpgradeAuthorityPda,
  deriveVulnerableLogicRefsPda,
} from "./pda.js";

/** The system program, which creates the accounts the forwarder's instructions initialize. */
const SYSTEM_PROGRAM = address("11111111111111111111111111111111");

/** Why an instance could not be made or could not answer. */
export type InstanceErrorDetail =
  /** The environment records no deployed forwarder on the cluster. */
  | { kind: "NoDeployment"; chainId: string }
  /** An account the answer is read from does not exist. */
  | { kind: "AccountMissing"; address: Address }
  /** An RPC request failed: retrying may succeed. */
  | { kind: "Rpc"; what: string; error: string }
  /** What the chain holds does not decode. */
  | { kind: "Decode"; what: string; error: string }
  /** A simulated instruction failed. */
  | { kind: "Simulation"; error: TransactionError; logs: readonly string[] };

function describe(detail: InstanceErrorDetail): string {
  switch (detail.kind) {
    case "NoDeployment":
      return `no deployed forwarder is recorded on ${detail.chainId}`;
    case "AccountMissing":
      return `the account ${detail.address} does not exist`;
    case "Rpc":
      return `failed to fetch ${detail.what}: ${detail.error}`;
    case "Decode":
      return `${detail.what} does not decode: ${detail.error}`;
    case "Simulation":
      return `the simulation failed: ${toJson(detail.error)}; logs: ${toJson(detail.logs)}`;
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

/** `request` sent, an RPC failure thrown as `Rpc`. */
async function send<T>(what: string, request: PendingRpcRequest<T>): Promise<T> {
  try {
    return await request.send();
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

async function recordedProgram(rpc: Rpc<SolanaRpcApiMainnet>, environment: Environment): Promise<Address> {
  const genesisHash = await send("the genesis hash", rpc.getGenesisHash());
  return recordedDeployment(environment, genesisHash).programId;
}

/** The forwarder `environment` records on the cluster behind `rpc`, as the EVM bindings return the ERC20 forwarder. */
export async function splTokenForwarder(rpc: Rpc<SolanaRpcApiMainnet>, environment: Environment): Promise<SplTokenForwarder> {
  return SplTokenForwarder.at(rpc, await recordedProgram(rpc, environment));
}

/** The forwarder `environment` records on the cluster behind `rpc`, read as the emergency-migrating build. */
export async function emergencyMigratingSplTokenForwarder(
  rpc: Rpc<SolanaRpcApiMainnet>,
  environment: Environment,
): Promise<EmergencyMigratingSplTokenForwarder> {
  return EmergencyMigratingSplTokenForwarder.at(rpc, await recordedProgram(rpc, environment));
}

/** The role of an account with the IDL's `signer` and `writable` flags. */
function role(signer: boolean, writable: boolean): AccountRole {
  if (signer) return writable ? AccountRole.WRITABLE_SIGNER : AccountRole.READONLY_SIGNER;
  return writable ? AccountRole.WRITABLE : AccountRole.READONLY;
}

/** The forwarder at an address, read over an RPC at its commitment, as the build whose IDL is `idl`. */
abstract class ForwarderInstance {
  protected constructor(
    protected readonly rpc: Rpc<SolanaRpcApiMainnet>,
    /** The address the forwarder is deployed at. */
    readonly programId: Address,
    private readonly idl: Idl,
    private readonly coder: BorshCoder,
  ) {}

  /** The forwarder's config, as the coder decodes `Config`: its IDL field names and the coder's values. */
  private async config(): Promise<any> {
    const [address] = await deriveForwarderConfigPda(this.programId);
    const what = `the forwarder's config ${address}`;
    const account = await this.account(address, what);
    if (account === null) throw new InstanceError({ kind: "AccountMissing", address });
    try {
      return this.coder.accounts.decode("Config", account.data);
    } catch (error) {
      throw new InstanceError({ kind: "Decode", what, error: String(error) }, { cause: error });
    }
  }

  /** The data and owner of the account at `address`, or `null` when it does not exist. */
  private async account(address: Address, what: string): Promise<{ data: Uint8Array; owner: Address } | null> {
    const { value } = await send(what, this.rpc.getAccountInfo(address, { encoding: "base64" }));
    if (value === null) return null;
    return { data: Uint8Array.from(getBase64Encoder().encode(value.data[0])), owner: value.owner };
  }

  /** The data of the account at `address` when it exists and the forwarder owns it: an account the forwarder created. */
  protected async ownedData(address: Address, what: string): Promise<Uint8Array | null> {
    const account = await this.account(address, `${what} ${address}`);
    return account !== null && account.owner === this.programId ? account.data : null;
  }

  /** `getLogicRef()`. */
  async getLogicRef(): Promise<number[]> {
    return (await this.config()).logic_ref;
  }

  /** `getProtocolAdapter()`. */
  async getProtocolAdapter(): Promise<Address> {
    return (await this.config()).protocol_adapter;
  }

  /** `owner()`: the zero key once the ownership is renounced. */
  async owner(): Promise<Address> {
    return (await this.config()).owner;
  }

  /**
   * `VERSION()`: the forwarder's `version` simulated, with `feePayer`, any
   * funded system account, paying the simulation's fee (nothing is signed or
   * sent).
   */
  async version(feePayer: Address): Promise<string> {
    const transaction = compileTransaction(
      pipe(
        createTransactionMessage({ version: "legacy" }),
        (m) => setTransactionMessageFeePayer(feePayer, m),
        // The RPC replaces it with its latest.
        (m) =>
          setTransactionMessageLifetimeUsingBlockhash(
            { blockhash: blockhash("11111111111111111111111111111111"), lastValidBlockHeight: 0n },
            m,
          ),
        (m) => appendTransactionMessageInstruction(this.versionIx(), m),
      ),
    );
    const { value } = await send(
      "the simulation of version",
      this.rpc.simulateTransaction(getBase64EncodedWireTransaction(transaction), {
        encoding: "base64",
        sigVerify: false,
        replaceRecentBlockhash: true,
      }),
    );
    if (value.err !== null) {
      throw new InstanceError({ kind: "Simulation", error: value.err, logs: value.logs ?? [] });
    }
    return returnedString(this.programId, value.returnData);
  }

  /**
   * The instruction `name` at the forwarder's address: `accounts` by their IDL
   * names, every one the IDL lists, in its order and with its flags, then
   * `remaining`, and `args` encoded by the build's coder.
   */
  protected instruction(
    name: string,
    accounts: Record<string, Address>,
    args: object,
    remaining: AccountMeta[] = [],
  ): Instruction {
    const ix = this.idl.instructions.find((i) => i.name === name);
    if (ix === undefined) throw new Error(`the IDL of ${this.idl.metadata.name} declares no instruction ${name}`);
    const names = ix.accounts.map((a) => a.name);
    const unknown = Object.keys(accounts).filter((n) => !names.includes(n));
    if (unknown.length > 0) throw new Error(`${name} takes no account ${unknown.join(", ")}`);
    const metas = ix.accounts.map((a): AccountMeta => {
      if ("accounts" in a) throw new Error(`${name}: the account group ${a.name} is not supported`);
      const account = accounts[a.name];
      if (account === undefined) throw new Error(`${name} takes an account ${a.name} not given`);
      return { address: account, role: role(a.signer ?? false, a.writable ?? false) };
    });
    return {
      programAddress: this.programId,
      accounts: [...metas, ...remaining],
      data: this.coder.instruction.encode(name, args),
    };
  }

  /** The accounts Anchor's `#[event_cpi]` adds to an instruction that emits events. */
  protected async events(): Promise<Record<string, Address>> {
    return { event_authority: await deriveEventAuthorityPda(this.programId), program: this.programId };
  }

  protected async configPda(): Promise<Address> {
    return (await deriveForwarderConfigPda(this.programId))[0];
  }

  /** `initialize`, by `authority`, the program's upgrade authority. */
  async initializeIx(
    authority: Address,
    protocolAdapter: Address,
    logicRef: Uint8Array,
    initialOwner: Address,
  ): Promise<Instruction> {
    return this.instruction(
      "initialize",
      {
        authority,
        config: await this.configPda(),
        program: this.programId,
        program_data: await deriveProgramDataAddress(this.programId),
        system_program: SYSTEM_PROGRAM,
        event_authority: await deriveEventAuthorityPda(this.programId),
        upgrade_authority: await deriveUpgradeAuthorityPda(this.programId),
        bpf_loader_upgradeable: BPF_LOADER_UPGRADEABLE,
      },
      { protocol_adapter: protocolAdapter, logic_ref: Array.from(logicRef), initial_owner: initialOwner },
    );
  }

  /** `init_nonce_bitmap` of `user`'s word `wordIndex`, `payer` funding the rent. */
  async initNonceBitmapIx(payer: Address, user: Address, wordIndex: bigint): Promise<Instruction> {
    return this.instruction(
      "init_nonce_bitmap",
      {
        payer,
        nonce_bitmap: (await deriveNonceBitmapPda(this.programId, user, wordIndex))[0],
        system_program: SYSTEM_PROGRAM,
      },
      { user, word_index: wordIndex },
    );
  }

  /** `transfer_ownership` to `newOwner`, by the owner. */
  async transferOwnershipIx(owner: Address, newOwner: Address): Promise<Instruction> {
    return this.instruction(
      "transfer_ownership",
      { authority: owner, config: await this.configPda(), ...(await this.events()) },
      { new_owner: newOwner },
    );
  }

  /** `renounce_ownership`, by the owner. */
  async renounceOwnershipIx(owner: Address): Promise<Instruction> {
    return this.instruction(
      "renounce_ownership",
      { authority: owner, config: await this.configPda(), ...(await this.events()) },
      {},
    );
  }

  /** `upgrade` to `buffer`'s code, by the owner, after the compute budget it needs; the buffer's rent goes to `spill`. */
  async upgradeIxs(owner: Address, buffer: Address, spill: Address): Promise<[Instruction, Instruction]> {
    return [
      getSetComputeUnitLimitInstruction({ units: UPGRADE_COMPUTE_UNIT_LIMIT }),
      this.instruction(
        "upgrade",
        {
          authority: owner,
          config: await this.configPda(),
          program_data: await deriveProgramDataAddress(this.programId),
          upgraded_program: this.programId,
          buffer,
          spill,
          upgrade_authority: await deriveUpgradeAuthorityPda(this.programId),
          rent: SYSVAR_RENT_ADDRESS,
          clock: SYSVAR_CLOCK_ADDRESS,
          bpf_loader_upgradeable: BPF_LOADER_UPGRADEABLE,
          ...(await this.events()),
        },
        {},
      ),
    ];
  }

  /** `version`. */
  versionIx(): Instruction {
    return this.instruction("version", {}, {});
  }

  /** `close_escrow` of `mint`'s escrow into `recipientAta`, by the owner, while the adapter whose state is `paState` is paused. */
  async closeEscrowIx(owner: Address, paState: Address, mint: Address, recipientAta: Address): Promise<Instruction> {
    const [escrowAuthority] = await deriveForwarderEscrowAuthority(this.programId);
    return this.instruction(
      "close_escrow",
      {
        authority: owner,
        config: await this.configPda(),
        escrow_ata: await deriveTokenAccount(escrowAuthority, mint),
        escrow_authority: escrowAuthority,
        recipient_ata: recipientAta,
        token_mint: mint,
        token_program: TOKEN_PROGRAM_ADDRESS,
        pa_state: paState,
      },
      {},
    );
  }

  /** `close_nonce_bitmaps_batch` of `bitmaps`, by the owner, while the adapter whose state is `paState` is paused. */
  async closeNonceBitmapsBatchIx(owner: Address, paState: Address, bitmaps: Address[]): Promise<Instruction> {
    return this.instruction(
      "close_nonce_bitmaps_batch",
      { authority: owner, config: await this.configPda(), pa_state: paState },
      {},
      bitmaps.map((bitmap) => ({ address: bitmap, role: AccountRole.WRITABLE })),
    );
  }

  /** `close_config`, by the owner, while the adapter whose state is `paState` is paused. */
  async closeConfigIx(owner: Address, paState: Address): Promise<Instruction> {
    return this.instruction("close_config", { authority: owner, config: await this.configPda(), pa_state: paState }, {});
  }
}

/** The production build at an address. */
export class SplTokenForwarder extends ForwarderInstance {
  private constructor(rpc: Rpc<SolanaRpcApiMainnet>, programId: Address) {
    super(rpc, programId, idl, coder);
  }

  /** The forwarder at `programId`, as alloy's `new(address, provider)`. */
  static at(rpc: Rpc<SolanaRpcApiMainnet>, programId: Address): SplTokenForwarder {
    return new SplTokenForwarder(rpc, programId);
  }

  /** `reinitialize` to the resource logic `logicRef`, by the owner. */
  async reinitializeIx(owner: Address, logicRef: Uint8Array): Promise<Instruction> {
    return this.instruction(
      "reinitialize",
      { authority: owner, config: await this.configPda(), ...(await this.events()) },
      { logic_ref: Array.from(logicRef) },
    );
  }
}

/** The emergency-migrating build at an address. */
export class EmergencyMigratingSplTokenForwarder extends ForwarderInstance {
  private constructor(rpc: Rpc<SolanaRpcApiMainnet>, programId: Address) {
    super(rpc, programId, emergencyMigratingIdl, emergencyMigratingCoder);
  }

  /** The forwarder at `programId`, as alloy's `new(address, provider)`. */
  static at(rpc: Rpc<SolanaRpcApiMainnet>, programId: Address): EmergencyMigratingSplTokenForwarder {
    return new EmergencyMigratingSplTokenForwarder(rpc, programId);
  }

  /**
   * `isLogicRefVulnerable(logicRef)`: the forwarder lists `logicRef`. False
   * before `reinitialize` has listed any, as the EVM draft's empty storage
   * answers.
   */
  async isLogicRefVulnerable(logicRef: Uint8Array): Promise<boolean> {
    const [address] = await deriveVulnerableLogicRefsPda(this.programId);
    const what = "the vulnerable logic refs";
    const data = await this.ownedData(address, what);
    if (data === null) return false;
    let listed: Uint8Array[];
    try {
      listed = decodeVulnerableLogicRefs(data);
    } catch (error) {
      throw new InstanceError({ kind: "Decode", what: `${what} ${address}`, error: String(error) }, { cause: error });
    }
    return listed.some((r) => r.length === logicRef.length && r.every((b, i) => b === logicRef[i]));
  }

  /**
   * `isNullifierMigrated(nullifier)`: the resource with `nullifier` has a
   * migration record that says so. False without a record, as the EVM draft's
   * empty storage answers.
   */
  async isNullifierMigrated(nullifier: Uint8Array): Promise<boolean> {
    const [address] = await deriveMigrationRecordPda(this.programId, nullifier);
    const what = "the migration record";
    const data = await this.ownedData(address, what);
    if (data === null) return false;
    try {
      return emergencyMigratingCoder.accounts.decode("MigrationRecord", data).migrated;
    } catch (error) {
      throw new InstanceError({ kind: "Decode", what: `${what} ${address}`, error: String(error) }, { cause: error });
    }
  }

  /** `LEGACY_FORWARDER()`: the legacy forwarder whose resources migrate too, as the build's IDL records it. */
  legacyForwarder(): Address {
    return address(idlConstant(emergencyMigratingIdl, "LEGACY_FORWARDER"));
  }

  /**
   * `reinitialize`, by the owner: lists `vulnerableLogicRefs`, which the
   * adapter `protocolAdapter` denies, and sets the logic ref to `newLogicRef`.
   */
  async reinitializeIx(
    owner: Address,
    protocolAdapter: Address,
    newLogicRef: Uint8Array,
    vulnerableLogicRefs: Uint8Array[],
  ): Promise<Instruction> {
    return this.instruction(
      "reinitialize",
      {
        authority: owner,
        config: await this.configPda(),
        vulnerable_logic_refs: (await deriveVulnerableLogicRefsPda(this.programId))[0],
        pa_state: (await derivePaStatePda(protocolAdapter))[0],
        system_program: SYSTEM_PROGRAM,
        ...(await this.events()),
      },
      { new_logic_ref: Array.from(newLogicRef), vulnerable_logic_refs: vulnerableLogicRefs.map((r) => Array.from(r)) },
    );
  }

  /** `init_migration_record` of the resource with `nullifier`, `payer` funding the rent. */
  async initMigrationRecordIx(payer: Address, nullifier: Uint8Array): Promise<Instruction> {
    return this.instruction(
      "init_migration_record",
      {
        payer,
        migration_record: (await deriveMigrationRecordPda(this.programId, nullifier))[0],
        system_program: SYSTEM_PROGRAM,
      },
      { nullifier: Array.from(nullifier) },
    );
  }
}

/** The string `program` returned, as a simulation reports it: a Borsh `String`, all of the return data. */
export function returnedString(
  program: Address,
  returnData: Readonly<{ data: readonly [string, "base64"]; programId: Address }> | null,
): string {
  const what = `the return data of ${program}`;
  const fail = (error: string) => new InstanceError({ kind: "Decode", what, error });
  if (returnData === null) throw fail("the simulation returned nothing");
  if (returnData.programId !== program) throw fail(`${returnData.programId} returned it`);
  const bytes = Uint8Array.from(getBase64Encoder().encode(returnData.data[0]));
  if (bytes.length < 4) throw fail(`${bytes.length} byte(s), shorter than a string's length`);
  const length = new DataView(bytes.buffer).getUint32(0, true);
  if (bytes.length !== 4 + length) throw fail(`${bytes.length} byte(s), not one string of ${length}`);
  try {
    return new TextDecoder("utf-8", { fatal: true }).decode(bytes.subarray(4));
  } catch (error) {
    throw fail(`not UTF-8: ${error}`);
  }
}
