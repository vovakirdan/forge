use forge_provider_common::SecretBytes;
use forge_provider_litellm::{LiteLlmClient, RouteSpec, RunKeySpec, generate_virtual_key};
use reqwest::Client;
use serde_json::{Value, json};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

const MASTER: &str = "sk-forge-contract-master-not-a-real-key";

fn spec(seconds: i64, budget: f64) -> RunKeySpec {
    RunKeySpec {
        run_id: Uuid::now_v7(),
        project_id: Uuid::now_v7(),
        environment_epoch: 1,
        fencing_token: 1,
        execution_profile_id: Uuid::now_v7(),
        execution_profile_revision: 1,
        models: ["forge-contract-stub".to_owned()].into(),
        expires_at: OffsetDateTime::now_utc() + Duration::seconds(seconds),
        requests_per_minute: 100,
        tokens_per_minute: 100000,
        max_budget_usd: Some(budget),
    }
}

#[tokio::test]
#[ignore = "requires dedicated internal-network LiteLLM fixture, never real provider credentials"]
async fn published_litellm_enforces_models_expiry_budget_and_revoke() {
    assert_eq!(std::env::var("FORGE_LITELLM_CONTRACT").as_deref(), Ok("1"));
    let endpoint = "http://127.0.0.1:4007";
    let http = Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .unwrap();
    let client =
        LiteLlmClient::new(endpoint, &SecretBytes::new(MASTER.as_bytes().to_vec())).unwrap();
    let models: Value = http
        .get(format!("{endpoint}/model/info"))
        .bearer_auth(MASTER)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        models["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|model| model["model_name"] == "forge-contract-stub"
                && model["litellm_params"]["api_base"] == "http://stub:4101/v1")
    );

    let route = RouteSpec {
        project_id: Uuid::now_v7(),
        credential_binding_id: Uuid::now_v7(),
        secret_id: Uuid::now_v7(),
        secret_version: 1,
        provider_id: "openai".into(),
        model: "gpt-4o-mini".into(),
    };
    let upstream = SecretBytes::new(b"synthetic-no-network-route-key".to_vec());
    assert!(!client.ensure_route(&route, &upstream).await.unwrap().reused);
    assert!(client.ensure_route(&route, &upstream).await.unwrap().reused);

    let normal = spec(300, 100.0);
    let key = generate_virtual_key().unwrap();
    let issued = client
        .create_or_reconcile(&normal, &key, OffsetDateTime::now_utc())
        .await
        .unwrap();
    assert!(
        client
            .create_or_reconcile(&normal, &key, OffsetDateTime::now_utc())
            .await
            .unwrap()
            .reused
    );
    let completion = |key: &SecretBytes, model: &str| {
        http.post(format!("{endpoint}/v1/chat/completions"))
        .bearer_auth(std::str::from_utf8(key.expose()).unwrap())
        .json(&json!({"model":model,"messages":[{"role":"user","content":"synthetic fixture only"}]}))
    };
    assert!(
        completion(&key, "forge-contract-stub")
            .send()
            .await
            .unwrap()
            .status()
            .is_success()
    );
    assert!(
        !completion(&key, "not-the-allowed-model")
            .send()
            .await
            .unwrap()
            .status()
            .is_success()
    );
    assert!(
        !http
            .get(format!("{endpoint}/model/info"))
            .bearer_auth(std::str::from_utf8(key.expose()).unwrap())
            .send()
            .await
            .unwrap()
            .status()
            .is_success()
    );
    client.revoke(&normal, &issued.key_hash).await.unwrap();
    assert!(
        !completion(&key, "forge-contract-stub")
            .send()
            .await
            .unwrap()
            .status()
            .is_success()
    );

    let short = spec(35, 100.0);
    let expiring = generate_virtual_key().unwrap();
    let issued = client
        .create_or_reconcile(&short, &expiring, OffsetDateTime::now_utc())
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(6)).await;
    assert!(
        !completion(&expiring, "forge-contract-stub")
            .send()
            .await
            .unwrap()
            .status()
            .is_success()
    );
    client.revoke(&short, &issued.key_hash).await.unwrap();

    let budgeted = spec(300, 0.000001);
    let limited = generate_virtual_key().unwrap();
    let issued = client
        .create_or_reconcile(&budgeted, &limited, OffsetDateTime::now_utc())
        .await
        .unwrap();
    assert!(
        completion(&limited, "forge-contract-stub")
            .send()
            .await
            .unwrap()
            .status()
            .is_success()
    );
    let mut observed = false;
    for _ in 0..20 {
        let usage = client
            .usage(&budgeted, &issued.key_hash, OffsetDateTime::now_utc())
            .await
            .unwrap();
        if usage.spend_usd.is_some_and(|spend| spend > 0.000001) {
            observed = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    assert!(
        observed,
        "actual service must report spend before this test claims a budget gate"
    );
    let denied = completion(&limited, "forge-contract-stub")
        .send()
        .await
        .unwrap();
    let status = denied.status();
    assert_eq!(status.as_u16(), 429);
    let denied: Value = denied.json().await.unwrap();
    assert_eq!(denied["error"]["type"], "budget_exceeded");
    assert_eq!(denied["error"]["code"], "429");
    client.revoke(&budgeted, &issued.key_hash).await.unwrap();
}
