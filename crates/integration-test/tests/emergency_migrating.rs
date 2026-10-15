//! The emergency-migrating build, as the EVM draft
//! `EmergencyMigratingERC20Forwarder`'s unit and upgrade tests check it: the
//! owner upgrades the forwarder to it and its `reinitialize` lists vulnerable
//! logic refs, which the adapter denies, and sets the logic ref; a `Migrate`
//! call records the nullifiers of resources with a listed logic ref, without
//! the adapter consuming them, and moves no tokens.
//!
//! The EVM draft's unit tests call `forwardCall` from a protocol adapter
//! mock. Here the adapter itself makes the calls, during settlements of the
//! pa-testkit's pass-through logic, whose resource makes any external call
//! it is given: `reinitialize` moves the forwarder to that logic's ref, so
//! the forwarder accepts its calls.

use anoma_pa_solana_client::external_call::{OutputMode, SolanaExternalCall};
use anoma_pa_solana_client::settlement::CallAccounts;
use anoma_pa_solana_integration_test::envs::common::environment::Environment;
use anoma_pa_solana_integration_test::envs::local::Environment as LocalEnv;
use anoma_pa_solana_integration_test::executed::Executed;
use anoma_pa_testkit::assert::{Needle, expect_integration_panic};
use anoma_pa_testkit::environment::{DeniedLogicRef, ProtocolAdapter as _, Prover};
use anoma_pa_testkit::fixtures::passthrough::{self, PASSTHROUGH_LOGIC_VK};
use anoma_pa_testkit::transaction::Transaction;
use anoma_pa_testkit::{execute_tx, latest_root, prove_actions};
use anoma_rm_risc0::Digest;
use anoma_rm_risc0::resource::Resource;
use anoma_rm_risc0::utils::bytes_to_words;
use anomapay_spl_token_forwarder_client::emergency_migrating::{
    derive_migration_record_pda, derive_vulnerable_logic_refs_pda, reinitialize_ix,
};
use anomapay_spl_token_forwarder_client::{
    CONFIG_VERSION, ForwarderEvent, LogicRefReplacedEvent, MigrateEntry, MigrateInput,
    MigratedEvent, VulnerableLogicRefListedEvent, decode_migration_record,
    decode_vulnerable_logic_refs, encode_migrate_forwarder_input, forwarder_migrate_num_accounts,
};
use anomapay_spl_token_forwarder_integration_test::fixtures::{self, ShieldedOwner};
use anomapay_spl_token_forwarder_integration_test::logic::{logic_ref, random_logic_ref};
use anomapay_spl_token_forwarder_integration_test::refusal::{NOT_THE_OWNER, balances};
use anomapay_spl_token_forwarder_integration_test::setup::{
    self, FORWARDER_EMERGENCY_MIGRATING_DEV_SO, FORWARDER_EMERGENCY_MIGRATING_SO, OwnedForwarder,
    balance, config, events, set_config_version,
};
use emergency_migrating_transfer_library::{
    EMERGENCY_MIGRATING_TOKEN_TRANSFER_ID, EmergencyMigratingTransferLogic,
};
use emergency_migrating_transfer_witness::MigrateEntry as EmergencyMigrateEntry;
use solana_instruction::Instruction;
use solana_keypair::Keypair;
use solana_signer::Signer;
use spl_associated_token_account_interface::address::get_associated_token_address;
use surfpool_sdk::Pubkey;

/// 100 tokens at the mint's 6 decimals.
const AMOUNT: u64 = 100_000_000;

/// The config version of the emergency-migrating build: one more than the
/// production build's.
const EMERGENCY_MIGRATING_CONFIG_VERSION: u64 = CONFIG_VERSION + 1;

fn bytes(digest: Digest) -> [u8; 32] {
    digest.into()
}

/// The adapter's owner denies `logic_ref` on both denylists.
async fn deny<P>(env: &mut Environment<P>, logic_ref: Digest) -> anyhow::Result<()> {
    deny_on(env, logic_ref, &[true, false]).await
}

/// The adapter's owner denies `logic_ref` on the denylists `consumed` names.
async fn deny_on<P>(
    env: &mut Environment<P>,
    logic_ref: Digest,
    consumed: &[bool],
) -> anyhow::Result<()> {
    let denied: Vec<DeniedLogicRef> = consumed
        .iter()
        .map(|&consumed| DeniedLogicRef {
            logic_ref,
            consumed,
        })
        .collect();
    env.protocol_adapter.deny_logic_refs(&denied).await
}

/// The owner's `reinitialize` of the emergency-migrating build.
fn reinitialize<P>(
    env: &Environment<P>,
    local: &OwnedForwarder,
    new_logic_ref: Digest,
    vulnerable: &[Digest],
) -> Instruction {
    let vulnerable: Vec<[u8; 32]> = vulnerable.iter().copied().map(bytes).collect();
    reinitialize_ix(
        &local.forwarder.program,
        &local.owner.pubkey(),
        &env.protocol_adapter.program,
        bytes(new_logic_ref),
        &vulnerable,
    )
}

