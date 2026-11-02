// The forwarder deployed at an address, as the EVM bindings return the ERC20
// forwarder (`contract::erc20_forwarder`): its reads under their EVM names,
// from its config and the emergency-migrating build's accounts, and its
// instructions, built for its address with its build's IDL, as the Rust
// crate's `contract` reads and builds them. The reads and instructions both
// builds declare are written once (`ForwarderInstance`); each build's class
// adds its own.

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
  fetchEncodedAccount,
  getBase64EncodedWireTransaction,
  getBase64Encoder,
  pipe,
  setTransactionMessageFeePayer,
  setTransactionMessageLifetimeUsingBlockhash,
  type AccountMeta,
  type Address,
  type Instruction,
  type Rpc,
  type SolanaRpcApiMainnet,
} from "@solana/kit";

import { decodeVulnerableLogicRefs } from "./accounts.js";
import { clusterOfGenesisHash, splTokenForwarderAddress, type Environment } from "./addresses.js";
import { coder, emergencyMigratingCoder } from "./coder.js";
import { UPGRADE_COMPUTE_UNIT_LIMIT } from "./constants.js";
import { BindingsError } from "./error.js";
import { accountRole, emergencyMigratingIdl, idl, idlConstant } from "./idl.js";
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
  SYSVAR_CLOCK_ADDRESS,
  SYSVAR_RENT_ADDRESS,
} from "./pda.js";

/** The system program, which creates the accounts the forwarder's instructions initialize. */
const SYSTEM_PROGRAM = address("11111111111111111111111111111111");

/** The RPC's answer `response`, a failed request thrown as `RpcTransportError`. */
async function answered<T>(what: string, response: Promise<T>): Promise<T> {
  try {
    return await response;
  } catch (error) {
    throw new BindingsError({ kind: "RpcTransportError", what, error: String(error) }, { cause: error });
  }
}

/** The forwarder `environment` records on the cluster whose genesis hash is `genesisHash`. */
export function recordedAddress(environment: Environment, genesisHash: string): Address {
  const cluster = clusterOfGenesisHash(genesisHash);
  if (cluster === undefined) throw new BindingsError({ kind: "ChainIdUnknown" });
  const recorded = splTokenForwarderAddress(environment, cluster);
  if (recorded === undefined) throw new BindingsError({ kind: "UnsupportedChain", cluster });
  return recorded;
}

/**
 * Returns an SPL token forwarder instance of the environment for the given
 * RPC. An emergency-migrating build at the recorded address is
 * `new EmergencyMigratingSplTokenForwarderInstance(instance.address, rpc)`.
 */
export async function splTokenForwarder(
  rpc: Rpc<SolanaRpcApiMainnet>,
  environment: Environment,
): Promise<SplTokenForwarderInstance> {
  const genesisHash = await answered("the genesis hash", rpc.getGenesisHash().send());
  return new SplTokenForwarderInstance(recordedAddress(environment, genesisHash), rpc);
}

/** The forwarder at an address, read over an RPC at its commitment, as the build whose IDL is `idl`. */
abstract class ForwarderInstance {
  protected constructor(
    /** The address the forwarder is deployed at. */
    readonly address: Address,
    protected readonly rpc: Rpc<SolanaRpcApiMainnet>,
    private readonly idl: Idl,
    private readonly coder: BorshCoder,
  ) {}

  /** The PDAs of the forwarder's address alone, derived on first use. */
  private derived?: Promise<{ config: Address; eventAuthority: Address; programData: Address; upgradeAuthority: Address }>;

  private pdas() {
    this.derived ??= Promise.all([
      deriveForwarderConfigPda(this.address),
      deriveEventAuthorityPda(this.address),
      deriveProgramDataAddress(this.address),
      deriveUpgradeAuthorityPda(this.address),
    ]).then(([[config], eventAuthority, programData, upgradeAuthority]) => ({
      config,
      eventAuthority,
      programData,
      upgradeAuthority,
    }));
    return this.derived;
  }

