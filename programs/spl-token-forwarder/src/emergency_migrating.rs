//! The emergency-migrating build (feature `emergency-migrating`), the port
//! of the EVM draft `EmergencyMigratingERC20Forwarder`
//! (anoma/anomapay-erc20-forwarder `next`, `contracts/src/draft/`). After
//! someone finds a flaw in a circuit version of the transfer resource, the
//! adapter's owner denies its logic ref, and the forwarder's owner upgrades
//! the forwarder in place to this build and calls its `reinitialize`, which
//! lists the vulnerable logic refs and sets the logic ref of the version the
//! forwarder moves to. A `Migrate` call then lets resources with a listed
//! logic ref move to that version without the adapter consuming them: the
//! forwarder records their nullifiers, and no tokens move
//! (docs/EMERGENCY_MIGRATION.md).

use anchor_lang::prelude::*;
use arm_core::Digest;
use protocol_adapter::nullifier::derive_nullifier_pda;
use protocol_adapter::root::is_root_valid;
use protocol_adapter::state::PAStateAccount;

use crate::{read_pa_state, Config, ErrorCode, ForwardCall, Reinitialize, CONFIG_VERSION};

/// The op byte of a `forward_call` input that migrates resources, after
/// `OP_WRAP` and `OP_UNWRAP`, as the EVM draft's call type `Migrate`.
#[constant]
pub const OP_MIGRATE: u8 = 2;

/// The seed of the vulnerable logic refs' account.
#[constant]
pub const VULNERABLE_LOGIC_REFS_SEED: &[u8] = b"vulnerable_logic_refs";

/// The vulnerable logic refs' address, derived at compile time.
pub const VULNERABLE_LOGIC_REFS_PDA: Pubkey = Pubkey::new_from_array(
    anchor_lang::derive_program_address(&[VULNERABLE_LOGIC_REFS_SEED], &crate::ID_CONST.to_bytes())
        .0,
);

/// The seed of a migrated resource's record, one per nullifier.
#[constant]
pub const MIGRATION_RECORD_SEED: &[u8] = b"migration_record";

/// The legacy SPL token forwarder of the cluster, whose resources can
/// emergency-migrate too because its balances moved to this forwarder, or
/// the zero key on a cluster without one, as the EVM draft's
/// `LEGACY_FORWARDER`. It comes from env/<cluster>.env, as the EVM draft's
/// constructor argument comes from the chain's deployment record.
#[constant]
pub const LEGACY_FORWARDER: Pubkey = Pubkey::from_str_const(env!("LEGACY_FORWARDER_PROGRAM_ID"));

/// The logic refs `reinitialize` listed as vulnerable, as the EVM draft's
/// `_vulnerableLogicRefs`: resources with one of them can emergency-migrate.
/// `reinitialize` creates the account and grows it; nothing removes an
/// entry.
#[account]
pub struct VulnerableLogicRefs {
    pub logic_refs: Vec<[u8; 32]>,
}

impl VulnerableLogicRefs {
    /// The account's size holding `count` logic refs: the discriminator, the
    /// vector's length and the refs.
    pub const fn space(count: usize) -> usize {
        Self::DISCRIMINATOR.len() + 4 + 32 * count
    }
}

/// Whether the resource with a nullifier migrated, as the EVM draft's
/// `_isNullifierMigrated`. The adapter forwards no signer to a forwarder, so
/// a migration cannot create this account: the permissionless
/// `init_migration_record` creates it beforehand, as `init_nonce_bitmap`
/// creates a wrap's bitmap, and a migration sets `migrated`.
///
/// Seeds: ["migration_record", nullifier].
#[account]
#[derive(InitSpace)]
pub struct MigrationRecord {
    pub migrated: bool,
    /// The canonical bump of the record's address, stored by
    /// `init_migration_record`.
    pub bump: u8,
}

impl MigrationRecord {
    /// The account's size: the discriminator and the fields.
    pub const ACCOUNT_SIZE: usize = Self::DISCRIMINATOR.len() + Self::INIT_SPACE;

    /// Whether `key` is the record of `nullifier` under the stored bump.
    /// `init_migration_record` creates records only at the canonical address
    /// and stores its bump, so this recognizes exactly that record.
    pub fn is_at(&self, key: &Pubkey, nullifier: &[u8; 32]) -> bool {
        Pubkey::create_program_address(
            &[MIGRATION_RECORD_SEED, nullifier, &[self.bump]],
            &crate::ID,
        )
        .is_ok_and(|address| address == *key)
    }
}

