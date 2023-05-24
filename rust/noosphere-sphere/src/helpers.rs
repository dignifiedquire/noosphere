// #![cfg(any(test, doc, feature = "helpers"))]

use std::sync::Arc;

use anyhow::Result;
use noosphere_core::{
    authority::{generate_capability, generate_ed25519_key, Author, SphereAction},
    data::{ContentType, Did, Link, LinkRecord},
    view::Sphere,
};
use noosphere_storage::{BlockStore, MemoryStorage, SphereDb, TrackingStorage, UcanStore};
use serde_json::json;
use tokio::{io::AsyncReadExt, sync::Mutex};
use ucan::{builder::UcanBuilder, crypto::KeyMaterial, store::UcanJwtStore};
use ucan_key_support::ed25519::Ed25519KeyMaterial;

use crate::{
    walk_versioned_map_elements, walk_versioned_map_elements_and, HasMutableSphereContext,
    HasSphereContext, SphereContentRead, SphereContentWrite, SphereContext, SpherePetnameWrite,
};

/// Access levels available when simulating a [SphereContext]
pub enum SimulationAccess {
    Readonly,
    ReadWrite,
}

/// Create a temporary, non-persisted [SphereContext] that tracks usage
/// internally. This is intended for use in docs and tests, and should otherwise
/// be ignored. When creating the simulated [SphereContext], you can pass a
/// [SimulationAccess] to control the kind of access the emphemeral credentials
/// have to the [SphereContext].
pub async fn simulated_sphere_context(
    profile: SimulationAccess,
    db: Option<SphereDb<TrackingStorage<MemoryStorage>>>,
) -> Result<Arc<Mutex<SphereContext<Ed25519KeyMaterial, TrackingStorage<MemoryStorage>>>>> {
    let mut db = match db {
        Some(db) => db,
        None => {
            let storage_provider = TrackingStorage::wrap(MemoryStorage::default());
            SphereDb::new(&storage_provider).await?
        }
    };

    let owner_key = generate_ed25519_key();
    let owner_did = owner_key.get_did().await?;

    let (sphere, proof, _) = Sphere::generate(&owner_did, &mut db).await?;

    let sphere_identity = sphere.get_identity().await?;
    let author = Author {
        key: owner_key,
        authorization: match profile {
            SimulationAccess::Readonly => None,
            SimulationAccess::ReadWrite => Some(proof),
        },
    };

    db.set_version(&sphere_identity, sphere.cid()).await?;

    Ok(Arc::new(Mutex::new(
        SphereContext::new(sphere_identity, author, db, None).await?,
    )))
}

/// Make a valid link record that represents a sphere "in the distance." The
/// link record and its proof are both put into the provided [UcanJwtStore]
pub async fn make_valid_link_record<S>(store: &mut S) -> Result<(Did, LinkRecord, Link<LinkRecord>)>
where
    S: UcanJwtStore,
{
    let owner_key = generate_ed25519_key();
    let owner_did = owner_key.get_did().await?;
    let mut db = SphereDb::new(&MemoryStorage::default()).await?;

    let (sphere, proof, _) = Sphere::generate(&owner_did, &mut db).await?;
    let ucan_proof = proof.resolve_ucan(&db).await?;

    let sphere_identity = sphere.get_identity().await?;

    let link_record = LinkRecord::from(
        UcanBuilder::default()
            .issued_by(&owner_key)
            .for_audience(&sphere_identity)
            .witnessed_by(&ucan_proof)
            .claiming_capability(&generate_capability(
                &sphere_identity,
                SphereAction::Publish,
            ))
            .with_lifetime(120)
            .with_fact(json!({
              "link": sphere.cid().to_string()
            }))
            .build()?
            .sign()
            .await?,
    );

    store.write_token(&ucan_proof.encode()?).await?;
    let link = Link::from(store.write_token(&link_record.encode()?).await?);

    Ok((sphere_identity, link_record, link))
}

#[cfg(docs)]
use noosphere_core::data::MemoIpld;