/// The logic refs the forwarder lists as vulnerable.
async fn listed(env: &LocalEnv, program: &Pubkey) -> anyhow::Result<Vec<[u8; 32]>> {
    let data = env
        .protocol_adapter
        .rpc
        .get_account_data(&derive_vulnerable_logic_refs_pda(program).0)
        .await?;
    Ok(decode_vulnerable_logic_refs(&data)?)
}

/// Whether the forwarder migrated the resource with `nullifier`, as the EVM
/// draft's `isNullifierMigrated`: its record exists and says so.
async fn is_migrated<P>(
    env: &Environment<P>,
    program: &Pubkey,
    nullifier: [u8; 32],
) -> anyhow::Result<bool> {
    let record = derive_migration_record_pda(program, &nullifier).0;
    let rpc = &env.protocol_adapter.rpc;
    Ok(
        match rpc
            .get_account_with_commitment(&record, rpc.commitment())
            .await?
            .value
        {
            Some(account) => decode_migration_record(&account.data)?.migrated,
            None => false,
        },
    )
}

/// The emergency-migrating build after an incident in the transfer
/// resource's circuit.
struct Incident {
    env: LocalEnv,
    local: OwnedForwarder,
    /// The owner of `wrapped`.
    owner: ShieldedOwner,
    /// A resource with the vulnerable logic ref, which the user wrapped.
    wrapped: Resource,
    /// The root right after the wrap, which holds `wrapped`.
    root: Digest,
}

impl Incident {
    /// The forwarder's own resource `wrapped`, as a migration entry against
    /// `root`.
    fn entry(&self) -> anyhow::Result<MigrateEntry> {
        let nullifier = self
            .wrapped
            .nullifier(&self.owner.keys.nf_key)
            .map_err(|e| anyhow::anyhow!("{e:?}"))?;
        Ok(MigrateEntry {
            nullifier: bytes(nullifier),
            commitment_tree_root: bytes(self.root),
            vulnerable_logic_ref: bytes(logic_ref()),
            forwarder: self.local.forwarder.program.to_bytes(),
        })
    }

    /// A migration of `entries`, carried by a pass-through resource whose
    /// nonces `seed` keeps apart from every other's, proven.
    async fn prove_migration(
        &self,
        entries: Vec<MigrateEntry>,
        seed: u8,
    ) -> anyhow::Result<Transaction> {
        let count = entries.len();
        let input = encode_migrate_forwarder_input(&MigrateInput {
            token_mint: self.local.forwarder.mint.to_bytes(),
            quantity: AMOUNT,
            entries,
        });
        let call = migrate_call(&self.local.forwarder.program, input, count)?;
        prove_actions(&self.env, &[passthrough_call(&call, seed)?]).await
    }

    /// The incident: the adapter's owner denies the transfer logic ref on
    /// both denylists, and the forwarder's owner reinitializes the
    /// emergency-migrating build, listing it and moving to the pass-through
    /// logic.
    async fn deny_and_reinitialize(&mut self) -> anyhow::Result<()> {
        deny(&mut self.env, logic_ref()).await?;
        let ix = reinitialize(&self.env, &self.local, PASSTHROUGH_LOGIC_VK, &[logic_ref()]);
        self.env.send(&[ix], &[&self.local.owner]).await?;
        Ok(())
    }
}

/// The forwarder call of a migration of `entries` resources with `input`;
/// the forwarder returns the empty output.
fn migrate_call(
    program: &Pubkey,
    input: Vec<u8>,
    entries: usize,
) -> anyhow::Result<SolanaExternalCall> {
    Ok(SolanaExternalCall {
        program_id: program.to_bytes(),
        instruction_data: input,
        expected_output: vec![],
        output_mode: OutputMode::ReturnData,
        num_accounts: forwarder_migrate_num_accounts(entries)
            .ok_or_else(|| anyhow::anyhow!("{entries} entries exceed one segment"))?,
    })
}

/// A pass-through action making `call`.
fn passthrough_call(
    call: &SolanaExternalCall,
    seed: u8,
) -> anyhow::Result<anoma_pa_testkit::witness::ActionWitnesses> {
    Ok(passthrough::build(
        seed,
        vec![bytes_to_words(&call.encode())],
        passthrough::Overrides::default(),
    )?
    .witnesses)
}

/// After an incident in the transfer resource's circuit: the user wrapped
/// `AMOUNT` into a resource a shielded owner holds; the adapter's owner
/// denied the transfer logic ref on both denylists; the forwarder's owner
/// upgraded the forwarder to the emergency-migrating build and reinitialized
/// it, listing the transfer logic ref and moving to the pass-through logic.
async fn after_an_incident() -> anyhow::Result<Incident> {
    after_an_incident_on(FORWARDER_EMERGENCY_MIGRATING_SO).await
}

/// `after_an_incident`, with `build` the emergency-migrating build the
/// owner upgrades to.
async fn after_an_incident_on(build: &[u8]) -> anyhow::Result<Incident> {
    let mut incident = before_the_incident(build).await?;
    incident.deny_and_reinitialize().await?;
    Ok(incident)
}

/// The forwarder upgraded to `build` and the user's wrap of `AMOUNT`, before
/// the incident's denial and `reinitialize`.
async fn before_the_incident(build: &[u8]) -> anyhow::Result<Incident> {
    let (mut env, local) = setup::upgraded_to(build).await?;
    let owner = ShieldedOwner::seeded("emergency-migration/owner");
    let wrapped = local
        .forwarder
        .wrap(&mut env, &owner, AMOUNT, 1, "emergency-migration/wrap")
        .await?;
    let root = latest_root(&env).await?;
    Ok(Incident {
        env,
        local,
        owner,
        wrapped,
        root,
    })
}