  /** The forwarder's config, as the coder decodes `Config`: its IDL field names and the coder's values. */
  private async config(): Promise<any> {
    const { config: address } = await this.pdas();
    const what = `the forwarder's config ${address}`;
    const account = await answered(what, fetchEncodedAccount(this.rpc, address));
    if (!account.exists) throw new BindingsError({ kind: "AccountMissing", address });
    try {
      return this.coder.accounts.decode("Config", account.data);
    } catch (error) {
      throw new BindingsError({ kind: "Decode", what, error: String(error) }, { cause: error });
    }
  }

  /** The data of the account at `address` when it exists and the forwarder owns it: an account the forwarder created. */
  protected async ownedData(address: Address, what: string): Promise<Uint8Array | null> {
    const account = await answered(`${what} ${address}`, fetchEncodedAccount(this.rpc, address));
    return account.exists && account.programAddress === this.address ? account.data : null;
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
    const { value } = await answered(
      "the simulation of version",
      this.rpc
        .simulateTransaction(getBase64EncodedWireTransaction(transaction), {
          encoding: "base64",
          sigVerify: false,
          replaceRecentBlockhash: true,
        })
        .send(),
    );
    if (value.err !== null) {
      throw new BindingsError({ kind: "Simulation", error: value.err, logs: value.logs ?? [] });
    }
    return returnedString(this.address, value.returnData);
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
      return { address: account, role: accountRole(a.signer ?? false, a.writable ?? false) };
    });
    return {
      programAddress: this.address,
      accounts: [...metas, ...remaining],
      data: this.coder.instruction.encode(name, args),
    };
  }

  /** The forwarder's config, and the accounts Anchor's `#[event_cpi]` adds to an instruction that emits events. */
  protected async configAndEvents(): Promise<Record<string, Address>> {
    const { config, eventAuthority } = await this.pdas();
    return { config, event_authority: eventAuthority, program: this.address };
  }

  /** `initialize`, by `authority`, the program's upgrade authority. */
  async initializeIx(
    authority: Address,
    protocolAdapter: Address,
    logicRef: Uint8Array,
    initialOwner: Address,
  ): Promise<Instruction> {
    const { programData, upgradeAuthority } = await this.pdas();
    return this.instruction(
      "initialize",
      {
        authority,
        ...(await this.configAndEvents()),
        program_data: programData,
        system_program: SYSTEM_PROGRAM,
        upgrade_authority: upgradeAuthority,
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
        nonce_bitmap: (await deriveNonceBitmapPda(this.address, user, wordIndex))[0],
        system_program: SYSTEM_PROGRAM,
      },
      { user, word_index: wordIndex },
    );
  }

  /** `transfer_ownership` to `newOwner`, by the owner. */
  async transferOwnershipIx(owner: Address, newOwner: Address): Promise<Instruction> {
    return this.instruction(
      "transfer_ownership",
      { authority: owner, ...(await this.configAndEvents()) },
      { new_owner: newOwner },
    );
  }

  /** `renounce_ownership`, by the owner. */
  async renounceOwnershipIx(owner: Address): Promise<Instruction> {
    return this.instruction(
      "renounce_ownership",
      { authority: owner, ...(await this.configAndEvents()) },
      {},
    );
  }

  /**
   * `upgrade` to `buffer`'s code, by the owner, after the compute budget it
   * needs; the buffer's rent goes to `spill`. kit refuses the writable invoked
   * program: compile with `compileLegacyTransaction`.
   */
  async upgradeIxs(owner: Address, buffer: Address, spill: Address): Promise<[Instruction, Instruction]> {
    const { programData, upgradeAuthority } = await this.pdas();
    return [
      getSetComputeUnitLimitInstruction({ units: UPGRADE_COMPUTE_UNIT_LIMIT }),
      this.instruction(
        "upgrade",
        {
          authority: owner,
          program_data: programData,
          upgraded_program: this.address,
          buffer,
          spill,
          upgrade_authority: upgradeAuthority,
          rent: SYSVAR_RENT_ADDRESS,
          clock: SYSVAR_CLOCK_ADDRESS,
          bpf_loader_upgradeable: BPF_LOADER_UPGRADEABLE,
          ...(await this.configAndEvents()),
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
    const [escrowAuthority] = await deriveForwarderEscrowAuthority(this.address);
    return this.instruction(
      "close_escrow",
      {
        authority: owner,
        config: (await this.pdas()).config,
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
      { authority: owner, config: (await this.pdas()).config, pa_state: paState },
      {},
      bitmaps.map((bitmap) => ({ address: bitmap, role: AccountRole.WRITABLE })),
    );
  }

  /** `close_config`, by the owner, while the adapter whose state is `paState` is paused. */
  async closeConfigIx(owner: Address, paState: Address): Promise<Instruction> {
    return this.instruction("close_config", { authority: owner, config: (await this.pdas()).config, pa_state: paState }, {});
  }
}

/** The production build at an address. */
export class SplTokenForwarderInstance extends ForwarderInstance {
  /** The forwarder at `address`, as alloy's `ERC20ForwarderInstance::new(address, provider)`. */
  constructor(address: Address, rpc: Rpc<SolanaRpcApiMainnet>) {
    super(address, rpc, idl, coder);
  }

  /** `reinitialize` to the resource logic `logicRef`, by the owner. */
  async reinitializeIx(owner: Address, logicRef: Uint8Array): Promise<Instruction> {
    return this.instruction(
      "reinitialize",
      { authority: owner, ...(await this.configAndEvents()) },
      { logic_ref: Array.from(logicRef) },
    );
  }
}

/** The emergency-migrating build at an address. */
export class EmergencyMigratingSplTokenForwarderInstance extends ForwarderInstance {
  /** The forwarder at `address`, as alloy's `new(address, provider)`. */
  constructor(address: Address, rpc: Rpc<SolanaRpcApiMainnet>) {
    super(address, rpc, emergencyMigratingIdl, emergencyMigratingCoder);
  }

  /**
   * `isLogicRefVulnerable(logicRef)`: the forwarder lists `logicRef`. False
   * before `reinitialize` has listed any, as the EVM draft's empty storage
   * answers.
   */
  async isLogicRefVulnerable(logicRef: Uint8Array): Promise<boolean> {
    const [address] = await deriveVulnerableLogicRefsPda(this.address);
    const what = "the vulnerable logic refs";
    const data = await this.ownedData(address, what);
    if (data === null) return false;
    let listed: Uint8Array[];
    try {
      listed = decodeVulnerableLogicRefs(data);
    } catch (error) {
      throw new BindingsError({ kind: "Decode", what: `${what} ${address}`, error: String(error) }, { cause: error });
    }
    return listed.some((r) => r.length === logicRef.length && r.every((b, i) => b === logicRef[i]));
  }

  /**
   * `isNullifierMigrated(nullifier)`: the resource with `nullifier` has a
   * migration record that says so. False without a record, as the EVM draft's
   * empty storage answers.
   */
  async isNullifierMigrated(nullifier: Uint8Array): Promise<boolean> {
    const [address] = await deriveMigrationRecordPda(this.address, nullifier);
    const what = "the migration record";
    const data = await this.ownedData(address, what);
    if (data === null) return false;
    try {
      return emergencyMigratingCoder.accounts.decode("MigrationRecord", data).migrated;
    } catch (error) {
      throw new BindingsError({ kind: "Decode", what: `${what} ${address}`, error: String(error) }, { cause: error });
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
        vulnerable_logic_refs: (await deriveVulnerableLogicRefsPda(this.address))[0],
        pa_state: (await derivePaStatePda(protocolAdapter))[0],
        system_program: SYSTEM_PROGRAM,
        ...(await this.configAndEvents()),
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
        migration_record: (await deriveMigrationRecordPda(this.address, nullifier))[0],
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
  const fail = (error: string) => new BindingsError({ kind: "Decode", what, error });
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
