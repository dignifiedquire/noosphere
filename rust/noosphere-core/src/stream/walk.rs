use crate::{
    data::{MapOperation, VersionedMapKey, VersionedMapValue},
    view::VersionedMap,
};
use anyhow::Result;
use noosphere_storage::BlockStore;
use std::ops::Fn;
use tokio_stream::StreamExt;

/// Given a [VersionedMap], visit its changelog and all of its underlying entries
pub async fn walk_versioned_map_elements<K, V, S>(
    versioned_map: VersionedMap<K, V, S>,
) -> Result<()>
where
    K: VersionedMapKey + 'static,
    V: VersionedMapValue + 'static,
    S: BlockStore + 'static,
{
    versioned_map.get_changelog().await?;
    let stream = versioned_map.into_stream().await?;
    tokio::pin!(stream);
    while (stream.try_next().await?).is_some() {}
    Ok(())
}

/// Given a [VersionedMap] and [BlockStore], visit the [VersionedMap]'s
/// changelog and all of its underlying entries, invoking a callback for each
/// entry
pub async fn walk_versioned_map_elements_and<K, V, S, F, Fut>(
    versioned_map: VersionedMap<K, V, S>,
    store: S,
    callback: F,
) -> Result<()>
where
    K: VersionedMapKey + 'static,
    V: VersionedMapValue + 'static,
    S: BlockStore + 'static,
    Fut: std::future::Future<Output = Result<()>>,
    F: 'static + Fn(K, V, S) -> Fut,
{
    versioned_map.get_changelog().await?;
    let stream = versioned_map.into_stream().await?;
    tokio::pin!(stream);
    while let Some((key, value)) = stream.try_next().await? {
        callback(key, value, store.clone()).await?;
    }
    Ok(())
}

/// Given a [VersionedMap] and [BlockStore], visit the [VersionedMap]'s
/// changelog; then, invoke the provided callback with each entry associated
/// with an 'add' operation in the changelog
pub async fn walk_versioned_map_changes_and<K, V, S, F, Fut>(
    versioned_map: VersionedMap<K, V, S>,
    store: S,
    callback: F,
) -> Result<()>
where
    K: VersionedMapKey + 'static,
    V: VersionedMapValue + 'static,
    S: BlockStore + 'static,
    Fut: std::future::Future<Output = Result<()>>,
    F: 'static + Fn(K, V, S) -> Fut,
{
    let changelog = versioned_map.load_changelog().await?;
    for op in changelog.changes {
        if let MapOperation::Add { key, value } = op {
            callback(key, value, store.clone()).await?;
        }
    }
    Ok(())
}
