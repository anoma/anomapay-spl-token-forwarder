/**
 * SPL token forwarder operations. Run through ops.sh, which sets the
 * cluster, the wallet, and the adapter's address (env/<cluster>.env):
 *
 *   ./scripts/dev.sh forwarder <command> --cluster <c> [--wallet <path>]
 *
 * Commands (the wallet that must sign is in parentheses):
 *   init                  (upgrade    Initialize the config, owned by
 *                          authority) STF_OWNER, which hands the upgrade
 *                                     authority to the program, and, with
 *                                     STF_TOKEN_MINT, the mint's escrow ATA.
 *                                     Idempotent; refuses an existing config
 *                                     that differs from the request.
 *   reinitialize          (owner)     After upgrading the program to a
 *                                     build that raises CONFIG_VERSION:
 *                                     rotate the config's logic ref to
 *                                     STF_LOGIC_REF, once; escrow and nonce
 *                                     bitmaps are untouched.
 *
 * Closing escrows, nonce bitmaps or the config are not commands: on a live
 * cluster they are done by hand (docs/OPERATIONS.md).
 *
 * The program enforces who may do what and when; a refused command fails
 * with the program's error (OwnableUnauthorizedAccount, InvalidInitialization,
 * ...).
 *
 * Environment (read per command):
 *   STF_LOGIC_REF           32-byte hex: the resource logic the config
 *                           authorizes (init, reinitialize)
 *   STF_OWNER               base58 pubkey of the initial owner (init)
 *   STF_TOKEN_MINT          base58 mint (init, optional)
 */
import * as anchor from "@anchor-lang/core";
import { Program } from "@anchor-lang/core";
import { getCreateAssociatedTokenIdempotentInstructionAsync } from "@solana-program/token";
import { appendTransactionMessageInstruction, createTransactionMessage, pipe, type Instruction } from "@solana/kit";
import { confirmedProvider } from "../client/provider";
import type { SplTokenForwarder as SplTokenForwarderIdl } from "../target/types/spl_token_forwarder";
import { SplTokenForwarder } from "../ts/src/instance";
import { deriveForwarderConfigPda, deriveForwarderEscrowAuthority, deriveTokenAccount } from "../ts/src/pda";
import { fail, requireEnv, requireHexBytes, requirePubkey } from "./cli-utils";

const provider = confirmedProvider(requireEnv("ANCHOR_PROVIDER_URL", "the RPC endpoint"), process.env.ANCHOR_WS_URL);
anchor.setProvider(provider);
const wallet = provider.wallet;
// This cluster's build (target/idl): its config read through the IDL's types,
// its instructions built by the bindings' instance.
const forwarder = anchor.workspace.SplTokenForwarder as Program<SplTokenForwarderIdl>;
const instance = SplTokenForwarder.at(provider.rpc, forwarder.address);
// The adapter this build of the forwarder pairs with (env/<cluster>.env).
const adapterProgramId = requirePubkey("PROTOCOL_ADAPTER_PROGRAM_ID", "the protocol adapter the forwarder serves");

const requireMint = () => requirePubkey("STF_TOKEN_MINT", "the mint whose escrow to operate on");

/** Sends `ix`, which the wallet signs and pays for. */
async function send(ix: Instruction) {
  await provider.sendAndConfirm(
    pipe(createTransactionMessage({ version: 0 }), (m) => appendTransactionMessageInstruction(ix, m)),
  );
}

async function configPda() {
  return (await deriveForwarderConfigPda(forwarder.address))[0];
}

async function requireConfig() {
  const config = await forwarder.account.config.fetchNullable(await configPda());
  return config.exists
    ? config.data
    : fail(`forwarder config ${config.address} does not exist — run 'forwarder init' first`);
}

async function init() {
  const logicRef = requireHexBytes(
    "STF_LOGIC_REF",
    32,
    "the 32-byte hex logic ref (verifying key) of the resource logic this forwarder serves",
  );
  const owner = requirePubkey(
    "STF_OWNER",
    "the forwarder's initial owner, who alone upgrades it and rotates its logic ref",
  );

  const config = await configPda();
  const fetched = await forwarder.account.config.fetchNullable(config);
  if (fetched.exists) {
    const existing = fetched.data;
    // (field, stored, requested)
    const fields: [string, string, string][] = [
      ["adapter", existing.protocolAdapter, adapterProgramId],
      ["logic ref", Buffer.from(existing.logicRef).toString("hex"), Buffer.from(logicRef).toString("hex")],
      ["owner", existing.owner, owner],
    ];
    const mismatches = fields.filter(([, stored, requested]) => stored !== requested);
    if (mismatches.length > 0) {
      fail(
        `config ${config} already exists with a different ${mismatches.map(([k]) => k).join(", ")}: ` +
          mismatches.map(([k, stored, requested]) => `${k} ${stored} (requested ${requested})`).join("; "),
      );
    }
    console.log(`Config ${config} already initialized with the requested values`);
  } else {
    console.log(`Initializing config ${config}`);
    console.log(`  adapter:   ${adapterProgramId}`);
    console.log(`  logic ref: ${Buffer.from(logicRef).toString("hex")}`);
    console.log(`  owner:     ${owner}`);
    await send(await instance.initializeIx(wallet.address, adapterProgramId, logicRef, owner));
    console.log("✅ Config initialized");
  }

  if (process.env.STF_TOKEN_MINT) {
    const mint = requireMint();
    const [escrowAuthority] = await deriveForwarderEscrowAuthority(forwarder.address);
    const create = await getCreateAssociatedTokenIdempotentInstructionAsync({
      payer: wallet,
      owner: escrowAuthority,
      mint,
    });
    await send(create);
    const escrowAta = await deriveTokenAccount(escrowAuthority, mint);
    console.log(`✅ Escrow for ${mint}: authority ${escrowAuthority}, ATA ${escrowAta}`);
  }
}

async function reinitializeCommand() {
  const logicRef = requireHexBytes(
    "STF_LOGIC_REF",
    32,
    "the 32-byte hex logic ref (verifying key) the config should authorize from now on",
  );
  const existing = await requireConfig();
  const previous = Buffer.from(existing.logicRef).toString("hex");
  await send(await instance.reinitializeIx(wallet.address, logicRef));
  const config = await requireConfig();
  console.log(
    `✅ Logic ref rotated: ${previous} -> ${Buffer.from(config.logicRef).toString("hex")} (config version ${config.version})`,
  );
}

const COMMANDS: Record<string, () => Promise<void>> = {
  init,
  reinitialize: reinitializeCommand,
};

const command = process.argv[2];
const run = command ? COMMANDS[command] : undefined;
if (!run) {
  fail(`unknown forwarder command "${command ?? ""}"; expected one of: ${Object.keys(COMMANDS).join(", ")}`);
}

console.log(`Forwarder: ${forwarder.address}  Wallet: ${wallet.address}`);
run().catch((err) => fail(`forwarder ${command} failed: ${err.message ?? err}`));