// ---------------------------------------------------------------------------
// The upgrade and `reinitialize`
// ---------------------------------------------------------------------------

// Mirrors EmergencyMigratingERC20Forwarder.upgrade.t.sol: test_upgrades_safely,
// and the ERC20Forwarder wrap and unwrap tests run through the draft.
#[tokio::test(flavor = "multi_thread")]
async fn the_upgraded_forwarder_wraps_and_unwraps_as_before() -> anyhow::Result<()> {
    let (mut env, local) = setup::upgraded_to(FORWARDER_EMERGENCY_MIGRATING_SO).await?;
    local
        .forwarder
        .wraps_then_unwraps(&mut env, AMOUNT, "upgraded")
        .await
}

// Mirrors test_reinitialize_replaces_the_logic_ref_and_lists_only_the_passed_logic_refs
// and test_reinitialize_emits_the_VulnerableLogicRefListed_and_LogicRefReplaced_events.
#[tokio::test(flavor = "multi_thread")]
async fn reinitialize_replaces_the_logic_ref_lists_the_passed_refs_and_announces_both()
-> anyhow::Result<()> {
    let (mut env, local) = setup::upgraded_to(FORWARDER_EMERGENCY_MIGRATING_SO).await?;
    let program = local.forwarder.program;
    let (first, second, new) = (logic_ref(), random_logic_ref(), random_logic_ref());
    deny(&mut env, first).await?;
    deny(&mut env, second).await?;
    let signature = env
        .send(
            &[reinitialize(&env, &local, new, &[first, second])],
            &[&local.owner],
        )
        .await?;

    let stored = config(&env, &program).await?;
    anyhow::ensure!(
        stored.logic_ref == bytes(new) && stored.version == EMERGENCY_MIGRATING_CONFIG_VERSION,
        "the config is {stored:?}, not at the new logic ref and version {EMERGENCY_MIGRATING_CONFIG_VERSION}"
    );
    let listed = listed(&env, &program).await?;
    anyhow::ensure!(
        listed == [bytes(first), bytes(second)],
        "the forwarder lists {listed:02x?}"
    );
    let emitted = events(
        &Executed::read(&env.protocol_adapter.rpc, &signature).await?,
        &program,
    )?;
    let expected = vec![
        ForwarderEvent::VulnerableLogicRefListed(VulnerableLogicRefListedEvent {
            logic_ref: bytes(first),
        }),
        ForwarderEvent::VulnerableLogicRefListed(VulnerableLogicRefListedEvent {
            logic_ref: bytes(second),
        }),
        ForwarderEvent::LogicRefReplaced(LogicRefReplacedEvent {
            previous_logic_ref: bytes(logic_ref()),
            new_logic_ref: bytes(new),
        }),
        ForwarderEvent::Initialized(anomapay_spl_token_forwarder_client::InitializedEvent {
            version: EMERGENCY_MIGRATING_CONFIG_VERSION,
        }),
    ];
    anyhow::ensure!(
        emitted == expected,
        "reinitialize emits {emitted:?}, not {expected:?}"
    );
    Ok(())
}

// Mirrors test_reinitialize_keeps_the_logic_ref_and_lists_the_vulnerable_logic_refs.
#[tokio::test(flavor = "multi_thread")]
async fn reinitialize_keeps_the_logic_ref_and_lists_the_vulnerable_refs() -> anyhow::Result<()> {
    let (mut env, local) = setup::upgraded_to(FORWARDER_EMERGENCY_MIGRATING_SO).await?;
    let program = local.forwarder.program;
    let deprecated = random_logic_ref();
    deny(&mut env, deprecated).await?;
    let signature = env
        .send(
            &[reinitialize(&env, &local, logic_ref(), &[deprecated])],
            &[&local.owner],
        )
        .await?;
    anyhow::ensure!(config(&env, &program).await?.logic_ref == bytes(logic_ref()));
    anyhow::ensure!(listed(&env, &program).await? == [bytes(deprecated)]);
    let emitted = events(
        &Executed::read(&env.protocol_adapter.rpc, &signature).await?,
        &program,
    )?;
    anyhow::ensure!(
        !emitted
            .iter()
            .any(|event| matches!(event, ForwarderEvent::LogicRefReplaced(_))),
        "reinitialize announces a replacement of a kept logic ref: {emitted:?}"
    );
    Ok(())
}

/// Reinitializing with `ix`, signed by `signer`, fails with `error`, and
/// the config keeps the production build's version.
async fn refuses_reinitialize(
    env: &LocalEnv,
    local: &OwnedForwarder,
    ix: Instruction,
    signer: &Keypair,
    error: &'static str,
) -> anyhow::Result<()> {
    expect_integration_panic(Needle::Static(error))(env.send(&[ix], &[signer]).await)?;
    let version = config(env, &local.forwarder.program).await?.version;
    anyhow::ensure!(
        version == EMERGENCY_MIGRATING_CONFIG_VERSION - 1,
        "a refused reinitialize moved the config to version {version}"
    );
    Ok(())
}

