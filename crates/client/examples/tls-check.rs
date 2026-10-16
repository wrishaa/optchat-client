//! Transport qualification without a desktop or credentials. Used by tests/linux-tls.py.
use optchat_client::client::{list_workspaces, EngineCfg, EngineConn};
use optchat_proto::api::HubEvent;
use std::time::Duration;

#[tokio::main]
async fn main() {
    let endpoint = std::env::args().nth(1).expect("HTTPS fixture endpoint");
    assert!(endpoint.starts_with("https://"));
    let cfg = EngineCfg {
        endpoint,
        name: "TLS qualification fixture".into(),
        ..Default::default()
    };
    let https = list_workspaces(&cfg).await.is_ok();
    let (events, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    let connection = EngineConn::start(cfg, None, events);
    let wss = tokio::time::timeout(Duration::from_secs(12), async {
        while let Some((_, _, event)) = receiver.recv().await {
            match event {
                HubEvent::EngineOnline(true) => return true,
                HubEvent::EngineError(_) => return false,
                _ => {}
            }
        }
        false
    })
    .await
    .unwrap_or(false);
    connection.shutdown();
    println!("{}", serde_json::json!({"https": https, "wss": wss}));
}
