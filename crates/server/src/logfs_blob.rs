use std::sync::Arc;

use objstore::{DynObjStore, ObjStoreError, ObjStoreProvider};
use objstore_logfs::{LogFsObjStore, LogFsObjStoreConfig};

/// Injects CLI credentials into the logfs-owned provider's typed configuration.
pub(super) struct LogFsBlobProvider {
    password: Option<String>,
}

impl std::fmt::Debug for LogFsBlobProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LogFsBlobProvider").finish_non_exhaustive()
    }
}

impl LogFsBlobProvider {
    pub fn new(password: Option<String>) -> Self {
        Self { password }
    }
}

impl ObjStoreProvider for LogFsBlobProvider {
    type Config = LogFsObjStoreConfig;

    fn kind(&self) -> &'static str {
        LogFsObjStore::KIND
    }

    fn url_scheme(&self) -> &str {
        "logfs"
    }

    fn build(&self, url: &url::Url) -> Result<DynObjStore, ObjStoreError> {
        let mut config = LogFsObjStoreConfig::from_url(url)?;
        if let Some(password) = self
            .password
            .as_ref()
            .filter(|password| !password.is_empty())
        {
            config.crypto.get_or_insert_default().key = Some(password.clone());
        } else {
            // An empty prompt explicitly selects no encryption, even if a key
            // was supplied in the URI.
            config.crypto = None;
        }
        Ok(Arc::new(LogFsObjStore::new(config)?))
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use objstore::{ObjStore, Put};

    use super::*;

    #[tokio::test]
    async fn prompt_password_overrides_uri_key_and_preserves_profile() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("encrypted.log");
        let uri = url::Url::parse(&format!(
            "logfs://{}?allow_create=true&key=ignored&profile=low-memory",
            path.display()
        ))
        .unwrap();
        let provider = LogFsBlobProvider::new(Some("supplied-password".into()));
        let store = provider.build(&uri).unwrap();
        store
            .send_put(Put::new("item", Bytes::from_static(b"encrypted")))
            .await
            .unwrap();
        drop(store);
        let store = provider.build(&uri).unwrap();
        assert_eq!(
            store.get("item").await.unwrap().unwrap().as_ref(),
            b"encrypted"
        );
        drop(store);
        assert!(
            LogFsBlobProvider::new(Some("ignored".into()))
                .build(&uri)
                .is_err()
        );
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[tokio::test]
    async fn empty_password_disables_uri_crypto_configuration() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plain.log");
        let uri = url::Url::parse(&format!(
            "logfs://{}?allow_create=true&key=ignored",
            path.display()
        ))
        .unwrap();
        let store = LogFsBlobProvider::new(None).build(&uri).unwrap();
        store
            .send_put(Put::new("item", Bytes::from_static(b"plain")))
            .await
            .unwrap();
        drop(store);
        let store = LogFsBlobProvider::new(None)
            .build(&url::Url::parse(&format!("logfs://{}", path.display())).unwrap())
            .unwrap();
        assert_eq!(store.get("item").await.unwrap().unwrap().as_ref(), b"plain");
    }
}