// Mirrors test_reinitialize_reverts_if_the_caller_is_not_the_owner.
#[tokio::test(flavor = "multi_thread")]
async fn reinitialize_refuses_anyone_but_the_owner() -> anyhow::Result<()> {
    let (mut env, local) = setup::upgraded_to(FORWARDER_EMERGENCY_MIGRATING_SO).await?;
    deny(&mut env, logic_ref()).await?;
    let stranger = Keypair::new();
    setup::give_sol(&env, &stranger.pubkey(), 1_000_000_000)?;
    let ix = reinitialize_ix(
        &local.forwarder.program,
        &stranger.pubkey(),
        &env.protocol_adapter.program,
        bytes(random_logic_ref()),
        &[bytes(logic_ref())],
    );
    refuses_reinitialize(&env, &local, ix, &stranger, NOT_THE_OWNER).await
}

// Mirrors test_reinitialize_reverts_if_it_runs_a_second_time.
#[tokio::test(flavor = "multi_thread")]
async fn reinitialize_runs_once() -> anyhow::Result<()> {
    let incident = after_an_incident().await?;
    let (env, local) = (&incident.env, &incident.local);
    expect_integration_panic(Needle::Static("Error Code: InvalidInitialization."))(
        env.send(
            &[reinitialize(env, local, random_logic_ref(), &[logic_ref()])],
            &[&local.owner],
        )
        .await,
    )
}

// Mirrors test_reinitialize_keeps_earlier_vulnerable_logic_refs_migratable: a
// later incident's implementation lists another ref, here the development
// build's `reinitialize` run again once `dev_set_config_version` lowers the
// version. The first incident's ref stays listed, and its resources still
// migrate.
#[tokio::test(flavor = "multi_thread")]
async fn a_later_incident_keeps_the_earlier_refs_listed_and_migratable() -> anyhow::Result<()> {
    let mut incident = after_an_incident_on(FORWARDER_EMERGENCY_MIGRATING_DEV_SO).await?;
    let program = incident.local.forwarder.program;
    let later = random_logic_ref();
    deny(&mut incident.env, later).await?;
    set_config_version(
        &incident.env,
        &incident.local,
        EMERGENCY_MIGRATING_CONFIG_VERSION - 1,
    )
    .await?;
    let (env, local) = (&incident.env, &incident.local);
    expect_integration_panic(Needle::Static("Error Code: LogicRefAlreadyVulnerable."))(
        env.send(
            &[reinitialize(
                env,
                local,
                PASSTHROUGH_LOGIC_VK,
                &[logic_ref()],
            )],
            &[&local.owner],
        )
        .await,
    )?;
    env.send(
        &[reinitialize(env, local, PASSTHROUGH_LOGIC_VK, &[later])],
        &[&local.owner],
    )
    .await?;
    let refs = listed(env, &program).await?;
    anyhow::ensure!(
        refs == [bytes(logic_ref()), bytes(later)],
        "the forwarder lists {refs:02x?}"
    );

    let entry = incident.entry()?;
    let tx = incident.prove_migration(vec![entry.clone()], 1).await?;
    execute_tx(&mut incident.env, tx).await?;
    anyhow::ensure!(is_migrated(&incident.env, &program, entry.nullifier).await?);
    Ok(())
}

// Mirrors test_reinitialize_reverts_on_the_zero_logic_ref.
#[tokio::test(flavor = "multi_thread")]
async fn reinitialize_refuses_the_zero_logic_ref() -> anyhow::Result<()> {
    let (mut env, local) = setup::upgraded_to(FORWARDER_EMERGENCY_MIGRATING_SO).await?;
    deny(&mut env, logic_ref()).await?;
    let ix = reinitialize(&env, &local, Digest::default(), &[logic_ref()]);
    refuses_reinitialize(
        &env,
        &local,
        ix,
        &local.owner,
        "Error Code: ZeroLogicRefNotAllowed.",
    )
    .await
}

// Mirrors test_reinitialize_reverts_on_an_empty_list.
#[tokio::test(flavor = "multi_thread")]
async fn reinitialize_refuses_an_empty_list() -> anyhow::Result<()> {
    let (env, local) = setup::upgraded_to(FORWARDER_EMERGENCY_MIGRATING_SO).await?;
    let ix = reinitialize(&env, &local, random_logic_ref(), &[]);
    refuses_reinitialize(
        &env,
        &local,
        ix,
        &local.owner,
        "Error Code: EmptyVulnerableLogicRefList.",
    )
    .await
}

// Mirrors test_reinitialize_reverts_if_the_logic_ref_is_vulnerable_already.
#[tokio::test(flavor = "multi_thread")]
async fn reinitialize_refuses_a_ref_listed_twice() -> anyhow::Result<()> {
    let (mut env, local) = setup::upgraded_to(FORWARDER_EMERGENCY_MIGRATING_SO).await?;
    deny(&mut env, logic_ref()).await?;
    let ix = reinitialize(
        &env,
        &local,
        random_logic_ref(),
        &[logic_ref(), logic_ref()],
    );
    refuses_reinitialize(
        &env,
        &local,
        ix,
        &local.owner,
        "Error Code: LogicRefAlreadyVulnerable.",
    )
    .await
}

