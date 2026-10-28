import { describe, expect, it } from "vitest";
import type { Idl } from "@anchor-lang/core";
import { ComputeBudgetProgram, Connection, Keypair, PublicKey, type TransactionInstruction } from "@solana/web3.js";

import { UPGRADE_COMPUTE_UNIT_LIMIT } from "./constants.js";
import type { DeploymentRecord } from "./deployments.js";
import { emergencyMigratingIdl, idl, idlConstant } from "./idl.js";
import {
  EmergencyMigratingSplTokenForwarder,
  InstanceError,
  recordedDeployment,
  returnedString,
  SplTokenForwarder,
} from "./instance.js";
import {
  deriveForwarderConfigPda,
  deriveForwarderEscrowAuthority,
  deriveProgramDataAddress,
  deriveVulnerableLogicRefsPda,
} from "./pda.js";

const DEVNET_GENESIS = "EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG";
const MAINNET_GENESIS = "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d";

/** The error `f` throws, which must be an `InstanceError`. */
function instanceError(f: () => unknown): InstanceError {
  try {
    f();
  } catch (error) {
    expect(error).toBeInstanceOf(InstanceError);
    return error as InstanceError;
  }
  throw new Error("no error was thrown");
}

describe("recordedDeployment", () => {
  it("is no deployment on devnet while staging's program is not deployed yet", () => {
    const error = instanceError(() => recordedDeployment("staging", DEVNET_GENESIS));
    console.log(error.message);
    expect(error.detail).toEqual({ kind: "NoDeployment", chainId: "solana:EtWTRABZaYq6iMfeYKouRu166VU2xqa1" });
  });

  it("is no deployment where production records none", () => {
    expect(instanceError(() => recordedDeployment("production", MAINNET_GENESIS)).detail).toEqual({
      kind: "NoDeployment",
      chainId: "solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp",
    });
  });

  // A cluster no environment records, such as a local validator, is no
  // deployment, under the chain id its genesis hash gives.
  it("is no deployment on an unrecorded cluster", () => {
    for (const environment of ["staging", "production"] as const) {
      expect(instanceError(() => recordedDeployment(environment, "11111111111111111111111111111111")).detail).toEqual({
        kind: "NoDeployment",
        chainId: "solana:11111111111111111111111111111111",
      });
    }
  });

  it("is the cluster's deployed entry", () => {
    const record: DeploymentRecord = {
      staging: [{ chainId: "solana:EtWTRABZaYq6iMfeYKouRu166VU2xqa1", programId: idl.address, deploymentSlot: 4242 }],
      production: [],
    };
    const found = recordedDeployment("staging", DEVNET_GENESIS, record);
    console.log(found);
    expect(found).toEqual({
      chainId: "solana:EtWTRABZaYq6iMfeYKouRu166VU2xqa1",
      programId: new PublicKey(idl.address),
      deploymentSlot: 4242,
    });
    expect(() => recordedDeployment("production", DEVNET_GENESIS, record)).toThrow(InstanceError);
  });
});

const connection = new Connection("http://127.0.0.1:1");
const key = (n: number) => new PublicKey(new Uint8Array(32).fill(n));
const bytes = (n: number) => new Uint8Array(32).fill(n);
const u64Le = (v: bigint) => {
  const b = new Uint8Array(8);
  new DataView(b.buffer).setBigUint64(0, v, true);
  return b;
};
const concat = (...parts: Uint8Array[]) => Uint8Array.from(Buffer.concat(parts));

/**
 * Checks `ix` against the instruction `name` of `of`: its data is the IDL's
 * discriminator then `args`, and each account has the IDL's flags, its fixed
 * address, or its PDA from the IDL's seeds when they are all constants, for
 * the program the IDL is built for.
 */
