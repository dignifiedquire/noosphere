//! Platform-specific types and bindings
//! Platforms will vary in capabilities for things like block storage and
//! secure key management. This module lays out the concrete strategies we will
//! use on a per-platform basis.

#[cfg(apple)]
mod inner {
    #![allow(missing_docs)]

    use ucan_key_support::ed25519::Ed25519KeyMaterial;

    use crate::key::InsecureKeyStorage;

    // NOTE: This is going to change when we transition to secure key storage
    // This key material type implies insecure storage on disk
    pub type PlatformKeyMaterial = Ed25519KeyMaterial;
    pub type PlatformKeyStorage = InsecureKeyStorage;

    #[cfg(sled)]
    pub(crate) type PrimitiveStorage = noosphere_storage::SledStorage;
    #[cfg(rocksdb)]
    pub(crate) type PrimitiveStorage = noosphere_storage::RocksDbStorage;

    #[cfg(not(ipfs_storage))]
    pub type PlatformStorage = PrimitiveStorage;
    #[cfg(ipfs_storage)]
    pub type PlatformStorage =
        noosphere_ipfs::IpfsStorage<PrimitiveStorage, noosphere_ipfs::KuboClient>;

    #[cfg(test)]
    use anyhow::Result;

    #[cfg(test)]
    use std::path::PathBuf;

    #[cfg(test)]
    use tempfile::TempDir;

    #[cfg(test)]
    pub async fn make_temporary_platform_primitives(
    ) -> Result<(PathBuf, PlatformKeyStorage, (TempDir, TempDir))> {
        let sphere_dir = TempDir::new().unwrap();

        let key_dir = TempDir::new().unwrap();

        let key_storage = InsecureKeyStorage::new(key_dir.path())?;

        Ok((sphere_dir.path().into(), key_storage, (sphere_dir, key_dir)))
    }
}

#[cfg(wasm)]
mod inner {
    #![allow(missing_docs)]

    use crate::key::WebCryptoKeyStorage;

    use std::sync::Arc;
    use ucan_key_support::web_crypto::WebCryptoRsaKeyMaterial;

    pub type PlatformKeyMaterial = Arc<WebCryptoRsaKeyMaterial>;
    pub type PlatformKeyStorage = WebCryptoKeyStorage;

    use noosphere_storage::IndexedDbStorage;

    pub(crate) type PrimitiveStorage = IndexedDbStorage;

    #[cfg(ipfs_storage)]
    pub type PlatformStorage =
        noosphere_ipfs::IpfsStorage<PrimitiveStorage, noosphere_ipfs::GatewayClient>;

    #[cfg(not(ipfs_storage))]
    pub type PlatformStorage = PrimitiveStorage;

    #[cfg(test)]
    use anyhow::Result;

    #[cfg(test)]
    use std::path::PathBuf;

    #[cfg(test)]
    pub async fn make_temporary_platform_primitives() -> Result<(PathBuf, PlatformKeyStorage, ())> {
        let db_name: PathBuf = witty_phrase_generator::WPGen::new()
            .with_words(3)
            .unwrap()
            .into_iter()
            .map(|word| String::from(word))
            .collect();

        let key_storage_name: String = witty_phrase_generator::WPGen::new()
            .with_words(3)
            .unwrap()
            .into_iter()
            .map(|word| String::from(word))
            .collect();

        let key_storage = WebCryptoKeyStorage::new(&key_storage_name).await?;

        Ok((db_name, key_storage, ()))
    }
}

#[cfg(all(native, not(apple)))]
mod inner {
    use crate::key::InsecureKeyStorage;
    use ucan_key_support::ed25519::Ed25519KeyMaterial;

    /// The default key type produced by the [crate::key::KeyStorage]
    /// implementation in use for this platform
    pub type PlatformKeyMaterial = Ed25519KeyMaterial;
    /// The default [crate::key::KeyStorage] in use for this platform
    pub type PlatformKeyStorage = InsecureKeyStorage;

    #[cfg(sled)]
    pub(crate) type PrimitiveStorage = noosphere_storage::SledStorage;
    #[cfg(rocksdb)]
    pub(crate) type PrimitiveStorage = noosphere_storage::RocksDbStorage;

    /// The default backing [noosphere_storage::Storage] in use for this
    /// platform
    #[cfg(not(ipfs_storage))]
    pub type PlatformStorage = PrimitiveStorage;
    #[cfg(ipfs_storage)]
    pub type PlatformStorage =
        noosphere_ipfs::IpfsStorage<PrimitiveStorage, noosphere_ipfs::KuboClient>;

    #[cfg(test)]
    use anyhow::Result;

    #[cfg(test)]
    use std::path::PathBuf;

    #[cfg(test)]
    use tempfile::TempDir;

    #[cfg(test)]
    pub async fn make_temporary_platform_primitives(
    ) -> Result<(PathBuf, PlatformKeyStorage, (TempDir, TempDir))> {
        let sphere_dir = TempDir::new().unwrap();

        let key_dir = TempDir::new().unwrap();

        let key_storage = InsecureKeyStorage::new(key_dir.path())?;

        Ok((sphere_dir.path().into(), key_storage, (sphere_dir, key_dir)))
    }
}

use std::sync::Arc;

pub use inner::*;
use noosphere_core::context::{SphereContext, SphereCursor};
use tokio::sync::Mutex;

use crate::sphere::SphereChannel;

// NOTE: We may someday define the 3rd and 4th terms of this type differently on
// web, where `Arc` and `Mutex` are currently overkill for our needs and may be
// substituted for `Rc` and `RwLock`, respectively.

/// A type alias for the kind of [noosphere_core::context::HasSphereContext] in use
/// by the Noosphere implementation on the current platform
pub type PlatformSphereContext = SphereCursor<Arc<SphereContext<PlatformStorage>>, PlatformStorage>;
/// A type alias for the kind of [noosphere_core::context::HasMutableSphereContext] in use
/// by the Noosphere implementation on the current platform
pub type PlatformMutableSphereContext = Arc<Mutex<SphereContext<PlatformStorage>>>;
/// A type alias for the kind of [SphereChannel] in use by the Noosphere
/// implementation on the current platform
pub type PlatformSphereChannel =
    SphereChannel<PlatformStorage, PlatformSphereContext, PlatformMutableSphereContext>;