// Mirrors testFuzz_reinitialize_reverts_if_the_protocol_adapter_does_not_deny_the_vulnerable_logic_ref,
// once per denylist the ref is missing from.
#[tokio::test(flavor = "multi_thread")]
async fn reinitialize_refuses_a_ref_either_denylist_lacks() -> anyhow::Result<()> {
    let (mut env, local) = setup::upgraded_to(FORWARDER_EMERGENCY_MIGRATING_SO).await?;
    for consumed in [true, false] {
        // Denied on the other list only.
        let vulnerable = random_logic_ref();
        deny_on(&mut env, vulnerable, &[!consumed]).await?;
        let ix = reinitialize(&env, &local, random_logic_ref(), &[vulnerable]);
        refuses_reinitialize(
            &env,
            &local,
            ix,
            &local.owner,
            "Error Code: LogicRefNotDenied.",
        )
        .await?;
    }
    Ok(())
}

// Mirrors testFuzz_reinitialize_reverts_if_the_protocol_adapter_denies_the_new_logic_ref,
// once per denylist that holds it.
#[tokio::test(flavor = "multi_thread")]
async fn reinitialize_refuses_a_new_ref_either_denylist_holds() -> anyhow::Result<()> {
    let (mut env, local) = setup::upgraded_to(FORWARDER_EMERGENCY_MIGRATING_SO).await?;
    deny(&mut env, logic_ref()).await?;
    for consumed in [true, false] {
        let new = random_logic_ref();
        deny_on(&mut env, new, &[consumed]).await?;
        let ix = reinitialize(&env, &local, new, &[logic_ref()]);
        refuses_reinitialize(
            &env,
            &local,
            ix,
            &local.owner,
            "Error Code: LogicRefAlreadyDenied.",
        )
        .await?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// `Migrate`
// ---------------------------------------------------------------------------

// Mirrors test_migrate_records_every_nullifier_of_a_batch,
// test_migrate_emits_the_Migrated_event and the integration test
// test_execute_emergency_migrates_a_resource_after_the_protocol_adapter_denies_its_logic_ref:
// the forwarder records each nullifier and announces it, and moves no tokens.
#[tokio::test(flavor = "multi_thread")]
async fn a_migration_records_every_nullifier_and_moves_no_tokens() -> anyhow::Result<()> {
    let mut incident = after_an_incident().await?;
    let program = incident.local.forwarder.program;
    let own = incident.entry()?;
    // A resource the adapter never consumed, under another historical root:
    // the forwarder checks the nullifier and the root, not the resource.
    let other = MigrateEntry {
        nullifier: [7; 32],
        commitment_tree_root: bytes(latest_root(&incident.env).await?),
        ..own.clone()
    };
    let unmoved = [
        incident.local.forwarder.user_account(),
        incident.local.forwarder.escrow_account(),
    ];
    let before = balances(&incident.env, &unmoved).await?;

    let tx = incident
        .prove_migration(vec![own.clone(), other.clone()], 1)
        .await?;
    let executed = incident.env.protocol_adapter.settled(tx).await?;

    let emitted = events(&executed, &program)?;
    let expected: Vec<ForwarderEvent> = [&own, &other]
        .into_iter()
        .map(|entry| {
            ForwarderEvent::Migrated(MigratedEvent {
                token: incident.local.forwarder.mint.to_bytes(),
                vulnerable_logic_ref: entry.vulnerable_logic_ref,
                nullifier: entry.nullifier,
            })
        })
        .collect();
    anyhow::ensure!(
        emitted == expected,
        "the migration emits {emitted:?}, not {expected:?}"
    );
    for entry in [&own, &other] {
        anyhow::ensure!(
            is_migrated(&incident.env, &program, entry.nullifier).await?,
            "the forwarder did not record {:02x?}",
            entry.nullifier
        );
    }
    anyhow::ensure!(
        balances(&incident.env, &unmoved).await? == before,
        "a migration moved tokens"
    );
    Ok(())
}

/// Settling a migration of `entries` fails with `error`, and records no
/// nullifier.
async fn refuses_migration(
    incident: &mut Incident,
    entries: Vec<MigrateEntry>,
    seed: u8,
    error: &'static str,
) -> anyhow::Result<()> {
    let tx = incident.prove_migration(entries.clone(), seed).await?;
    expect_integration_panic(Needle::Static(error))(execute_tx(&mut incident.env, tx).await)?;
    for entry in &entries {
        anyhow::ensure!(
            !is_migrated(
                &incident.env,
                &incident.local.forwarder.program,
                entry.nullifier
            )
            .await?,
            "a refused migration recorded {:02x?}",
            entry.nullifier
        );
    }
    Ok(())
}

// Mirrors test_migrate_reverts_if_the_resource_was_migrated_already.
#[tokio::test(flavor = "multi_thread")]
async fn a_migration_refuses_a_resource_migrated_already() -> anyhow::Result<()> {
    let mut incident = after_an_incident().await?;
    let entry = incident.entry()?;
    let tx = incident.prove_migration(vec![entry.clone()], 1).await?;
    execute_tx(&mut incident.env, tx).await?;
    let tx = incident.prove_migration(vec![entry], 2).await?;
    expect_integration_panic(Needle::Static("Error Code: ResourceAlreadyMigrated."))(
        execute_tx(&mut incident.env, tx).await,
    )
}

// Mirrors test_migrate_reverts_if_a_batch_names_a_resource_twice.
#[tokio::test(flavor = "multi_thread")]
async fn a_migration_refuses_a_batch_naming_a_resource_twice() -> anyhow::Result<()> {
    let mut incident = after_an_incident().await?;
    let entry = incident.entry()?;
    refuses_migration(
        &mut incident,
        vec![entry.clone(), entry],
        1,
        "Error Code: ResourceAlreadyMigrated.",
    )
    .await
}

// Mirrors test_migrate_reverts_if_the_protocol_adapter_consumed_the_resource:
// the user unwrapped a resource before the denial, so the adapter's
// nullifier set holds it.
#[tokio::test(flavor = "multi_thread")]
async fn a_migration_refuses_a_resource_the_adapter_consumed() -> anyhow::Result<()> {
    let mut incident = before_the_incident(FORWARDER_EMERGENCY_MIGRATING_SO).await?;
    let tx = incident
        .local
        .forwarder
        .prove_unwrap(
            &incident.env,
            incident.wrapped,
            &incident.owner,
            Keypair::new().pubkey(),
        )
        .await?;
    execute_tx(&mut incident.env, tx).await?;
    incident.deny_and_reinitialize().await?;
    let entry = incident.entry()?;
    refuses_migration(
        &mut incident,
        vec![entry],
        1,
        "Error Code: ResourceAlreadyConsumed.",
    )
    .await
}

// Mirrors test_migrate_reverts_on_a_logic_ref_that_is_not_vulnerable: the
// denied ref of another application, which the forwarder does not list.
#[tokio::test(flavor = "multi_thread")]
async fn a_migration_refuses_a_logic_ref_the_forwarder_does_not_list() -> anyhow::Result<()> {
    let mut incident = after_an_incident().await?;
    let unlisted = random_logic_ref();
    deny(&mut incident.env, unlisted).await?;
    let entry = MigrateEntry {
        vulnerable_logic_ref: bytes(unlisted),
        ..incident.entry()?
    };
    refuses_migration(
        &mut incident,
        vec![entry],
        1,
        "Error Code: LogicRefNotVulnerable.",
    )
    .await
}

// Mirrors test_migrate_accepts_any_historical_root: the adapter's first
// root, the empty tree's (the padding leaf), which holds no resource but is
// historical; then the wrap's root once a later settlement has replaced it.
#[tokio::test(flavor = "multi_thread")]
async fn a_migration_accepts_any_historical_root() -> anyhow::Result<()> {
    let mut incident = after_an_incident().await?;
    let program = incident.local.forwarder.program;
    let first = MigrateEntry {
        nullifier: [8; 32],
        commitment_tree_root: bytes(anoma_rm_risc0::merkle_path::PADDING_LEAF),
        ..incident.entry()?
    };
    let tx = incident.prove_migration(vec![first.clone()], 1).await?;
    execute_tx(&mut incident.env, tx).await?;
    anyhow::ensure!(is_migrated(&incident.env, &program, first.nullifier).await?);

    // The migration's settlement added its commitments: the wrap's root is
    // no longer the latest.
    anyhow::ensure!(latest_root(&incident.env).await? != incident.root);
    let own = incident.entry()?;
    let tx = incident.prove_migration(vec![own.clone()], 2).await?;
    execute_tx(&mut incident.env, tx).await?;
    anyhow::ensure!(is_migrated(&incident.env, &program, own.nullifier).await?);
    Ok(())
}

// Mirrors test_migrate_reverts_on_a_root_that_is_not_historical.
#[tokio::test(flavor = "multi_thread")]
async fn a_migration_refuses_a_root_that_is_not_historical() -> anyhow::Result<()> {
    let mut incident = after_an_incident().await?;
    let entry = MigrateEntry {
        commitment_tree_root: [9; 32],
        ..incident.entry()?
    };
    refuses_migration(
        &mut incident,
        vec![entry],
        1,
        "Error Code: NonExistingRoot.",
    )
    .await
}

// Mirrors test_migrate_reverts_on_another_forwarder.
#[tokio::test(flavor = "multi_thread")]
async fn a_migration_refuses_another_forwarder() -> anyhow::Result<()> {
    let mut incident = after_an_incident().await?;
    let entry = MigrateEntry {
        forwarder: Keypair::new().pubkey().to_bytes(),
        ..incident.entry()?
    };
    refuses_migration(
        &mut incident,
        vec![entry],
        1,
        "Error Code: UnknownForwarder.",
    )
    .await
}

// Mirrors test_migrate_reverts_on_the_zero_forwarder_on_a_chain_without_a_legacy_forwarder:
// env/localnet.env names no legacy forwarder.
#[tokio::test(flavor = "multi_thread")]
async fn a_migration_refuses_the_zero_forwarder_on_a_cluster_without_a_legacy_forwarder()
-> anyhow::Result<()> {
    let mut incident = after_an_incident().await?;
    let entry = MigrateEntry {
        forwarder: [0; 32],
        ..incident.entry()?
    };
    refuses_migration(
        &mut incident,
        vec![entry],
        1,
        "Error Code: UnknownForwarder.",
    )
    .await
}

// Mirrors test_migrate_reverts_on_an_empty_batch.
#[tokio::test(flavor = "multi_thread")]
async fn a_migration_refuses_an_empty_batch() -> anyhow::Result<()> {
    let mut incident = after_an_incident().await?;
    refuses_migration(&mut incident, vec![], 1, "Error Code: EmptyMigrationBatch.").await
}

/// A submitter passing the forwarder `accounts` for every call, whatever
/// the call: what a migration whose input the client cannot decode needs.
struct Fixed(CallAccounts);

impl anoma_pa_solana_client::forwarders::Forwarder for Fixed {
    fn call_accounts<'a>(
        &'a self,
        _rpc: &'a solana_rpc_client::nonblocking::rpc_client::RpcClient,
        _calls: &'a [SolanaExternalCall],
        _preceding_start: usize,
    ) -> anoma_pa_solana_client::forwarders::CallAccountsFuture<'a> {
        let accounts = self.0.clone();
        Box::pin(async move { Ok(accounts) })
    }
}

