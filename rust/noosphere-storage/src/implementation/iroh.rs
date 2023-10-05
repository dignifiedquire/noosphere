use crate::{storage::Storage, BlockStore, KeyValueStore, SPHERE_DB_STORE_NAMES};
use anyhow::{anyhow, ensure, Result};
use async_trait::async_trait;
use bytes::Bytes;
use cid::Cid;
use iroh::bytes::{
    baomap::{Map, MapEntry, Store},
    util::BlobFormat,
    Hash,
};
use iroh_io::AsyncSliceReaderExt;
use noosphere_common::ConditionalSend;
use serde::{de::DeserializeOwned, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct IrohStorage {
    rt: iroh::bytes::util::runtime::Handle,
    path: PathBuf,
}

impl IrohStorage {
    pub fn new(path: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&path)?;
        let canonicalized = path.canonicalize()?;
        let rt = iroh::bytes::util::runtime::Handle::from_current(1)?;

        Ok(IrohStorage {
            rt,
            path: canonicalized,
        })
    }
}

#[async_trait]
impl crate::Space for IrohStorage {
    async fn get_space_usage(&self) -> Result<u64> {
        todo!()
    }
}

#[async_trait]
impl Storage for IrohStorage {
    type BlockStore = IrohStore;
    type KeyValueStore = RedbStore;

    async fn get_block_store(&self, name: &str) -> Result<Self::BlockStore> {
        if SPHERE_DB_STORE_NAMES
            .iter()
            .find(|val| **val == name)
            .is_none()
        {
            return Err(anyhow!("No such store named {}", name));
        }

        IrohStore::new(&self.path, &self.rt, name)
    }

    async fn get_key_value_store(&self, name: &str) -> Result<Self::KeyValueStore> {
        if SPHERE_DB_STORE_NAMES
            .iter()
            .find(|val| **val == name)
            .is_none()
        {
            return Err(anyhow!("No such store named {}", name));
        }
        RedbStore::new(name)
    }
}

#[derive(Debug, Clone)]
pub struct IrohStore {
    db: iroh::baomap::flat::Store,
    name: String,
}

impl IrohStore {
    fn new(root: &PathBuf, rt: &iroh::bytes::util::runtime::Handle, name: &str) -> Result<Self> {
        let complete_path = root.join(name).join("complete");
        let partial_path = root.join(name).join("partial");
        let meta_path = root.join(name).join("meta");

        std::fs::create_dir_all(&complete_path)?;
        std::fs::create_dir_all(&partial_path)?;
        std::fs::create_dir_all(&meta_path)?;

        let db =
            iroh::baomap::flat::Store::load_blocking(complete_path, partial_path, meta_path, rt)?;

        Ok(Self {
            db,
            name: name.to_string(),
        })
    }
}

#[async_trait]
impl BlockStore for IrohStore {
    /// Given a block and its [Cid], persist the block in storage.
    async fn put_block(&mut self, cid: &Cid, block: &[u8]) -> Result<()> {
        let expected_hash = Hash::from_cid_bytes(&cid.to_bytes())?;
        let tag = self
            .db
            .import_bytes(Bytes::copy_from_slice(block), BlobFormat::RAW)
            .await?;

        ensure!(tag.hash() == &expected_hash, "hash missmatch");

        Ok(())
    }

    /// Given the [Cid] of a block, retrieve the block bytes from storage.
    async fn get_block(&self, cid: &Cid) -> Result<Option<Vec<u8>>> {
        let hash = Hash::from_cid_bytes(&cid.to_bytes())?;
        match self.db.get(&hash) {
            Some(entry) => {
                let mut reader = entry.data_reader().await?;
                let bytes = reader.read_to_end().await?;
                Ok(Some(bytes.to_vec()))
            }
            None => Ok(None),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RedbStore {
    name: String,
}

impl RedbStore {
    fn new(name: &str) -> Result<Self> {
        Ok(Self {
            name: name.to_string(),
        })
    }
}

#[async_trait]
impl KeyValueStore for RedbStore {
    async fn set_key<K, V>(&mut self, key: K, value: V) -> Result<()>
    where
        K: AsRef<[u8]> + ConditionalSend,
        V: Serialize + ConditionalSend,
    {
        todo!()
    }

    async fn get_key<K, V>(&self, key: K) -> Result<Option<V>>
    where
        K: AsRef<[u8]> + ConditionalSend,
        V: DeserializeOwned + ConditionalSend,
    {
        todo!()
    }

    async fn unset_key<K>(&mut self, key: K) -> Result<()>
    where
        K: AsRef<[u8]> + ConditionalSend,
    {
        todo!()
    }
}
