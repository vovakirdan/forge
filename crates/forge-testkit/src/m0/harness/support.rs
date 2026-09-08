//! Local harness setup and bounded polling helpers.
use super::*;
pub(super) fn required_environment(name: &str) -> Result<String> {
    env::var(name)
        .with_context(|| format!("set {name} through the owner-only Forge dev environment"))
}

pub(super) async fn connect_nats(nats_url: &str) -> Result<async_nats::Client> {
    let address = nats_url
        .parse::<async_nats::ServerAddr>()
        .map_err(|_| anyhow::anyhow!("parse configured NATS JetStream URL"))?;
    let client = match (address.username(), address.password()) {
        (Some(token), None) => {
            async_nats::ConnectOptions::with_token(token.to_owned())
                .connect(nats_url)
                .await
        }
        (Some(user), Some(password)) => {
            async_nats::ConnectOptions::with_user_and_password(user.to_owned(), password.to_owned())
                .connect(nats_url)
                .await
        }
        (None, None) => async_nats::connect(nats_url).await,
        (None, Some(_)) => bail!("configured NATS URL has a password without a username"),
    };
    client.map_err(|_| anyhow::anyhow!("connect local NATS JetStream"))
}

pub(super) fn test_directory() -> Result<PathBuf> {
    let directory = env::temp_dir().join(format!("forge-m0-acceptance-{}", Uuid::now_v7()));
    fs::create_dir(&directory).context("create isolated M0 test socket directory")?;
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
        .context("restrict M0 test socket directory")?;
    Ok(directory)
}

pub(super) async fn wait_for_shutdown(mut receiver: watch::Receiver<bool>) {
    if !*receiver.borrow() {
        let _ = receiver.changed().await;
    }
}

pub(super) async fn wait_until<T, F, Fut>(description: &str, mut check: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<Option<T>>>,
{
    timeout(WAIT_TIMEOUT, async {
        loop {
            if let Some(value) = check().await? {
                return Ok(value);
            }
            sleep(POLL_INTERVAL).await;
        }
    })
    .await
    .with_context(|| format!("wait for {description}"))?
}

pub(super) fn resource_id<T: From<Uuid>>(
    receipt: &CommandReceipt,
    expected_kind: &str,
) -> Result<T> {
    let resource = receipt
        .resource
        .as_ref()
        .context("command returned no resource")?;
    if resource.kind != expected_kind {
        bail!("expected {expected_kind} resource, got {}", resource.kind)
    }
    Ok(T::from(
        Uuid::parse_str(&resource.id).context("parse command resource identity")?,
    ))
}

pub(super) fn retryable_revision_race(error: &CoreError) -> bool {
    matches!(
        error,
        CoreError::Domain(DomainError::StaleProjectRevision { .. })
    )
}