// Mirrors test_migrate_reverts_on_trailing_bytes: a byte past the last entry.
#[tokio::test(flavor = "multi_thread")]
async fn a_migration_refuses_trailing_bytes() -> anyhow::Result<()> {
    let mut incident = after_an_incident().await?;
    let program = incident.local.forwarder.program;
    let entry = incident.entry()?;
    let migration = MigrateInput {
        token_mint: incident.local.forwarder.mint.to_bytes(),
        quantity: AMOUNT,
        entries: vec![entry.clone()],
    };
    // The client supplies the accounts of the well-formed migration.
    let accounts = anomapay_spl_token_forwarder_client::settlement::settlement_calls(
        &program,
        &incident.env.protocol_adapter.program,
        &incident.env.protocol_adapter.payer.pubkey(),
        &[migrate_call(
            &program,
            encode_migrate_forwarder_input(&migration),
            1,
        )?],
        0,
    )?;
    incident
        .env
        .protocol_adapter
        .forwarders
        .register(program, std::sync::Arc::new(Fixed(accounts)));
    let mut input = encode_migrate_forwarder_input(&migration);
    input.push(0);
    let call = migrate_call(&program, input, 1)?;
    let tx = prove_actions(&incident.env, &[passthrough_call(&call, 1)?]).await?;
    expect_integration_panic(Needle::Static("Error Code: InvalidInputLength."))(
        execute_tx(&mut incident.env, tx).await,
    )?;
    anyhow::ensure!(!is_migrated(&incident.env, &program, entry.nullifier).await?);
    Ok(())
}