function expectMatchesTheIdl(of: Idl, ix: TransactionInstruction, name: string, args: Uint8Array) {
  const spec = of.instructions.find((i) => i.name === name)!;
  expect(Array.from(ix.data), `${name}: the discriminator, then the arguments`).toEqual([
    ...spec.discriminator,
    ...args,
  ]);
  expect(ix.keys.length, `${name}: accounts`).toBeGreaterThanOrEqual(spec.accounts.length);
  spec.accounts.forEach((account, i) => {
    if ("accounts" in account) throw new Error(`${name}: the account group ${account.name}`);
    const meta = ix.keys[i]!;
    expect(meta.isWritable, `${name}.${account.name} writable`).toBe(account.writable ?? false);
    expect(meta.isSigner, `${name}.${account.name} signer`).toBe(account.signer ?? false);
    if (account.address !== undefined) {
      expect(meta.pubkey.toBase58(), `${name}.${account.name} address`).toBe(account.address);
    }
    if (account.pda !== undefined && account.pda.seeds.every((s) => s.kind === "const")) {
      const seeds = account.pda.seeds.map((s) => Uint8Array.from((s as { value: number[] }).value));
      expect(meta.pubkey.toBase58(), `${name}.${account.name} PDA`).toBe(
        PublicKey.findProgramAddressSync(seeds, ix.programId)[0].toBase58(),
      );
    }
  });
}

/**
 * `of`'s instructions, but `forward_call`: only the adapter makes that call,
 * inside a settlement, with the accounts the segment builders
 * (`build*ForwarderAccounts`) give it.
 */
const sentInstructions = (of: Idl) =>
  of.instructions
    .map((i) => i.name)
    .filter((n) => n !== "forward_call")
    .sort();

/** The builders both builds' instances share, each with its arguments' expected bytes. */
function sharedBuilders(f: SplTokenForwarder | EmergencyMigratingSplTokenForwarder): [string, TransactionInstruction, Uint8Array][] {
  return [
    ["initialize", f.initializeIx(key(1), key(2), bytes(3), key(4)), concat(key(2).toBytes(), bytes(3), key(4).toBytes())],
    ["init_nonce_bitmap", f.initNonceBitmapIx(key(1), key(2), 7n), concat(key(2).toBytes(), u64Le(7n))],
    ["transfer_ownership", f.transferOwnershipIx(key(1), key(2)), key(2).toBytes()],
    ["renounce_ownership", f.renounceOwnershipIx(key(1)), new Uint8Array()],
    ["upgrade", f.upgradeIxs(key(1), key(2), key(3))[1], new Uint8Array()],
    ["version", f.versionIx(), new Uint8Array()],
    ["close_escrow", f.closeEscrowIx(key(1), key(2), key(3), key(4)), new Uint8Array()],
    ["close_nonce_bitmaps_batch", f.closeNonceBitmapsBatchIx(key(1), key(2), [key(5), key(6)]), new Uint8Array()],
    ["close_config", f.closeConfigIx(key(1), key(2)), new Uint8Array()],
  ];
}

const productionBuilders = (f: SplTokenForwarder): [string, TransactionInstruction, Uint8Array][] => [
  ...sharedBuilders(f),
  ["reinitialize", f.reinitializeIx(key(1), bytes(3)), bytes(3)],
];

const emergencyMigratingBuilders = (f: EmergencyMigratingSplTokenForwarder): [string, TransactionInstruction, Uint8Array][] => [
  ...sharedBuilders(f),
  [
    "reinitialize",
    f.reinitializeIx(key(1), key(2), bytes(3), [bytes(5), bytes(6)]),
    concat(bytes(3), Uint8Array.of(2, 0, 0, 0), bytes(5), bytes(6)),
  ],
  ["init_migration_record", f.initMigrationRecordIx(key(1), bytes(5)), bytes(5)],
];

