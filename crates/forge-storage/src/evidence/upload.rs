use std::{sync::Arc, time::Duration};

use forge_domain::EvidenceObject;
use object_store::{
    Attribute, Attributes, ObjectStore, ObjectStoreExt, PutMode, PutOptions, RetryConfig,
    aws::{AmazonS3Builder, S3ConditionalPut},
    path::Path,
};

use super::{EvidenceError, PendingEvidence, digest};

/// Explicit credentials/configuration for the storage adapter. This type does
/// not implement Debug or serialization, so secrets cannot enter diagnostics.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S3EvidenceConfig {
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    /// Explicit opt-in for the loopback MinIO deployment.
    pub allow_http: bool,
}

/// Maintained object_store S3 adapter. It uses conditional creation and never
/// overwrites a previously stored immutable object.
pub struct ObjectEvidenceStore {
    store: Arc<dyn ObjectStore>,
}

impl ObjectEvidenceStore {
    /// Read-only readiness probe against the configured bucket. The reserved
    /// health prefix contains no Task bodies, and no object is created/deleted.
    pub async fn healthcheck(&self) -> bool {
        self.store
            .list_with_delimiter(Some(&Path::from("forge-health-probe/")))
            .await
            .is_ok()
    }
    /// Builds an S3/MinIO client with bounded retries and no implicit ambient
    /// credential lookup. Request signing is implemented by object_store.
    pub fn s3(config: S3EvidenceConfig) -> Result<Self, EvidenceError> {
        if config.access_key_id.is_empty()
            || config.secret_access_key.is_empty()
            || config.bucket.is_empty()
            || config.region.is_empty()
            || config.endpoint.contains(['@', '?', '#'])
            || (!config.endpoint.starts_with("https://")
                && !(config.allow_http && config.endpoint.starts_with("http://")))
        {
            return Err(EvidenceError::InvalidConfiguration);
        }
        let store = AmazonS3Builder::new()
            .with_endpoint(config.endpoint)
            .with_region(config.region)
            .with_bucket_name(config.bucket)
            .with_access_key_id(config.access_key_id)
            .with_secret_access_key(config.secret_access_key)
            .with_allow_http(config.allow_http)
            .with_virtual_hosted_style_request(false)
            .with_conditional_put(S3ConditionalPut::ETagMatch)
            .with_retry(RetryConfig {
                max_retries: 2,
                retry_timeout: Duration::from_secs(10),
                ..RetryConfig::default()
            })
            .build()
            .map_err(|_| EvidenceError::InvalidConfiguration)?;
        Ok(Self::from_store(Arc::new(store)))
    }

    /// Injects a standard ObjectStore implementation for local testing or a
    /// configured object backend. It must honor PutMode::Create.
    pub fn from_store(store: Arc<dyn ObjectStore>) -> Self {
        Self { store }
    }

    /// Confirms remote storage before returning a new Stored receipt. The
    /// caller commits this receipt through Core; failure keeps local evidence.
    pub async fn upload(&self, pending: &PendingEvidence) -> Result<EvidenceObject, EvidenceError> {
        let to_read = pending.clone();
        let bytes = tokio::task::spawn_blocking(move || to_read.read_verified())
            .await
            .map_err(|_| EvidenceError::SpoolIo)??;
        let receipt = pending.receipt();
        let key = Path::parse(receipt.object_key()).map_err(|_| EvidenceError::InvalidMetadata)?;
        let mut attributes = Attributes::new();
        attributes.insert(Attribute::ContentType, "application/octet-stream".into());
        attributes.insert(
            Attribute::Metadata("sha256".into()),
            receipt.data().sha256.clone().into(),
        );
        attributes.insert(
            Attribute::Metadata("redaction-policy".into()),
            receipt.data().redaction_policy_reference.clone().into(),
        );
        let options = PutOptions {
            mode: PutMode::Create,
            attributes,
            ..PutOptions::default()
        };
        let result = tokio::time::timeout(
            Duration::from_secs(15),
            self.store.put_opts(&key, bytes.into(), options),
        )
        .await
        .map_err(|_| EvidenceError::ObjectStoreUnavailable)?;
        match result {
            Ok(_) => {}
            Err(object_store::Error::AlreadyExists { .. }) => {
                self.verify_existing(&key, receipt).await?
            }
            Err(_) => return Err(EvidenceError::ObjectStoreUnavailable),
        }
        receipt.stored().map_err(|_| EvidenceError::InvalidMetadata)
    }

    async fn verify_existing(
        &self,
        key: &Path,
        receipt: &EvidenceObject,
    ) -> Result<(), EvidenceError> {
        tokio::time::timeout(Duration::from_secs(15), async {
            let existing = self
                .store
                .get(key)
                .await
                .map_err(|_| EvidenceError::ObjectStoreUnavailable)?;
            if existing.meta.size != receipt.data().size_bytes {
                return Err(EvidenceError::IntegrityMismatch);
            }
            let bytes = existing
                .bytes()
                .await
                .map_err(|_| EvidenceError::ObjectStoreUnavailable)?;
            if digest(&bytes) != receipt.data().sha256 {
                return Err(EvidenceError::IntegrityMismatch);
            }
            Ok(())
        })
        .await
        .map_err(|_| EvidenceError::ObjectStoreUnavailable)?
    }
}
