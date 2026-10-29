//! AnomaPay wrap, unwrap and migration actions, built with
//! anomapay-solana-resource's action builders and wrapped as pa-testkit
//! action witnesses.

use anoma_pa_testkit::witness::{ActionWitnesses, LogicWitness};
use anoma_rm_risc0::Digest;
use anoma_rm_risc0::kind_tree::KindTree;
use anoma_rm_risc0::merkle_path::MerklePath;
use anoma_rm_risc0::nullifier_key::NullifierKey;
use anoma_rm_risc0::resource::Resource;
use anoma_rm_risc0_gadgets::authority::{
    AuthoritySignature, AuthoritySigningKey, AuthorityVerifyingKey,
};
use anoma_rm_risc0_gadgets::encryption::{SecretKey, generate_public_key};
use anyhow::Context;
use emergency_migrating_transfer_library::action::migrate as migrate_action;
use emergency_migrating_transfer_witness::{
    EMERGENCY_MIGRATING_AUTH_SIGNATURE_DOMAIN, MigrateEntry,
};
use k256::AffinePoint;
use k256::elliptic_curve::rand_core::SeedableRng;
use rand_chacha::ChaCha20Rng;
use solana_keypair::Keypair;
use solana_signer::Signer;
use surfpool_sdk::Pubkey;
use transfer_library::TransferLogic;
use transfer_library::action::{
    self, ComplianceParams, Owner, TransferAction, Unwrap, Wrap, WrapAuth,
};
use transfer_witness::{ForwarderCall, LabelInfo, ValueInfo, WrapAuthInfo};

use crate::logic::{Logic, Witness};

/// Year 2100: the deadline never passes under test.
pub const WRAP_DEADLINE: i64 = 4_102_444_800;

/// The 32 bytes a label hashes to: the seed of every test key.
pub fn label_hash(label: &str) -> [u8; 32] {
    anoma_rm_risc0::utils::hash_bytes(label.as_bytes()).into()
}

fn scalar(label: &str) -> k256::Scalar {
    *k256::SecretKey::from_slice(&label_hash(label))
        .expect("a sha256 output is a valid secp256k1 scalar")
        .to_nonzero_scalar()
}

/// A shielded resource's owner: the keys the resource commits to, and the
/// authorization signing key behind them. Seeded from a label, so a test's
/// owner is the same on every run.
#[derive(Clone)]
pub struct ShieldedOwner {
    pub auth_sk: AuthoritySigningKey,
    pub keys: Owner,
}

impl ShieldedOwner {
    pub fn seeded(label: &str) -> Self {
        let auth_sk = AuthoritySigningKey::from_bytes(&label_hash(&format!("{label}/auth")))
            .expect("a sha256 output is a valid secp256k1 scalar");
        ShieldedOwner {
            keys: Owner {
                value: ValueInfo {
                    auth_pk: AuthorityVerifyingKey::from_signing_key(&auth_sk),
                    encryption_pk: generate_public_key(
                        SecretKey::new(scalar(&format!("{label}/encryption"))).inner(),
                    ),
                },
                nf_key: NullifierKey::from_bytes(label_hash(&format!("{label}/nf_key"))),
            },
            auth_sk,
        }
    }
}

/// The label of the forwarder `forwarder`'s resources of `mint`.
fn label(forwarder: Pubkey, mint: Pubkey) -> LabelInfo {
    LabelInfo {
        forwarder_program_id: forwarder.to_bytes(),
        spl_token_mint: mint.to_bytes(),
    }
}

/// The key the created resources' discovery payloads are encrypted to.
pub fn discovery_pk() -> AffinePoint {
    generate_public_key(SecretKey::new(scalar("anomapay-spl-token-forwarder/discovery")).inner())
}

/// The randomness of the created resources' payloads, drawn from `seed`.
pub fn rng(seed: &str) -> ChaCha20Rng {
    ChaCha20Rng::from_seed(label_hash(&format!("{seed}/rng")))
}

