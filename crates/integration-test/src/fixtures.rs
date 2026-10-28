//! AnomaPay wrap, unwrap and migration actions, built with
//! anomapay-solana-resource's action builders and wrapped as pa-testkit
//! action witnesses.

use anoma_pa_testkit::witness::{ActionWitnesses, LogicWitness};
use anoma_rm_risc0::Digest;
use anoma_rm_risc0::action_tree::ActionTree;
use anoma_rm_risc0::compliance::{ComplianceWitness, INITIAL_ROOT};
use anoma_rm_risc0::kind_tree::KindTree;
use anoma_rm_risc0::merkle_path::MerklePath;
use anoma_rm_risc0::nullifier_key::NullifierKey;
use anoma_rm_risc0::resource::{Resource, ResourceWitness, ResourceWitnessConsumed};
use anoma_rm_risc0_gadgets::authority::{AuthoritySigningKey, AuthorityVerifyingKey};
use anoma_rm_risc0_gadgets::encryption::{SecretKey, generate_public_key};
use anomapay_spl_token_forwarder_client::WrapMessage;
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
use transfer_library::action::{self, ComplianceParams, Owner, TransferAction, WrapAuth};
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
fn discovery_pk() -> AffinePoint {
    generate_public_key(SecretKey::new(scalar("anomapay-spl-token-forwarder/discovery")).inner())
}

/// The compliance facts of an action: commitment randomness drawn from
/// `seed`, and the kind tree it is proven against.
fn compliance<'a>(seed: &str, kind_tree: &'a KindTree) -> ComplianceParams<'a> {
    ComplianceParams {
        rcv: scalar(&format!("{seed}/rcv")).to_bytes().to_vec(),
        kind_tree,
    }
}

