//! Disposable local-topology acceptance against PostgreSQL and JetStream.

use anyhow::{Context, Result, bail};
use async_nats::jetstream::{Context as JetStreamContext, stream::Config as StreamConfig};
use forge_testkit::m0::M0Harness;
use sqlx::PgPool;
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires local PostgreSQL and NATS; run `just test-integration`"]
async fn m0_local_topology_creates_and_removes_disposable_schema_and_stream() -> Result<()> {
    let harness = M0Harness::start().await?;
    let result = async {
        let jetstream = harness.jetstream_context().await?;
        let mut resources = DisposableTopology::new(&harness.pool, jetstream);

        let provision_result = resources.provision().await;
        let cleanup_result = resources.cleanup().await;
        let absence_result = if cleanup_result.is_ok() {
            resources.assert_absent().await
        } else {
            Ok(())
        };

        match (provision_result, cleanup_result, absence_result) {
            (Err(error), Err(cleanup_error), _) => Err(error.context(format!(
                "disposable-resource cleanup also failed: {cleanup_error:#}"
            ))),
            (Err(error), _, _) => Err(error),
            (Ok(()), Err(error), _) => Err(error),
            (Ok(()), Ok(()), result) => result,
        }
    }
    .await;
    harness.shutdown().await;
    result
}

struct DisposableTopology<'a> {
    pool: &'a PgPool,
    jetstream: JetStreamContext,
    schema_name: String,
    stream_name: String,
    subject: String,
    schema_created: bool,
    stream_created: bool,
}

impl<'a> DisposableTopology<'a> {
    fn new(pool: &'a PgPool, jetstream: JetStreamContext) -> Self {
        let suffix = Uuid::now_v7().simple().to_string();
        Self {
            pool,
            jetstream,
            schema_name: format!("forge_m0_disposable_{suffix}"),
            stream_name: format!("FORGE_M0_DISPOSABLE_{suffix}"),
            subject: format!("forge.m0.disposable.{suffix}"),
            schema_created: false,
            stream_created: false,
        }
    }

    async fn provision(&mut self) -> Result<()> {
        sqlx::query(&format!("CREATE SCHEMA {}", self.schema_name))
            .execute(self.pool)
            .await
            .context("create disposable PostgreSQL schema")?;
        self.schema_created = true;

        self.jetstream
            .create_stream(StreamConfig {
                name: self.stream_name.clone(),
                subjects: vec![self.subject.clone()],
                ..StreamConfig::default()
            })
            .await
            .context("create disposable JetStream stream")?;
        self.stream_created = true;

        self.assert_present().await
    }

    async fn assert_present(&self) -> Result<()> {
        let schema_present = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM information_schema.schemata WHERE schema_name = $1)",
        )
        .bind(&self.schema_name)
        .fetch_one(self.pool)
        .await
        .context("query disposable PostgreSQL schema")?;
        if !schema_present {
            bail!("disposable PostgreSQL schema was not created");
        }

        self.jetstream
            .get_stream(&self.stream_name)
            .await
            .context("read disposable JetStream stream")?;
        Ok(())
    }

    async fn cleanup(&mut self) -> Result<()> {
        let stream_result = self.remove_stream().await;
        let schema_result = self.remove_schema().await;

        match (stream_result, schema_result) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
            (Err(stream_error), Err(schema_error)) => Err(stream_error.context(format!(
                "disposable PostgreSQL schema cleanup also failed: {schema_error:#}"
            ))),
        }
    }

    async fn assert_absent(&self) -> Result<()> {
        let schema_present = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM information_schema.schemata WHERE schema_name = $1)",
        )
        .bind(&self.schema_name)
        .fetch_one(self.pool)
        .await
        .context("query removed disposable PostgreSQL schema")?;
        if schema_present {
            bail!("disposable PostgreSQL schema was not removed");
        }
        if self.jetstream.get_stream(&self.stream_name).await.is_ok() {
            bail!("disposable JetStream stream was not removed");
        }
        Ok(())
    }

    async fn remove_stream(&mut self) -> Result<()> {
        if self.stream_created {
            self.jetstream
                .delete_stream(&self.stream_name)
                .await
                .context("remove disposable JetStream stream")?;
            self.stream_created = false;
        }
        Ok(())
    }

    async fn remove_schema(&mut self) -> Result<()> {
        if self.schema_created {
            sqlx::query(&format!(
                "DROP SCHEMA IF EXISTS {} CASCADE",
                self.schema_name
            ))
            .execute(self.pool)
            .await
            .context("remove disposable PostgreSQL schema")?;
            self.schema_created = false;
        }
        Ok(())
    }
}