/// The compliance facts of an action: commitment randomness drawn from
/// `seed`, and the kind tree it is proven against.
pub fn compliance<'a>(seed: &str, kind_tree: &'a KindTree) -> ComplianceParams<'a> {
    ComplianceParams {
        rcv: scalar(&format!("{seed}/rcv")).to_bytes().to_vec(),
        kind_tree,
    }
}

/// An action's witnesses as pa-testkit's.
pub fn witnesses<L: Logic>(action: TransferAction<L>) -> ActionWitnesses {
    ActionWitnesses {
        compliance_witness: Box::new(action.compliance_witness),
        logic_witnesses: action
            .logic_witnesses
            .into_iter()
            .map(|logic| Box::new(Witness(logic)) as Box<dyn LogicWitness>)
            .collect(),
    }
}

/// A wrap, unproven: its witnesses, the shielded resource it creates, and
/// the message the user signed, which its input carries the signature over.
pub struct WrapData {
    pub witnesses: ActionWitnesses,
    /// The shielded resource the wrap creates, which an unwrap consumes.
    pub created: Resource,
    /// The base64 digest of the wrap message, as UTF-8 text.
    pub signed_message: Vec<u8>,
}

/// What a user's wrap authorization names besides the mint: the amount, the
/// forwarder nonce, and where the settlement transaction carries the ed25519
/// instruction.
#[derive(Clone, Copy)]
pub struct WrapTerms {
    pub amount: u64,
    pub nonce: u64,
    pub ed25519_ix_index: u8,
}

/// The resources of a wrap of `amount` of `mint` through the forwarder
/// `forwarder` into a resource `owner` holds. `seed` makes them distinct
/// from every other test's.
pub fn wrap_flow(
    forwarder: Pubkey,
    mint: Pubkey,
    owner: &ShieldedOwner,
    amount: u64,
    seed: &str,
) -> Wrap<TransferLogic> {
    action::wrap(
        label(forwarder, mint),
        amount,
        label_hash(&format!("{seed}/ephemeral-nonce")),
        owner.keys.clone(),
        label_hash(&format!("{seed}/rand-seed")),
    )
}

/// `user`'s authorization of `wrap` on `terms` in the action of tree root
/// `root`, and the message it signs: the wrap's signed message, as UTF-8
/// text.
pub fn wrap_auth(
    wrap: &Wrap<TransferLogic>,
    user: &Keypair,
    terms: WrapTerms,
    root: Digest,
) -> anyhow::Result<(WrapAuth, Vec<u8>)> {
    let message = wrap
        .signed_message(terms.nonce, WRAP_DEADLINE, root)
        .context("failed to derive the wrap's signed message")?
        .into_bytes();
    let auth = WrapAuth {
        user: user.pubkey().to_bytes(),
        info: WrapAuthInfo {
            nonce: terms.nonce,
            deadline: WRAP_DEADLINE,
            signature: user.sign_message(&message).into(),
            ed25519_ix_index: terms.ed25519_ix_index,
        },
    };
    Ok((auth, message))
}

/// A wrap through the forwarder `forwarder` of `terms.amount` of `mint`,
/// from `user`'s token account into a resource `owner` holds, proven against
/// `kind_tree`. `seed` makes the action's resources distinct from every
/// other test's.
pub fn wrap(
    forwarder: Pubkey,
    mint: Pubkey,
    user: &Keypair,
    owner: &ShieldedOwner,
    terms: WrapTerms,
    kind_tree: &KindTree,
    seed: &str,
) -> anyhow::Result<WrapData> {
    let wrap = wrap_flow(forwarder, mint, owner, terms.amount, seed);
    let layout = wrap
        .deposit
        .layout()
        .context("failed to lay out the wrap")?;
    let (auth, message) = wrap_auth(&wrap, user, terms, layout.root())?;
    let action = wrap
        .action(
            auth,
            &discovery_pk(),
            compliance(seed, kind_tree),
            &mut rng(seed),
        )
        .context("failed to build the wrap's witnesses")?;
    Ok(WrapData {
        witnesses: witnesses(action),
        created: layout.resource(1),
        signed_message: message,
    })
}