/// A migrate operand of `forward_call`, after the op byte: the EVM draft's
/// `(IERC20 token, uint128 quantity, MigrateEntry[] entries)` after its call
/// type, Borsh-serialized in field order, the quantity in SPL Token's width.
/// `quantity` is the batch's total, which the trigger's logic checks; the
/// forwarder moves no tokens and reads only the token and the entries.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct MigrateInput {
    pub token_mint: Pubkey,
    pub quantity: u64,
    pub entries: Vec<MigrateEntry>,
}

/// One resource of a migration batch, as the EVM draft's `MigrateEntry`.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct MigrateEntry {
    /// The resource's nullifier.
    pub nullifier: [u8; 32],
    /// The commitment tree root the resource is proven against.
    pub commitment_tree_root: [u8; 32],
    /// The resource's logic ref.
    pub vulnerable_logic_ref: [u8; 32],
    /// The forwarder in the resource's label.
    pub forwarder: Pubkey,
}

/// Mirrors EVM: `event LogicRefReplaced(bytes32 indexed previousLogicRef,
/// bytes32 indexed newLogicRef);`
#[event]
pub struct LogicRefReplaced {
    pub previous_logic_ref: [u8; 32],
    pub new_logic_ref: [u8; 32],
}

/// Mirrors EVM: `event Migrated(address indexed token, bytes32 indexed
/// vulnerableLogicRef, bytes32 indexed nullifier);`. A migration moves no
/// tokens, so an indexer adding up the `Wrapped` and `Unwrapped` amounts must
/// not count it.
#[event]
pub struct Migrated {
    pub token: Pubkey,
    pub vulnerable_logic_ref: [u8; 32],
    pub nullifier: [u8; 32],
}

/// Mirrors EVM: `event VulnerableLogicRefListed(bytes32 indexed logicRef);`
#[event]
pub struct VulnerableLogicRefListed {
    pub logic_ref: [u8; 32],
}

/// Both of the adapter's denylists hold `logic_ref` when `denied`, as the EVM
/// draft's `_checkLogicRefDenied` requires of a vulnerable logic ref (the
/// adapter then neither consumes the resources this forwarder migrates nor
/// creates new ones); neither holds it otherwise, as its
/// `_checkLogicRefNotDenied` requires of the new logic ref.
fn require_denial(state: &PAStateAccount, logic_ref: &[u8; 32], denied: bool) -> Result<()> {
    for consumed in [true, false] {
        if state.is_logic_ref_denied(logic_ref, consumed) != denied {
            msg!(
                "The adapter's denylist for {} resources {} the logic ref {:02x?}",
                if consumed { "consumed" } else { "created" },
                if denied { "lacks" } else { "holds" },
                logic_ref
            );
            return Err(if denied {
                ErrorCode::LogicRefNotDenied
            } else {
                ErrorCode::LogicRefAlreadyDenied
            }
            .into());
        }
    }
    Ok(())
}

/// Whether this forwarder holds the tokens of resources with `forwarder` in
/// their label: its own, and the legacy forwarder `legacy`'s, whose balances
/// moved to it, unless `legacy` is the zero key of a cluster without one.
pub(crate) fn holds_the_tokens_of(forwarder: &Pubkey, legacy: &Pubkey) -> bool {
    *forwarder == crate::ID || (*legacy != Pubkey::default() && forwarder == legacy)
}

/// The listed logic refs: none before `reinitialize` creates the account.
fn listed(account: &AccountInfo) -> Result<Vec<[u8; 32]>> {
    require_keys_eq!(
        account.key(),
        VULNERABLE_LOGIC_REFS_PDA,
        ErrorCode::InvalidVulnerableLogicRefsPda
    );
    if account.data_is_empty() {
        return Ok(Vec::new());
    }
    // Only this program can give its own PDA data.
    Ok(VulnerableLogicRefs::try_deserialize(&mut &account.try_borrow_data()?[..])?.logic_refs)
}