describe("the instances' builders", () => {
  const program = new PublicKey(idl.address);

  for (const [build, of, built] of [
    ["production", idl, () => productionBuilders(SplTokenForwarder.at(connection, program))],
    [
      "emergency-migrating",
      emergencyMigratingIdl,
      () => emergencyMigratingBuilders(EmergencyMigratingSplTokenForwarder.at(connection, program)),
    ],
  ] as const) {
    it(`build every instruction a client sends the ${build} build, as its IDL declares it`, () => {
      const names: string[] = [];
      for (const [name, ix, args] of built()) {
        console.log(`${build} ${name}: ${ix.keys.length} accounts, ${ix.data.length} bytes`);
        expect(ix.programId.equals(program), name).toBe(true);
        expectMatchesTheIdl(of, ix, name, args);
        names.push(name);
      }
      expect(names.sort()).toEqual(sentInstructions(of));
    });
  }

  // At another address, every account the program determines is that
  // program's: none is an address the IDL fixes for the program it is built
  // for.
  it("derive every account at the instance's own address", () => {
    const other = Keypair.generate().publicKey;
    const idlProgram = new PublicKey(idl.address);
    const ofTheIdlProgram = new Set(
      [
        idlProgram,
        deriveForwarderConfigPda(idlProgram)[0],
        deriveForwarderEscrowAuthority(idlProgram)[0],
        deriveProgramDataAddress(idlProgram),
        deriveVulnerableLogicRefsPda(idlProgram)[0],
      ].map((k) => k.toBase58()),
    );
    const all = [
      ...productionBuilders(SplTokenForwarder.at(connection, other)),
      ...emergencyMigratingBuilders(EmergencyMigratingSplTokenForwarder.at(connection, other)),
    ];
    for (const [name, ix] of all) {
      expect(ix.programId.equals(other), name).toBe(true);
      const leaked = ix.keys.map((k) => k.pubkey.toBase58()).filter((k) => ofTheIdlProgram.has(k));
      expect(leaked, `${name} names the IDL program's accounts`).toEqual([]);
      if (ix.keys.some((k) => k.pubkey.equals(deriveForwarderConfigPda(other)[0]))) continue;
      expect(["init_nonce_bitmap", "init_migration_record", "version"], `${name} has no config`).toContain(name);
    }
  });

  it("put the upgrade's compute budget first", () => {
    const [budget] = SplTokenForwarder.at(connection, program).upgradeIxs(key(1), key(2), key(3));
    expect(budget).toEqual(ComputeBudgetProgram.setComputeUnitLimit({ units: UPGRADE_COMPUTE_UNIT_LIMIT }));
  });

  it("put the bitmaps to close after the named accounts, writable", () => {
    const ix = SplTokenForwarder.at(connection, program).closeNonceBitmapsBatchIx(key(1), key(2), [key(5), key(6)]);
    expect(ix.keys.slice(3)).toEqual([
      { pubkey: key(5), isSigner: false, isWritable: true },
      { pubkey: key(6), isSigner: false, isWritable: true },
    ]);
  });

  // `@anchor-lang/core` 1.2.1's coder encodes an instruction's arguments into
  // a 1000-byte buffer: the emergency-migrating `reinitialize` fits 30
  // vulnerable logic refs (4 + 32 + 30 * 32 bytes), and refuses more.
  it("encode a reinitialize of up to 30 vulnerable logic refs, and refuse more", () => {
    const f = EmergencyMigratingSplTokenForwarder.at(connection, program);
    const refs = (n: number) => Array.from({ length: n }, (_, i) => bytes(i + 1));
    const thirty = f.reinitializeIx(key(1), key(2), bytes(3), refs(30));
    expect(thirty.data.length).toBe(8 + 32 + 4 + 30 * 32);
    expect(Array.from(thirty.data.subarray(thirty.data.length - 32))).toEqual(Array.from(bytes(30)));
    expect(() => f.reinitializeIx(key(1), key(2), bytes(3), refs(31))).toThrow(RangeError);
  });
});

describe("the emergency-migrating instance", () => {
  it("names the IDL's legacy forwarder", () => {
    const declared = idlConstant(emergencyMigratingIdl, "LEGACY_FORWARDER");
    console.log(`LEGACY_FORWARDER: ${declared}`);
    expect(EmergencyMigratingSplTokenForwarder.at(connection, key(9)).legacyForwarder().toBase58()).toBe(declared);
  });
});

describe("returnedString", () => {
  const program = key(9);
  const returnData = (from: PublicKey, data: Uint8Array) => ({
    programId: from.toBase58(),
    data: [Buffer.from(data).toString("base64"), "base64"] as [string, "base64"],
  });
  // A Borsh String: its length as a little-endian u32, then the UTF-8.
  const encoded = concat(Uint8Array.of(5, 0, 0, 0), new TextEncoder().encode("0.4.0"));

  it("is the string the program returned", () => {
    expect(returnedString(program, returnData(program, encoded))).toBe("0.4.0");
  });

  it("refuses anything but one string the program returned", () => {
    for (const [what, data] of [
      ["nothing", null],
      ["another program's", returnData(key(8), encoded)],
      ["a byte short", returnData(program, encoded.subarray(0, encoded.length - 1))],
      ["a trailing byte", returnData(program, concat(encoded, Uint8Array.of(0)))],
      ["no bytes", returnData(program, new Uint8Array())],
      ["not UTF-8", returnData(program, Uint8Array.of(1, 0, 0, 0, 0xff))],
    ] as const) {
      const error = instanceError(() => returnedString(program, data));
      console.log(`${what}: ${error.message}`);
      expect(error.detail.kind, what).toBe("Decode");
    }
  });
});