/// The resources of an unwrap of `wrapped`, `owner`'s shielded resource of
/// `L` under the label of `forwarder` and `mint` at `path` in the adapter's
/// commitment tree, to `recipient`'s token account.
pub fn unwrap_flow<L: Logic>(
    forwarder: Pubkey,
    mint: Pubkey,
    wrapped: Resource,
    owner: &ShieldedOwner,
    recipient: Pubkey,
    path: MerklePath,
) -> anyhow::Result<Unwrap<L>> {
    action::unwrap(
        label(forwarder, mint),
        wrapped,
        path,
        owner.keys.clone(),
        recipient.to_bytes(),
    )
    .context("failed to build the unwrap's resources")
}

/// `owner`'s signature over the tree root `root` of an action that spends
/// its resource of `L`, under `L`'s domain.
pub fn unwrap_auth<L: Logic>(owner: &ShieldedOwner, root: Digest) -> AuthoritySignature {
    owner.auth_sk.sign(
        <L::Call as ForwarderCall>::AUTH_SIGNATURE_DOMAIN,
        root.as_bytes(),
    )
}

/// An unwrap of `wrapped`, `owner`'s shielded resource of `L` under the
/// label of `forwarder` and `mint`, releasing its tokens to `recipient`'s
/// token account. `path` is the wrapped resource's path in the adapter's
/// commitment tree.
pub fn unwrap<L: Logic>(
    forwarder: Pubkey,
    mint: Pubkey,
    wrapped: Resource,
    owner: &ShieldedOwner,
    recipient: Pubkey,
    path: MerklePath,
) -> anyhow::Result<ActionWitnesses> {
    let unwrap = unwrap_flow::<L>(forwarder, mint, wrapped, owner, recipient, path)?;
    let root = unwrap
        .layout()
        .context("failed to lay out the unwrap")?
        .root();
    let action = unwrap
        .action(
            unwrap_auth::<L>(owner, root),
            compliance(
                &format!("unwrap/{root}"),
                anoma_pa_testkit::fixtures::kind_tree(),
            ),
        )
        .context("failed to build the unwrap's witnesses")?;
    Ok(witnesses(action))
}

/// A migration, unproven: its witnesses and the resource of the
/// emergency-migrating logic it creates.
pub struct MigrationData {
    pub witnesses: ActionWitnesses,
    pub created: Resource,
}

/// A migration of `wrapped`, resources `owner` holds under the label of
/// `forwarder` and `mint`, at `path` in the adapter's commitment tree,
/// into one resource of the emergency-migrating logic `owner` holds.
/// `seed` makes the action's resources distinct from every other test's.
pub fn migrate(
    forwarder: Pubkey,
    mint: Pubkey,
    wrapped: Resource,
    path: MerklePath,
    owner: &ShieldedOwner,
    seed: &str,
) -> anyhow::Result<MigrationData> {
    let entry = MigrateEntry {
        resource: wrapped,
        nf_key: owner.keys.nf_key.clone(),
        path,
        value_info: owner.keys.value.clone(),
        forwarder_program_id: forwarder.to_bytes(),
    };
    let migration = migrate_action(
        label(forwarder, mint),
        vec![entry],
        label_hash(&format!("{seed}/ephemeral-nonce")),
        owner.keys.clone(),
        label_hash(&format!("{seed}/rand-seed")),
    )
    .context("failed to build the migration's resources")?;
    let layout = migration
        .deposit
        .layout()
        .context("failed to lay out the migration")?;
    let auth_sig = owner.auth_sk.sign(
        EMERGENCY_MIGRATING_AUTH_SIGNATURE_DOMAIN,
        layout.root().as_bytes(),
    );
    let action = migration
        .action(
            auth_sig,
            &discovery_pk(),
            compliance(seed, anoma_pa_testkit::fixtures::kind_tree()),
            &mut rng(seed),
        )
        .context("failed to build the migration's witnesses")?;
    Ok(MigrationData {
        witnesses: witnesses(action),
        created: layout.resource(1),
    })
}