/// Write `listed` to the vulnerable logic refs' account, which the owner
/// pays to create at the first incident and to grow at a later one. The
/// account is created by funding, allocating and assigning it rather than
/// by `create_account`, which a transfer to the address beforehand would
/// block, as Anchor's `init` does.
fn store(ctx: &Context<Reinitialize>, listed: &VulnerableLogicRefs) -> Result<()> {
    let account = ctx.accounts.vulnerable_logic_refs.to_account_info();
    let size = VulnerableLogicRefs::space(listed.logic_refs.len());
    let shortfall = Rent::get()?
        .minimum_balance(size)
        .saturating_sub(account.lamports());
    let system_program = ctx.accounts.system_program.key();
    if shortfall > 0 {
        anchor_lang::system_program::transfer(
            CpiContext::new(
                system_program,
                anchor_lang::system_program::Transfer {
                    from: ctx.accounts.authority.to_account_info(),
                    to: account.clone(),
                },
            ),
            shortfall,
        )?;
    }
    if account.owner == &system_program {
        let signer_seeds: &[&[&[u8]]] = &[&[
            VULNERABLE_LOGIC_REFS_SEED,
            &[ctx.bumps.vulnerable_logic_refs],
        ]];
        anchor_lang::system_program::allocate(
            CpiContext::new_with_signer(
                system_program,
                anchor_lang::system_program::Allocate {
                    account_to_allocate: account.clone(),
                },
                signer_seeds,
            ),
            size as u64,
        )?;
        anchor_lang::system_program::assign(
            CpiContext::new_with_signer(
                system_program,
                anchor_lang::system_program::Assign {
                    account_to_assign: account.clone(),
                },
                signer_seeds,
            ),
            &crate::ID,
        )?;
    } else {
        account.resize(size)?;
    }
    let mut data = account.try_borrow_mut_data()?;
    listed.try_serialize(&mut &mut data[..])
}

/// `reinitialize` of this build, as the EVM draft's
/// `reinitialize(newLogicRef, vulnerableLogicRefs)`: once per build that
/// raises CONFIG_VERSION, the owner lists the vulnerable logic refs, at least
/// one, each on both of the adapter's denylists and not listed already, and
/// sets the forwarder's logic ref to `new_logic_ref`, on neither denylist,
/// or keeps it.
pub(crate) fn reinitialize(
    ctx: Context<Reinitialize>,
    new_logic_ref: [u8; 32],
    vulnerable_logic_refs: Vec<[u8; 32]>,
) -> Result<()> {
    require!(
        ctx.accounts.config.version < CONFIG_VERSION,
        ErrorCode::InvalidInitialization
    );
    require!(
        new_logic_ref != [0u8; 32],
        ErrorCode::ZeroLogicRefNotAllowed
    );
    require!(
        !vulnerable_logic_refs.is_empty(),
        ErrorCode::EmptyVulnerableLogicRefList
    );
    let state = read_pa_state(&ctx.accounts.config, &ctx.accounts.pa_state)?;
    require_denial(&state, &new_logic_ref, false)?;

    // The account holds every listed ref, the earlier incidents' too.
    let account = ctx.accounts.vulnerable_logic_refs.to_account_info();
    let mut listed = VulnerableLogicRefs {
        logic_refs: listed(&account)?,
    };
    for logic_ref in vulnerable_logic_refs {
        require_denial(&state, &logic_ref, true)?;
        if listed.logic_refs.contains(&logic_ref) {
            msg!("The logic ref {:02x?} is listed already", logic_ref);
            return err!(ErrorCode::LogicRefAlreadyVulnerable);
        }
        listed.logic_refs.push(logic_ref);
        emit_cpi!(VulnerableLogicRefListed { logic_ref });
    }
    store(&ctx, &listed)?;

    let config = &mut ctx.accounts.config;
    let previous_logic_ref = config.logic_ref;
    if new_logic_ref != previous_logic_ref {
        config.logic_ref = new_logic_ref;
        emit_cpi!(LogicRefReplaced {
            previous_logic_ref,
            new_logic_ref,
        });
    }
    config.version = CONFIG_VERSION;
    emit_cpi!(crate::Initialized {
        version: CONFIG_VERSION
    });
    Ok(())
}

