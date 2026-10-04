//! AnomaPay wrap and unwrap actions, built with anomapay-solana-resource's
//! action builders and wrapped as pa-testkit action witnesses.

use anoma_pa_testkit::witness::ActionWitnesses;
use anoma_rm_risc0::Digest;
use anoma_rm_risc0::compliance::KindTableEntry;
use anoma_rm_risc0::merkle_path::MerklePath;
use anoma_rm_risc0::nullifier_key::NullifierKey;
use anoma_rm_risc0::resource::Resource;
use anoma_rm_risc0_gadgets::authority::{AuthoritySigningKey, AuthorityVerifyingKey};
use anoma_rm_risc0_gadgets::encryption::{SecretKey, generate_public_key};
use anyhow::Context;
use k256::AffinePoint;
use k256::elliptic_curve::rand_core::SeedableRng;
use rand_chacha::ChaCha20Rng;
use solana_keypair::Keypair;
use solana_signer::Signer;
use surfpool_sdk::Pubkey;
use transfer_library::action::{self, ComplianceParams, Owner, TransferAction, WrapAuth};
use transfer_witness::{AUTH_SIGNATURE_DOMAIN, LabelInfo, ValueInfo, WrapAuthInfo};

use crate::logic::Witness;

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

/// The key the created resources' discovery payloads are encrypted to.
fn discovery_pk() -> AffinePoint {
    generate_public_key(SecretKey::new(scalar("anomapay-spl-token-forwarder/discovery")).inner())
}

/// The compliance facts of an action: commitment randomness drawn from
/// `seed`, and the kind table it is proven against.
fn compliance(seed: &str, kind_table: Vec<KindTableEntry>) -> ComplianceParams {
    ComplianceParams {
        rcv: scalar(&format!("{seed}/rcv")).to_bytes().to_vec(),
        kind_table,
    }
}

/// The kind table loaded for proving: the empty table locally, the
/// deployment's in e2e.
pub fn loaded_kind_table() -> Vec<KindTableEntry> {
    anoma_rm_risc0::constants::kind_table().to_vec()
}

fn witnesses(action: TransferAction) -> ActionWitnesses {
    ActionWitnesses {
        compliance_witness: Box::new(action.compliance_witness),
        logic_witnesses: vec![
            Box::new(Witness(action.consumed_logic)),
            Box::new(Witness(action.created_logic)),
        ],
    }
}

/// What the user's wallet signs to authorize a wrap, and the signature: the
/// ed25519 instruction of the wrap's settlement carries both.
#[derive(Clone)]
pub struct WrapAuthorization {
    /// The base64 digest of the wrap message, as UTF-8 text.
    pub message: Vec<u8>,
    pub signature: [u8; 64],
    /// The key the instruction names as the signer.
    pub signer: Pubkey,
}

/// A wrap, unproven: its witnesses, the shielded resource it creates, and
/// the user's authorization of it.
pub struct WrapData {
    pub witnesses: ActionWitnesses,
    /// The shielded resource the wrap creates, which an unwrap consumes.
    pub created: Resource,
    pub authorization: WrapAuthorization,
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

/// A wrap through the forwarder `forwarder` of `terms.amount` of `mint`,
/// from `user`'s token account into a resource `owner` holds, proven against
/// `kind_table`. `seed` makes the action's resources distinct from every
/// other test's.
pub fn wrap(
    forwarder: Pubkey,
    mint: Pubkey,
    user: &Keypair,
    owner: &ShieldedOwner,
    terms: WrapTerms,
    kind_table: Vec<KindTableEntry>,
    seed: &str,
) -> anyhow::Result<WrapData> {
    let wrap = action::wrap(
        LabelInfo {
            forwarder_program_id: forwarder.to_bytes(),
            spl_token_mint: mint.to_bytes(),
        },
        terms.amount,
        label_hash(&format!("{seed}/ephemeral-nonce")),
        owner.keys.clone(),
        label_hash(&format!("{seed}/rand-seed")),
    )
    .map_err(|e| anyhow::anyhow!("{e:?}"))
    .context("failed to build the wrap's resources")?;
    let auth = WrapAuth {
        user: user.pubkey().to_bytes(),
        info: WrapAuthInfo {
            nonce: terms.nonce,
            deadline: WRAP_DEADLINE,
            ed25519_ix_index: terms.ed25519_ix_index,
        },
    };
    let message = wrap
        .signed_message(&auth)
        .map_err(|e| anyhow::anyhow!("{e:?}"))
        .context("failed to derive the wrap's signed message")?
        .into_bytes();
    let signature = user.sign_message(&message);
    let action = wrap
        .action(
            auth,
            &discovery_pk(),
            compliance(seed, kind_table),
            &mut ChaCha20Rng::from_seed(label_hash(&format!("{seed}/rng"))),
        )
        .map_err(|e| anyhow::anyhow!("{e:?}"))
        .context("failed to build the wrap's witnesses")?;
    Ok(WrapData {
        witnesses: witnesses(action),
        created: wrap.created,
        authorization: WrapAuthorization {
            message,
            signature: signature.into(),
            signer: user.pubkey(),
        },
    })
}

/// An unwrap of `wrapped`, `owner`'s shielded resource under the label of
/// `forwarder` and `mint`, releasing its tokens to `recipient`'s token
/// account. `path` is the wrapped resource's path in the adapter's
/// commitment tree.
pub fn unwrap(
    forwarder: Pubkey,
    mint: Pubkey,
    wrapped: Resource,
    owner: &ShieldedOwner,
    recipient: Pubkey,
    path: MerklePath,
) -> anyhow::Result<ActionWitnesses> {
    let unwrap = action::unwrap(
        LabelInfo {
            forwarder_program_id: forwarder.to_bytes(),
            spl_token_mint: mint.to_bytes(),
        },
        wrapped,
        owner.keys.clone(),
        recipient.to_bytes(),
    )
    .map_err(|e| anyhow::anyhow!("{e:?}"))
    .context("failed to build the unwrap's resources")?;
    let root: Digest = unwrap
        .action_tree_root()
        .map_err(|e| anyhow::anyhow!("{e:?}"))
        .context("failed to compute the unwrap's action tree root")?;
    let auth_sig = owner.auth_sk.sign(AUTH_SIGNATURE_DOMAIN, root.as_bytes());
    let action = unwrap
        .action(
            auth_sig,
            path,
            compliance(&format!("unwrap/{}", root), loaded_kind_table()),
        )
        .map_err(|e| anyhow::anyhow!("{e:?}"))
        .context("failed to build the unwrap's witnesses")?;
    Ok(witnesses(action))
}