/// Attempt to walk an entire sphere, touching every block up to and including
/// any [MemoIpld] nodes, but excluding those memo's body content. This helper
/// is useful for asserting that the blocks expected to be sent during
/// replication have in fact been sent.
pub async fn touch_all_sphere_blocks<S>(sphere: &Sphere<S>) -> Result<()>
where
    S: BlockStore + 'static,
{
    trace!("Touching content blocks...");
    let content = sphere.get_content().await?;
    let _ = content.load_changelog().await?;

    walk_versioned_map_elements(content).await?;

    trace!("Touching identity blocks...");
    let identities = sphere.get_address_book().await?.get_identities().await?;
    let _ = identities.load_changelog().await?;

    walk_versioned_map_elements_and(
        identities,
        sphere.store().clone(),
        |_, identity, store| async move {
            let ucan_store = UcanStore(store);
            if let Some(record) = identity.link_record(&ucan_store).await {
                record.collect_proofs(&ucan_store).await?;
            }
            Ok(())
        },
    )
    .await?;

    trace!("Touching authority blocks...");
    let authority = sphere.get_authority().await?;

    trace!("Touching delegation blocks...");
    let delegations = authority.get_delegations().await?;
    walk_versioned_map_elements(delegations).await?;

    trace!("Touching revocation blocks...");
    let revocations = authority.get_revocations().await?;
    walk_versioned_map_elements(revocations).await?;

    Ok(())
}

pub type TrackedHasMutableSphereContext =
    Arc<Mutex<SphereContext<Ed25519KeyMaterial, TrackingStorage<MemoryStorage>>>>;

/// Create a series of spheres where each sphere has the next as resolved
/// entry in its address book; return a [HasMutableSphereContext] for the
/// first sphere in the sequence.
pub async fn make_sphere_context_with_peer_chain(
    peer_chain: &[String],
) -> Result<(TrackedHasMutableSphereContext, Vec<Did>)> {
    let origin_sphere_context = simulated_sphere_context(SimulationAccess::ReadWrite, None)
        .await
        .unwrap();

    let mut db = origin_sphere_context
        .sphere_context()
        .await
        .unwrap()
        .db()
        .clone();

    let mut contexts = vec![origin_sphere_context.clone()];

    for name in peer_chain.iter() {
        let mut sphere_context =
            simulated_sphere_context(SimulationAccess::ReadWrite, Some(db.clone()))
                .await
                .unwrap();

        sphere_context
            .write("my-name", &ContentType::Subtext, name.as_bytes(), None)
            .await
            .unwrap();
        sphere_context.save(None).await.unwrap();

        contexts.push(sphere_context);
    }

    let mut next_sphere_context: Option<TrackedHasMutableSphereContext> = None;
    let mut dids = Vec::new();

    for mut sphere_context in contexts.into_iter().rev() {
        dids.push(sphere_context.identity().await?);
        if let Some(next_sphere_context) = next_sphere_context {
            let version = next_sphere_context.version().await.unwrap();

            let next_author = next_sphere_context
                .sphere_context()
                .await
                .unwrap()
                .author()
                .clone();
            let next_identity = next_sphere_context.identity().await.unwrap();

            let link_record = LinkRecord::from(
                UcanBuilder::default()
                    .issued_by(&next_author.key)
                    .for_audience(&next_identity)
                    .witnessed_by(
                        &next_author
                            .authorization
                            .as_ref()
                            .unwrap()
                            .resolve_ucan(&db)
                            .await
                            .unwrap(),
                    )
                    .claiming_capability(&generate_capability(
                        &next_identity,
                        SphereAction::Publish,
                    ))
                    .with_lifetime(120)
                    .with_fact(json!({
                    "link": version.to_string()
                    }))
                    .build()
                    .unwrap()
                    .sign()
                    .await
                    .unwrap(),
            );

            let mut name = String::new();
            let mut file = next_sphere_context.read("my-name").await.unwrap().unwrap();
            file.contents.read_to_string(&mut name).await.unwrap();

            debug!("Adopting {name}");

            sphere_context
                .adopt_petname(&name, &link_record)
                .await
                .unwrap();
            let identity = sphere_context.identity().await?;

            db.set_version(&identity, &sphere_context.save(None).await.unwrap())
                .await
                .unwrap();
        }

        next_sphere_context = Some(sphere_context);
    }

    Ok((origin_sphere_context, dids))
}