fn witnesses<L: Logic>(action: TransferAction<L>) -> ActionWitnesses {
    ActionWitnesses {
        compliance_witness: Box::new(action.compliance_witness),
        logic_witnesses: vec![
            Box::new(Witness(action.consumed_logic)),
            Box::new(Witness(action.created_logic)),
        ],
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
    let wrap = action::wrap::<TransferLogic>(
        label(forwarder, mint),
        terms.amount,
        label_hash(&format!("{seed}/ephemeral-nonce")),
        owner.keys.clone(),
        label_hash(&format!("{seed}/rand-seed")),
    )
    .map_err(|e| anyhow::anyhow!("{e:?}"))
    .context("failed to build the wrap's resources")?;
    let message = wrap
        .signed_message(terms.nonce, WRAP_DEADLINE)
        .map_err(|e| anyhow::anyhow!("{e:?}"))
        .context("failed to derive the wrap's signed message")?
        .into_bytes();
    let signature: [u8; 64] = user.sign_message(&message).into();
    let auth = WrapAuth {
        user: user.pubkey().to_bytes(),
        info: WrapAuthInfo {
            nonce: terms.nonce,
            deadline: WRAP_DEADLINE,
            signature,
            ed25519_ix_index: terms.ed25519_ix_index,
        },
    };
    let action = wrap
        .action(
            auth,
            &discovery_pk(),
            compliance(seed, kind_tree),
            &mut ChaCha20Rng::from_seed(label_hash(&format!("{seed}/rng"))),
        )
        .map_err(|e| anyhow::anyhow!("{e:?}"))
        .context("failed to build the wrap's witnesses")?;
    Ok(WrapData {
        witnesses: witnesses(action),
        created: wrap.deposit.created,
        signed_message: message,
    })
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
    let unwrap = action::unwrap::<L>(
        label(forwarder, mint),
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
    let auth_sig = owner.auth_sk.sign(
        <L::Call as ForwarderCall>::AUTH_SIGNATURE_DOMAIN,
        root.as_bytes(),
    );
    let action = unwrap
        .action(
            auth_sig,
            path,
            compliance(
                &format!("unwrap/{}", root),
                anoma_pa_testkit::fixtures::kind_tree(),
            ),
        )
        .map_err(|e| anyhow::anyhow!("{e:?}"))
        .context("failed to build the unwrap's witnesses")?;
    Ok(witnesses(action))
}

/// Builds the logic witness of a resource of an action from the resource and
/// the action's tree root.
pub type PartLogic = Box<dyn FnOnce(Resource, Digest) -> anyhow::Result<Box<dyn LogicWitness>>>;

/// A resource of an action that [`action`] assembles from several flows, and
/// its logic.
pub enum Part {
    /// A consumed resource, the key that nullifies it, and its path in the
    /// adapter's commitment tree, or None for an ephemeral resource.
    Consumed {
        resource: Resource,
        nf_key: NullifierKey,
        path: Option<MerklePath>,
        logic: PartLogic,
    },
    /// A created resource; [`action`] derives its nonce.
    Created {
        resource: Resource,
        logic: PartLogic,
    },
}

/// A flow's resources for [`action`]: the one it consumes and the one it
/// creates.
pub struct Parts {
    pub consumed: Part,
    pub created: Part,
}

/// One action of `parts`, in their order, which is the order the adapter
/// settles them and runs their forwarder calls in, proven against the empty
/// kind tree. A created resource's nonce derives from its index among the
/// created resources and the action's nullifiers, as the compliance circuit
/// requires. `seed` draws the action's commitment randomness.
pub fn action(parts: Vec<Part>, seed: &str) -> anyhow::Result<ActionWitnesses> {
    let nullifiers = parts
        .iter()
        .filter_map(|part| match part {
            Part::Consumed {
                resource, nf_key, ..
            } => Some(resource.nullifier(nf_key)),
            Part::Created { .. } => None,
        })
        .collect::<Result<Vec<Digest>, _>>()
        .map_err(|e| anyhow::anyhow!("{e:?}"))
        .context("failed to compute the consumed nullifiers")?;

    let mut consumed_nullifiers = nullifiers.iter().copied();
    let mut created_index = 0;
    let mut tags = Vec::with_capacity(parts.len());
    let mut placed = Vec::with_capacity(parts.len());
    for part in parts {
        match part {
            Part::Consumed {
                resource,
                nf_key,
                path,
                logic,
            } => {
                tags.push(
                    consumed_nullifiers
                        .next()
                        .expect("one nullifier per consumed resource"),
                );
                let witness = match path {
                    Some(path) => {
                        ResourceWitnessConsumed::from_resource_with_path(resource, nf_key, path)
                    }
                    None => ResourceWitnessConsumed::from_resource(resource, nf_key),
                };
                placed.push((resource, ResourceWitness::Consumed(witness), logic));
            }
            Part::Created {
                mut resource,
                logic,
            } => {
                resource.nonce = Resource::derive_nonce_from_nullifiers(created_index, &nullifiers)
                    .map_err(|e| anyhow::anyhow!("{e:?}"))
                    .with_context(|| format!("failed to derive created nonce {created_index}"))?;
                created_index += 1;
                tags.push(resource.commitment());
                placed.push((resource, ResourceWitness::Created(resource), logic));
            }
        }
    }
    let root = ActionTree::new(tags)
        .root()
        .map_err(|e| anyhow::anyhow!("{e:?}"))
        .context("failed to compute the action tree root")?;

    let mut resource_witnesses = Vec::with_capacity(placed.len());
    let mut logic_witnesses = Vec::with_capacity(placed.len());
    for (position, (resource, resource_witness, logic)) in placed.into_iter().enumerate() {
        resource_witnesses.push(resource_witness);
        logic_witnesses.push(
            logic(resource, root)
                .with_context(|| format!("failed to build the logic of resource {position}"))?,
        );
    }
    let kind_tree = anoma_pa_testkit::fixtures::kind_tree();
    let ComplianceParams { rcv, .. } = compliance(seed, kind_tree);
    let kind_paths = kind_tree.kind_paths_for(&resource_witnesses);
    Ok(ActionWitnesses {
        compliance_witness: Box::new(ComplianceWitness::from_parts(
            resource_witnesses,
            INITIAL_ROOT,
            &rcv,
            kind_paths,
        )),
        logic_witnesses,
    })
}

/// A wrap's resources for [`action`], as [`wrap`] builds them: the user signs
/// the root of the action they end up in.
pub fn wrap_parts(
    forwarder: Pubkey,
    mint: Pubkey,
    user: &Keypair,
    owner: &ShieldedOwner,
    terms: WrapTerms,
    seed: &str,
) -> anyhow::Result<Parts> {
    let wrap = action::wrap::<TransferLogic>(
        label(forwarder, mint),
        terms.amount,
        label_hash(&format!("{seed}/ephemeral-nonce")),
        owner.keys.clone(),
        label_hash(&format!("{seed}/rand-seed")),
    )
    .map_err(|e| anyhow::anyhow!("{e:?}"))
    .context("failed to build the wrap's resources")?;
    let (wrap_label, user) = (label(forwarder, mint), user.insecure_clone());
    let consumed: PartLogic = Box::new(move |resource, root| {
        let message = WrapMessage {
            forwarder_id: forwarder.to_bytes(),
            token_mint: mint.to_bytes(),
            amount: terms.amount,
            nonce: terms.nonce,
            deadline: WRAP_DEADLINE,
            action_tree_root: root.into(),
        }
        .base64_digest();
        Ok(Box::new(Witness(
            TransferLogic::mint_resource_logic_with_wrap_auth(
                resource,
                root,
                NullifierKey::default(),
                wrap_label,
                user.pubkey().to_bytes(),
                WrapAuthInfo {
                    nonce: terms.nonce,
                    deadline: WRAP_DEADLINE,
                    signature: user.sign_message(message.as_bytes()).into(),
                    ed25519_ix_index: terms.ed25519_ix_index,
                },
            ),
        )))
    });
    let (created_label, value, rng_seed) = (
        label(forwarder, mint),
        owner.keys.value.clone(),
        label_hash(&format!("{seed}/rng")),
    );
    let created: PartLogic = Box::new(move |resource, root| {
        Ok(Box::new(Witness(
            TransferLogic::create_persistent_resource_logic(
                resource,
                root,
                &discovery_pk(),
                value,
                created_label,
                &mut ChaCha20Rng::from_seed(rng_seed),
            ),
        )))
    });
    let deposit = wrap.deposit;
    Ok(Parts {
        consumed: Part::Consumed {
            resource: deposit.consumed,
            nf_key: NullifierKey::default(),
            path: None,
            logic: consumed,
        },
        created: Part::Created {
            resource: deposit.created,
            logic: created,
        },
    })
}

/// An unwrap's resources for [`action`], as [`unwrap`] builds them for the
/// transfer logic: the owner signs the root of the action they end up in.
pub fn unwrap_parts(
    forwarder: Pubkey,
    mint: Pubkey,
    wrapped: Resource,
    owner: &ShieldedOwner,
    recipient: Pubkey,
    path: MerklePath,
) -> anyhow::Result<Parts> {
    let unwrap = action::unwrap::<TransferLogic>(
        label(forwarder, mint),
        wrapped,
        owner.keys.clone(),
        recipient.to_bytes(),
    )
    .map_err(|e| anyhow::anyhow!("{e:?}"))
    .context("failed to build the unwrap's resources")?;
    let signer = owner.clone();
    let consumed: PartLogic = Box::new(move |resource, root| {
        let auth_sig = signer.auth_sk.sign(
            <<TransferLogic as Logic>::Call as ForwarderCall>::AUTH_SIGNATURE_DOMAIN,
            root.as_bytes(),
        );
        Ok(Box::new(Witness(
            TransferLogic::consume_persistent_resource_logic(
                resource,
                root,
                signer.keys.nf_key,
                signer.keys.value,
                auth_sig,
            ),
        )))
    });
    let burn_label = label(forwarder, mint);
    let created: PartLogic = Box::new(move |resource, root| {
        Ok(Box::new(Witness(TransferLogic::burn_resource_logic(
            resource,
            root,
            burn_label,
            recipient.to_bytes(),
        ))))
    });
    Ok(Parts {
        consumed: Part::Consumed {
            resource: unwrap.consumed,
            nf_key: owner.keys.nf_key.clone(),
            path: Some(path),
            logic: consumed,
        },
        created: Part::Created {
            resource: unwrap.created,
            logic: created,
        },
    })
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
    .map_err(|e| anyhow::anyhow!("{e:?}"))
    .context("failed to build the migration's resources")?;
    let root: Digest = migration
        .deposit
        .action_tree_root()
        .map_err(|e| anyhow::anyhow!("{e:?}"))
        .context("failed to compute the migration's action tree root")?;
    let auth_sig = owner
        .auth_sk
        .sign(EMERGENCY_MIGRATING_AUTH_SIGNATURE_DOMAIN, root.as_bytes());
    let action = migration
        .action(
            auth_sig,
            &discovery_pk(),
            compliance(seed, anoma_pa_testkit::fixtures::kind_tree()),
            &mut ChaCha20Rng::from_seed(label_hash(&format!("{seed}/rng"))),
        )
        .map_err(|e| anyhow::anyhow!("{e:?}"))
        .context("failed to build the migration's witnesses")?;
    Ok(MigrationData {
        witnesses: witnesses(action),
        created: migration.deposit.created,
    })
}
