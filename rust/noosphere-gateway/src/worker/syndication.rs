use std::{io::Cursor, sync::Arc};

use anyhow::Result;
use libipld_cbor::DagCborCodec;
use noosphere_common::UnsharedStream;
use noosphere_core::context::{
    metadata::COUNTERPART, HasMutableSphereContext, SphereContentRead, SphereContentWrite,
    SphereCursor,
};
use noosphere_core::stream::{memo_body_stream, to_car_stream};
use noosphere_core::{
    data::{ContentType, Did, Link, MemoIpld},
    view::Timeline,
};
use noosphere_ipfs::{IpfsClient, KuboClient};
use noosphere_storage::{block_deserialize, block_serialize, KeyValueStore, Storage};
use serde::{Deserialize, Serialize};
use tokio::{
    io::AsyncReadExt,
    sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
    task::JoinHandle,
};
use tokio_util::io::StreamReader;
use url::Url;

use deterministic_bloom::const_size::BloomFilter;

/// A [SyndicationJob] is a request to syndicate the blocks of a _counterpart_
/// sphere to the broader IPFS network.
pub struct SyndicationJob<C> {
    /// The revision of the _local_ sphere to discover the _counterpart_ sphere
    /// from; the counterpart sphere's revision will need to be derived using
    /// this checkpoint in local sphere history.
    pub revision: Link<MemoIpld>,
    /// The [SphereContext] that corresponds to the _local_ sphere relative to
    /// the gateway.
    pub context: C,
}

/// A [SyndicationCheckpoint] represents the last spot in the history of a
/// sphere that was successfully syndicated to an IPFS node. It records a Bloom
/// filter populated by the CIDs of all blocks that have been syndicated, which
/// gives us a short-cut to determine if a block should be added.
#[derive(Serialize, Deserialize)]
pub struct SyndicationCheckpoint {
    pub revision: Link<MemoIpld>,
    pub syndicated_blocks: BloomFilter<256, 30>,
}

/// Start a Tokio task that waits for [SyndicationJob] messages and then
/// attempts to syndicate to the configured IPFS RPC. Currently only Kubo IPFS
/// backends are supported.
pub fn start_ipfs_syndication<C, S>(
    ipfs_api: Url,
) -> (UnboundedSender<SyndicationJob<C>>, JoinHandle<Result<()>>)
where
    C: HasMutableSphereContext<S> + 'static,
    S: Storage + 'static,
{
    let (tx, rx) = unbounded_channel();

    (tx, tokio::task::spawn(ipfs_syndication_task(ipfs_api, rx)))
}

async fn ipfs_syndication_task<C, S>(
    ipfs_api: Url,
    mut receiver: UnboundedReceiver<SyndicationJob<C>>,
) -> Result<()>
where
    C: HasMutableSphereContext<S>,
    S: Storage + 'static,
{
    debug!("Syndicating sphere revisions to IPFS API at {}", ipfs_api);

    let kubo_client = Arc::new(KuboClient::new(&ipfs_api)?);

    while let Some(job) = receiver.recv().await {
        if let Err(error) = process_job(job, kubo_client.clone(), &ipfs_api).await {
            warn!("Error processing IPFS job: {}", error);
        }
    }
    Ok(())
}

