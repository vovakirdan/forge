//! Ignored transactional-outbox acceptance against local PostgreSQL and JetStream.

use std::time::Duration;

use anyhow::{Context, Result, bail};
use async_nats::jetstream::{
    consumer::{DeliverPolicy, pull},
    context::Publish,
    stream::Config as StreamConfig,
};
use forge_core::{OutboxError, OutboxPublisher, OutboxPublisherConfig};
use forge_domain::EventId;
use forge_testkit::m0::M0Harness;
use serde_json::Value;
use tokio::{time::sleep, time::timeout};
use tokio_stream::StreamExt;
use uuid::Uuid;

const OUTBOX_STREAM: &str = "FORGE_EVENTS_V1";
const OUTBOX_SUBJECT: &str = "forge.v1.>";

#[tokio::test]
#[ignore = "requires local PostgreSQL and NATS; run `just test-integration`"]
async fn m0_transactional_outbox_publishes_durable_event_once() -> Result<()> {
    let harness = M0Harness::start().await?;
    let project = harness.create_project("M0 transactional outbox").await?;
    let event = harness
        .store
        .list_events(project, None, 10)
        .await?
        .into_iter()
        .next()
        .context("CreateProject did not append a durable event")?;
    let expected_event_id = event.id.to_string();
    let expected_subject = event.subject();
    let canonical_revision = harness
        .store
        .load_project(project)
        .await?
        .context("created Project is missing")?
        .revision();
    let canonical_event_ids = event_ids(&harness, project).await?;
    assert_eq!(outbox_state(&harness, event.id).await?.0, "pending");

    let jetstream = harness.jetstream_context().await?;
    let mut stream = jetstream
        .get_or_create_stream(StreamConfig {
            name: OUTBOX_STREAM.to_owned(),
            subjects: vec![OUTBOX_SUBJECT.to_owned()],
            ..StreamConfig::default()
        })
        .await
        .context("ensure canonical Forge JetStream stream")?;
    if !stream
        .cached_info()
        .config
        .subjects
        .iter()
        .any(|subject| subject == OUTBOX_SUBJECT)
    {
        bail!("Forge JetStream stream does not retain {OUTBOX_SUBJECT}")
    }
    assert!(
        !stream.cached_info().config.duplicate_window.is_zero(),
        "Forge JetStream stream must retain a duplicate window for durable Event IDs"
    );

    let consumer_name = format!("m0_outbox_{}", Uuid::now_v7().simple());
    let consumer = stream
        .create_consumer(pull::Config {
            durable_name: Some(consumer_name.clone()),
            filter_subject: expected_subject.clone(),
            deliver_policy: DeliverPolicy::New,
            ..pull::Config::default()
        })
        .await
        .context("create isolated outbox pull consumer")?;
    let mut messages = consumer
        .fetch()
        .max_messages(1)
        .expires(Duration::from_secs(10))
        .messages()
        .await
        .context("request isolated outbox event")?;
    let publisher = harness.core.outbox_publisher(
        jetstream.clone(),
        OutboxPublisherConfig::new(
            format!("m0-outbox-{}", Uuid::now_v7()),
            100,
            2,
            Duration::from_secs(1),
            Duration::from_millis(100),
            Duration::from_millis(100),
        )?,
    );

    let (_, attempts) = drain_until_published(&harness, &publisher, event.id).await?;
    let message = timeout(Duration::from_secs(10), messages.next())
        .await
        .context("wait for outbox JetStream message")?
        .transpose()
        .map_err(|error| anyhow::anyhow!("receive outbox JetStream message: {error}"))?
        .context("publisher marked outbox event published without delivering it")?;
    let envelope: Value = serde_json::from_slice(&message.message.payload)
        .context("decode published outbox envelope")?;
    assert_eq!(message.message.subject.to_string(), expected_subject);
    assert_eq!(envelope["event_id"], expected_event_id);
    assert_eq!(
        message
            .message
            .headers
            .as_ref()
            .and_then(|headers| headers.get(async_nats::header::NATS_MESSAGE_ID))
            .map(|value| value.as_str()),
        Some(expected_event_id.as_str()),
        "the durable Event ID is the JetStream deduplication key"
    );
    message
        .ack()
        .await
        .map_err(|error| anyhow::anyhow!("ack isolated outbox message: {error}"))?;

    let messages_before_replay = stream.info().await?.state.messages;
    let replay = jetstream
        .send_publish(
            expected_subject,
            Publish::build()
                .payload(message.message.payload)
                .message_id(expected_event_id),
        )
        .await
        .context("send exact durable outbox replay")?
        .await
        .context("await JetStream replay acknowledgement")?;
    assert!(replay.duplicate, "same durable event must be deduplicated");
    assert_eq!(stream.info().await?.state.messages, messages_before_replay);
    assert_eq!(
        outbox_state(&harness, event.id).await?,
        ("published".to_owned(), attempts)
    );
    assert_eq!(
        harness
            .store
            .load_project(project)
            .await?
            .context("Project disappeared during outbox replay")?
            .revision(),
        canonical_revision
    );
    assert_eq!(event_ids(&harness, project).await?, canonical_event_ids);

    stream
        .delete_consumer(&consumer_name)
        .await
        .context("remove isolated outbox consumer")?;
    harness.shutdown().await;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local PostgreSQL and NATS; run `just test-integration`"]
async fn m0_outbox_recovers_after_broker_ack_before_canonical_mark() -> Result<()> {
    let harness = M0Harness::start().await?;
    let project = harness
        .create_project("M0 outbox post-ack recovery")
        .await?;
    let event = harness
        .store
        .list_events(project, None, 10)
        .await?
        .into_iter()
        .next()
        .context("CreateProject did not append a durable event")?;
    let expected_event_id = event.id.to_string();
    let expected_subject = event.subject();
    let canonical_revision = harness
        .store
        .load_project(project)
        .await?
        .context("created Project is missing")?
        .revision();
    let canonical_event_ids = event_ids(&harness, project).await?;

    let jetstream = harness.jetstream_context().await?;
    let stream = jetstream
        .get_or_create_stream(StreamConfig {
            name: OUTBOX_STREAM.to_owned(),
            subjects: vec![OUTBOX_SUBJECT.to_owned()],
            ..StreamConfig::default()
        })
        .await
        .context("ensure canonical Forge JetStream stream")?;
    let consumer_name = format!("m0_outbox_recovery_{}", Uuid::now_v7().simple());
    let consumer = stream
        .create_consumer(pull::Config {
            durable_name: Some(consumer_name.clone()),
            filter_subject: expected_subject,
            deliver_policy: DeliverPolicy::New,
            ..pull::Config::default()
        })
        .await
        .context("create isolated outbox recovery consumer")?;
    let mut first_delivery = consumer
        .fetch()
        .max_messages(1)
        .expires(Duration::from_secs(10))
        .messages()
        .await
        .context("request post-ack recovery Event")?;
    let configuration = outbox_configuration("m0-outbox-post-ack")?;
    let interrupted = harness
        .core
        .outbox_publisher(jetstream.clone(), configuration.clone())
        .fail_after_broker_ack_for(event.id);

    drain_until_post_publish_fault(&interrupted).await?;
    assert_eq!(
        outbox_state(&harness, event.id).await?.0,
        "leased",
        "the test interruption must leave the exact Event leased after PubAck"
    );
    let message = timeout(Duration::from_secs(10), first_delivery.next())
        .await
        .context("wait for broker-acknowledged Event")?
        .transpose()
        .map_err(|error| anyhow::anyhow!("receive broker-acknowledged Event: {error}"))?
        .context("fault was injected before JetStream accepted the Event")?;
    assert_eq!(message.message.subject.to_string(), event.subject());
    assert_eq!(
        message
            .message
            .headers
            .as_ref()
            .and_then(|headers| headers.get(async_nats::header::NATS_MESSAGE_ID))
            .map(|value| value.as_str()),
        Some(expected_event_id.as_str())
    );
    message
        .ack()
        .await
        .map_err(|error| anyhow::anyhow!("ack isolated recovery message: {error}"))?;

    // `OutboxClaimRequest` deliberately has a one-second minimum lease so a
    // separate publisher can prove durable recovery, not in-memory retry.
    sleep(Duration::from_millis(1_200)).await;
    let recovered = harness.core.outbox_publisher(jetstream, configuration);
    let (_, attempts) = drain_until_published(&harness, &recovered, event.id).await?;
    assert!(
        attempts >= 2,
        "recovery must reclaim the expired durable lease"
    );
    assert_eq!(outbox_state(&harness, event.id).await?.0, "published");
    assert_eq!(
        harness
            .store
            .load_project(project)
            .await?
            .context("Project disappeared during post-ack recovery")?
            .revision(),
        canonical_revision
    );
    assert_eq!(event_ids(&harness, project).await?, canonical_event_ids);

    let mut duplicate_delivery = consumer
        .fetch()
        .max_messages(1)
        .expires(Duration::from_millis(500))
        .messages()
        .await
        .context("request possible duplicate delivery")?;
    let duplicate = timeout(Duration::from_secs(2), duplicate_delivery.next()).await;
    assert!(
        !matches!(duplicate, Ok(Some(Ok(_)))),
        "reclaimed outbox Event must retain its JetStream Event-ID dedupe key"
    );

    stream
        .delete_consumer(&consumer_name)
        .await
        .context("remove isolated outbox recovery consumer")?;
    harness.shutdown().await;
    Ok(())
}

async fn drain_until_published(
    harness: &M0Harness,
    publisher: &OutboxPublisher,
    event_id: EventId,
) -> Result<(String, i32)> {
    for _ in 0..30 {
        let _ = publisher.drain_once().await?;
        let state = outbox_state(harness, event_id).await?;
        if state.0 == "published" {
            return Ok(state);
        }
        sleep(Duration::from_millis(15)).await;
    }
    bail!("outbox event was not published after bounded drain attempts")
}

async fn drain_until_post_publish_fault(publisher: &OutboxPublisher) -> Result<()> {
    for _ in 0..30 {
        match publisher.drain_once().await {
            Err(OutboxError::InjectedPostPublishFault) => return Ok(()),
            Ok(_) => {}
            Err(error) => return Err(error.into()),
        }
    }
    bail!("target outbox Event was not reached by the bounded fault-injection drain")
}

fn outbox_configuration(worker_prefix: &str) -> Result<OutboxPublisherConfig> {
    OutboxPublisherConfig::new(
        format!("{worker_prefix}-{}", Uuid::now_v7()),
        100,
        3,
        Duration::from_secs(1),
        Duration::from_millis(100),
        Duration::from_millis(100),
    )
    .map_err(Into::into)
}

async fn outbox_state(harness: &M0Harness, event_id: EventId) -> Result<(String, i32)> {
    sqlx::query_as("SELECT delivery_state, attempt_count FROM outbox WHERE event_id = $1")
        .bind(event_id.as_uuid())
        .fetch_one(&harness.pool)
        .await
        .context("read exact durable outbox row")
}

async fn event_ids(harness: &M0Harness, project: forge_domain::ProjectId) -> Result<Vec<EventId>> {
    Ok(harness
        .store
        .list_events(project, None, 1_000)
        .await?
        .into_iter()
        .map(|event| event.id)
        .collect())
}