/// A `Migrate` call, as the EVM draft's `_migrate`: the adapter still denies
/// every listed logic ref, and each entry names this forwarder or the legacy
/// forwarder in its label, a listed logic ref, a root the adapter holds as
/// historical, and a resource the adapter did not consume and this
/// forwarder did not migrate. Each entry's record is written before the next
/// entry is read, so a batch naming a resource twice fails.
///
/// The remaining accounts are the adapter's state, the vulnerable logic
/// refs, then for each entry the adapter's marker of its root, the
/// adapter's marker of its nullifier and its migration record.
pub(crate) fn migrate<'info>(ctx: &Context<'info, ForwardCall<'info>>, input: &[u8]) -> Result<()> {
    let migration =
        MigrateInput::try_from_slice(input).map_err(|_| ErrorCode::InvalidInputLength)?;
    require!(
        !migration.entries.is_empty(),
        ErrorCode::EmptyMigrationBatch
    );
    let [pa_state, vulnerable_logic_refs, entry_accounts @ ..] = ctx.remaining_accounts else {
        msg!(
            "Expected 2 remaining accounts before the migration's entries, got {}",
            ctx.remaining_accounts.len()
        );
        return err!(ErrorCode::InsufficientRemainingAccounts);
    };
    if entry_accounts.len() < 3 * migration.entries.len() {
        msg!(
            "Expected 3 remaining accounts for each of {} entries, got {}",
            migration.entries.len(),
            entry_accounts.len()
        );
        return err!(ErrorCode::InsufficientRemainingAccounts);
    }

    let config: &Config = &ctx.accounts.config;
    let state = read_pa_state(config, pa_state)?;
    let listed = listed(vulnerable_logic_refs)?;
    for logic_ref in &listed {
        require_denial(&state, logic_ref, true)?;
    }

    for (i, entry) in migration.entries.iter().enumerate() {
        let (root_marker, nullifier_marker, record) = (
            &entry_accounts[3 * i],
            &entry_accounts[3 * i + 1],
            &entry_accounts[3 * i + 2],
        );

        if !holds_the_tokens_of(&entry.forwarder, &LEGACY_FORWARDER) {
            msg!(
                "Unknown forwarder {} in the resource's label",
                entry.forwarder
            );
            return err!(ErrorCode::UnknownForwarder);
        }

        if !listed.contains(&entry.vulnerable_logic_ref) {
            msg!(
                "The logic ref {:02x?} is not listed as vulnerable",
                entry.vulnerable_logic_ref
            );
            return err!(ErrorCode::LogicRefNotVulnerable);
        }

        // Every historical root works: the adapter creates no resource with
        // a vulnerable logic ref.
        if !is_root_valid(
            &state,
            &config.protocol_adapter,
            pa_state.key,
            &Digest::from(entry.commitment_tree_root),
            std::slice::from_ref(root_marker),
        ) {
            msg!(
                "The root {:02x?} is not a historical root of the adapter",
                entry.commitment_tree_root
            );
            return err!(ErrorCode::NonExistingRoot);
        }

        let (expected_marker, _) =
            derive_nullifier_pda(&config.protocol_adapter, pa_state.key, &entry.nullifier);
        require_keys_eq!(
            expected_marker,
            *nullifier_marker.key,
            ErrorCode::NullifierPdaMismatch
        );
        if nullifier_marker.owner == &config.protocol_adapter {
            msg!(
                "The adapter consumed the resource with nullifier {:02x?}",
                entry.nullifier
            );
            return err!(ErrorCode::ResourceAlreadyConsumed);
        }

        let mut migration_record = Account::<MigrationRecord>::try_from(record).map_err(|_| {
            msg!(
                "Migration record {} for nullifier {:02x?} does not exist",
                record.key(),
                entry.nullifier
            );
            ErrorCode::MigrationRecordMissing
        })?;
        require!(
            migration_record.is_at(record.key, &entry.nullifier),
            ErrorCode::InvalidMigrationRecordPda
        );
        if migration_record.migrated {
            msg!(
                "The resource with nullifier {:02x?} migrated already",
                entry.nullifier
            );
            return err!(ErrorCode::ResourceAlreadyMigrated);
        }
        migration_record.migrated = true;
        migration_record.exit(&crate::ID)?;

        emit_cpi!(Migrated {
            token: migration.token_mint,
            vulnerable_logic_ref: entry.vulnerable_logic_ref,
            nullifier: entry.nullifier,
        });
    }
    Ok(())
}