// The adapter's CPI carries no signer, so a migration needs each entry's
// record created beforehand, as a wrap needs its nonce bitmap.
#[tokio::test(flavor = "multi_thread")]
async fn a_migration_refuses_an_entry_without_its_record() -> anyhow::Result<()> {
    let mut incident = after_an_incident().await?;
    let entry = incident.entry()?;
    let tx = incident.prove_migration(vec![entry], 1).await?;
    incident
        .local
        .forwarder
        .rewrite(&mut incident.env, |accounts| accounts.preceding.clear());
    expect_integration_panic(Needle::Static("Error Code: MigrationRecordMissing."))(
        execute_tx(&mut incident.env, tx).await,
    )
}

// The adapter's marker of a nullifier must be at its address: any other
// account would read as an unconsumed resource.
#[tokio::test(flavor = "multi_thread")]
async fn a_migration_refuses_another_account_as_the_adapters_nullifier_marker() -> anyhow::Result<()>
{
    let mut incident = after_an_incident().await?;
    let entry = incident.entry()?;
    let tx = incident.prove_migration(vec![entry], 1).await?;
    // The accounts before the entries, then the entry's root marker,
    // nullifier marker and record.
    incident
        .local
        .forwarder
        .rewrite(&mut incident.env, |accounts| {
            let marker = usize::from(forwarder_migrate_num_accounts(0).unwrap()) + 1;
            accounts.segments[0][marker].pubkey = Keypair::new().pubkey();
        });
    expect_integration_panic(Needle::Static("Error Code: NullifierPdaMismatch."))(
        execute_tx(&mut incident.env, tx).await,
    )
}

// The production build has no `Migrate` call, as the EVM `ERC20Forwarder`'s
// call type has no `Migrate`.
#[tokio::test(flavor = "multi_thread")]
async fn the_production_build_refuses_a_migration() -> anyhow::Result<()> {
    // The development build, whose config the owner lowers below this
    // build's version, so its `reinitialize` moves it to the pass-through
    // logic.
    let (mut env, local) = setup::local_below_this_builds_version().await?;
    env.send(
        &[anomapay_spl_token_forwarder_client::reinitialize_ix(
            &local.forwarder.program,
            &local.owner.pubkey(),
            bytes(PASSTHROUGH_LOGIC_VK),
        )],
        &[&local.owner],
    )
    .await?;
    let input = encode_migrate_forwarder_input(&MigrateInput {
        token_mint: local.forwarder.mint.to_bytes(),
        quantity: AMOUNT,
        entries: vec![MigrateEntry {
            nullifier: [1; 32],
            commitment_tree_root: bytes(latest_root(&env).await?),
            vulnerable_logic_ref: bytes(logic_ref()),
            forwarder: local.forwarder.program.to_bytes(),
        }],
    });
    let call = migrate_call(&local.forwarder.program, input, 1)?;
    let tx = prove_actions(&env, &[passthrough_call(&call, 1)?]).await?;
    // The client creates the entry's record with an instruction the
    // production build lacks.
    local
        .forwarder
        .rewrite(&mut env, |accounts| accounts.preceding.clear());
    expect_integration_panic(Needle::Static("Error Code: UnknownOperation."))(
        execute_tx(&mut env, tx).await,
    )
}