async fn process_job<C, S>(
    job: SyndicationJob<C>,
    kubo_client: Arc<KuboClient>,
    ipfs_api: &Url,
) -> Result<()>
where
    C: HasMutableSphereContext<S>,
    S: Storage + 'static,
{
    let SyndicationJob { revision, context } = job;
    debug!("Attempting to syndicate version DAG {revision} to IPFS");
    let kubo_identity = kubo_client.server_identity().await.map_err(|error| {
        anyhow::anyhow!(
            "Failed to identify an IPFS Kubo node at {}: {}",
            ipfs_api,
            error
        )
    })?;
    let checkpoint_key = format!("syndication/kubo/{kubo_identity}");

    debug!("IPFS node identified as {}", kubo_identity);

    // Take a lock on the `SphereContext` and look up the most recent
    // syndication checkpoint for this Kubo node
    let (sphere_revision, ancestor_revision, syndicated_blocks, db) = {
        let db = {
            let context = context.sphere_context().await?;
            context.db().clone()
        };

        let counterpart_identity = db.require_key::<_, Did>(COUNTERPART).await?;
        let sphere = context.to_sphere().await?;
        let content = sphere.get_content().await?;

        let counterpart_revision = content.require(&counterpart_identity).await?.clone();

        let (last_syndicated_revision, syndicated_blocks) =
            match context.read(&checkpoint_key).await? {
                Some(mut file) => match file.memo.content_type() {
                    Some(ContentType::Cbor) => {
                        let mut bytes = Vec::new();
                        file.contents.read_to_end(&mut bytes).await?;
                        let SyndicationCheckpoint {
                            revision,
                            syndicated_blocks,
                        } = block_deserialize::<DagCborCodec, _>(&bytes)?;
                        (Some(revision), syndicated_blocks)
                    }
                    _ => (None, BloomFilter::default()),
                },
                None => (None, BloomFilter::default()),
            };

        (
            counterpart_revision,
            last_syndicated_revision,
            syndicated_blocks,
            db,
        )
    };

    let timeline = Timeline::new(&db)
        .slice(&sphere_revision, ancestor_revision.as_ref())
        .to_chronological()
        .await?;

    // For all CIDs since the last historical checkpoint, syndicate a CAR
    // of blocks that are unique to that revision to the backing IPFS
    // implementation
    for cid in timeline {
        let car_stream = to_car_stream(
            vec![cid.clone().into()],
            memo_body_stream(db.clone(), &cid, true),
        );
        let car_reader = StreamReader::new(UnsharedStream::new(Box::pin(car_stream)));

        match kubo_client.syndicate_blocks(car_reader).await {
            Ok(_) => debug!("Syndicated sphere revision {} to IPFS", cid),
            Err(error) => warn!("Failed to syndicate revision {} to IPFS: {:?}", cid, error),
        };
    }

    // At the end, take another lock on the `SphereContext` in order to
    // update the syndication checkpoint for this particular IPFS server
    {
        let mut cursor = SphereCursor::latest(context.clone());
        let (_, bytes) = block_serialize::<DagCborCodec, _>(&SyndicationCheckpoint {
            revision,
            syndicated_blocks,
        })?;

        cursor
            .write(
                &checkpoint_key,
                &ContentType::Cbor,
                Cursor::new(bytes),
                None,
            )
            .await?;

        cursor.save(None).await?;
    }
    Ok(())
}

#[cfg(all(test, feature = "test-kubo"))]
mod tests {
    use anyhow::Result;
    use noosphere_common::helpers::wait;
    use noosphere_core::{
        authority::Access,
        context::{HasMutableSphereContext, HasSphereContext, SphereContentWrite, COUNTERPART},
        data::ContentType,
        helpers::simulated_sphere_context,
        tracing::{initialize_tracing, NoosphereLog},
    };
    use noosphere_ipfs::{IpfsClient, KuboClient};
    use noosphere_storage::KeyValueStore;
    use url::Url;

    use crate::worker::{start_ipfs_syndication, SyndicationJob};

    #[tokio::test(flavor = "multi_thread")]
    async fn it_syndicates_a_sphere_revision_to_kubo() -> Result<()> {
        initialize_tracing(Some(NoosphereLog::Deafening));

        println!("QQQ");

        let (mut user_sphere_context, _) =
            simulated_sphere_context(Access::ReadWrite, None).await?;

        let (mut gateway_sphere_context, _) = simulated_sphere_context(
            Access::ReadWrite,
            Some(user_sphere_context.lock().await.db().clone()),
        )
        .await?;

        let user_sphere_identity = user_sphere_context.identity().await?;

        gateway_sphere_context
            .lock()
            .await
            .db_mut()
            .set_key(COUNTERPART, &user_sphere_identity)
            .await?;

        let ipfs_url = Url::parse("http://127.0.0.1:5001")?;
        let local_kubo_client = KuboClient::new(&ipfs_url.clone())?;

        let (syndication_tx, _syndication_join_handle) = start_ipfs_syndication::<_, _>(ipfs_url);

        user_sphere_context
            .write("foo", &ContentType::Text, b"bar".as_ref(), None)
            .await?;

        user_sphere_context.save(None).await?;

        user_sphere_context
            .write("baz", &ContentType::Text, b"bar".as_ref(), None)
            .await?;

        let version = user_sphere_context.save(None).await?;

        gateway_sphere_context
            .link_raw(&user_sphere_identity, &version)
            .await?;
        gateway_sphere_context.save(None).await?;

        debug!("Sending syndication job...");
        syndication_tx.send(SyndicationJob {
            revision: version.clone(),
            context: gateway_sphere_context.clone(),
        })?;

        debug!("Giving syndication a moment to complete...");

        wait(1).await;

        debug!("Looking for blocks...");

        loop {
            debug!("Sending request to Kubo...");
            if local_kubo_client.get_block(&version).await?.is_some() {
                debug!("Found block!");
                break;
            }

            debug!("No block, retrying in one second...");
            wait(1).await;
        }

        Ok(())
    }
}