// ---------------------------------------------------------------------------
// A migration proven by the emergency-migrating logic
// ---------------------------------------------------------------------------

/// The incident end to end with the AnomaPay resource, as the EVM
/// integration test test_execute_emergency_migrates_a_resource_after_the_protocol_adapter_denies_its_logic_ref
/// runs it with the ERC20 resource: a holder wraps `AMOUNT` under the
/// transfer logic; the adapter's owner denies it; the forwarder's owner
/// upgrades to the emergency-migrating build and reinitializes it, listing
/// the transfer logic and moving to the emergency-migrating logic. The
/// holder's migration, which that logic proves, records the wrapped
/// resource's nullifier and moves no tokens, and its resource then unwraps
/// through that logic, releasing the escrowed tokens to a recipient.
async fn a_migration_recovers_the_wrapped_tokens<P: Prover>(
    env: &mut Environment<P>,
    owned: OwnedForwarder,
) -> anyhow::Result<()> {
    let holder = ShieldedOwner::seeded("real-migration/holder");
    let forwarder = &owned.forwarder;
    let program = forwarder.program;
    let escrow = forwarder.escrow_account();
    let escrow_before = balance(env, &escrow).await?;

    let wrapped = forwarder
        .wrap(env, &holder, AMOUNT, 1, "real-migration/wrap")
        .await?;
    let path = env
        .protocol_adapter
        .commitment_tree
        .path_to(wrapped.commitment())?;

    deny(env, logic_ref()).await?;
    setup::upgrade(
        env,
        &program,
        &owned.owner,
        FORWARDER_EMERGENCY_MIGRATING_SO,
    )
    .await?;
    let ix = reinitialize(
        env,
        &owned,
        EMERGENCY_MIGRATING_TOKEN_TRANSFER_ID,
        &[logic_ref()],
    );
    env.send(&[ix], &[&owned.owner]).await?;
    let forwarder = owned.forwarder.serving::<EmergencyMigratingTransferLogic>();

    let migration = fixtures::migrate(
        program,
        forwarder.mint,
        vec![EmergencyMigrateEntry {
            resource: wrapped,
            nf_key: holder.keys.nf_key.clone(),
            path,
            value_info: holder.keys.value.clone(),
            forwarder_program_id: program.to_bytes(),
        }],
        &holder,
        &holder,
        "real-migration/migrate",
    )?;
    let tx = prove_actions(env, &[migration.witnesses]).await?;
    let executed = env.protocol_adapter.settled(tx).await?;
    let nullifier = bytes(
        wrapped
            .nullifier(&holder.keys.nf_key)
            .map_err(|e| anyhow::anyhow!("{e:?}"))?,
    );
    let expected = vec![ForwarderEvent::Migrated(MigratedEvent {
        token: forwarder.mint.to_bytes(),
        vulnerable_logic_ref: bytes(logic_ref()),
        nullifier,
    })];
    let emitted = events(&executed, &program)?;
    anyhow::ensure!(
        emitted == expected,
        "the migration emits {emitted:?}, not {expected:?}"
    );
    anyhow::ensure!(
        is_migrated(env, &program, nullifier).await?,
        "the forwarder did not record the wrapped resource's nullifier"
    );
    anyhow::ensure!(
        balance(env, &escrow).await? == escrow_before + AMOUNT,
        "the migration moved escrowed tokens"
    );

    let recipient = Keypair::new().pubkey();
    let tx = forwarder
        .prove_unwrap(env, migration.created, &holder, recipient)
        .await?;
    execute_tx(env, tx).await?;
    let recipient_account = get_associated_token_address(&recipient, &forwarder.mint);
    anyhow::ensure!(
        balance(env, &recipient_account).await? == AMOUNT,
        "the unwrap of the migrated resource did not release {AMOUNT} to the recipient"
    );
    anyhow::ensure!(
        balance(env, &escrow).await? == escrow_before,
        "the escrow holds {} after the unwrap, not {escrow_before}",
        balance(env, &escrow).await?
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn local_a_migration_recovers_the_wrapped_tokens() -> anyhow::Result<()> {
    let (mut env, owned) = setup::local().await?;
    a_migration_recovers_the_wrapped_tokens(&mut env, owned).await
}

#[cfg(feature = "e2e")]
#[tokio::test(flavor = "multi_thread")]
async fn e2e_test_a_migration_recovers_the_wrapped_tokens() -> anyhow::Result<()> {
    let (mut env, owned) = setup::e2e_owned().await?;
    a_migration_recovers_the_wrapped_tokens(&mut env, owned).await
}
