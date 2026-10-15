//! Lan-first, live on this Mac (run by tests/lan/run-live.sh, which starts a fake engine as the "tunnel" and a lan
//! front instance in front of it, advertised over bonjour). Checks: the socket moves to lan by itself, the engine
//! answers there, a planned switch each way is make-before-break, a frozen front (heartbeat miss) and a killed front
//! (socket closed) fall back to the tunnel, and lan is taken again when the front is back. Every ping is answered
//! exactly once.
#![cfg(any(target_os = "macos", target_os = "ios"))]
use optchat_client::client::{EngineCfg, EngineConn};
use optchat_client::lan::PathKind;
use optchat_platform::{BonjourEvent, NetPath, Platform};
use std::sync::Arc;
use std::time::{Duration, Instant};

struct Mac;
impl Platform for Mac {
    fn browse(&self, service: &str, on: Arc<dyn Fn(BonjourEvent) + Send + Sync>) -> Option<Box<dyn Send + Sync>> {
        optchat_platform::apple_net::browse(service, on).map(|h| Box::new(h) as Box<dyn Send + Sync>)
    }
    fn watch_net(&self, on: Arc<dyn Fn(NetPath) + Send + Sync>) -> bool {
        optchat_platform::apple_net::watch_net(on)
    }
    fn log(&self, line: &str) {
        let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis();
        eprintln!("{ms} {line}");
    }
}

async fn wait_path(c: &EngineConn, p: PathKind, within: Duration) -> Option<f64> {
    let t = Instant::now();
    while t.elapsed() < within {
        if c.path() == Some(p) {
            return Some(t.elapsed().as_secs_f64() * 1000.0);
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    None
}
fn sig(s: &str) {
    let pid = std::env::var("LAN_SIDECAR_PID").expect("LAN_SIDECAR_PID");
    std::process::Command::new("kill").args([s, &pid]).status().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn lan_first_live() {
    optchat_platform::install(Arc::new(Mac));
    let url = std::env::var("LAN_TUNNEL_URL").unwrap_or("http://127.0.0.1:18791".into());
    let token = std::env::var("LAN_TEST_TOKEN").expect("LAN_TEST_TOKEN (a fake engine's token)");
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let snaps = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let s2 = snaps.clone();
    tokio::spawn(async move {
        while let Some((_, _, e)) = rx.recv().await {
            if matches!(e, optchat_proto::api::HubEvent::Snapshot(_)) {
                s2.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
        }
    });
    let cfg = EngineCfg { endpoint: url.clone(), name: "fake".into(), urls: vec![url.clone()], auto: true };
    let c = EngineConn::start(cfg, Some(token.clone()), tx);
    c.connect_ws("alpha", 0, &token);

    // 1. found by bonjour → on lan by itself
    let t = wait_path(&c, PathKind::Lan, Duration::from_secs(15)).await.expect("never moved to lan");
    eprintln!("on lan {t:.0} ms after start; snapshots so far {}", snaps.load(std::sync::atomic::Ordering::SeqCst));
    let rtt = c.ping().await.expect("engine answers on lan");
    eprintln!("engine round trip on lan {rtt:.1} ms");
    let r = c.lan_get("/engine/workspaces").await.expect("GET over lan");
    assert_eq!(r.status, 200);

    // 2. planned switches, make-before-break, with pings in flight
    let c2 = c.clone();
    let pinger = tokio::spawn(async move {
        let mut fails = 0;
        for _ in 0..200 {
            if c2.ping().await.is_err() {
                fails += 1;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        fails
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    c.set_lan_disabled(true);
    let t = wait_path(&c, PathKind::Tunnel, Duration::from_secs(10)).await.expect("planned lan → tunnel");
    eprintln!("planned lan -> tunnel {t:.0} ms");
    c.set_lan_disabled(false);
    let t = wait_path(&c, PathKind::Lan, Duration::from_secs(10)).await.expect("planned tunnel → lan");
    eprintln!("planned tunnel -> lan {t:.0} ms");
    let fails = pinger.await.unwrap();
    eprintln!("pings during planned switches: {:?}, failed {fails}", c.ping_stats.lock());
    assert_eq!(fails, 0, "a ping was lost during a make-before-break switch");
    assert_eq!(c.ping_stats.lock().dup, 0);

    // 3. the front freezes (silent loss): heartbeat miss → tunnel
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let t0 = Instant::now();
    sig("-STOP");
    let t = wait_path(&c, PathKind::Tunnel, Duration::from_secs(20)).await;
    sig("-CONT");
    eprintln!("frozen front -> tunnel after {:.0} ms", t0.elapsed().as_secs_f64() * 1000.0);
    assert!(t.is_some(), "no fallback on a frozen front");
    // back to lan once the retry backoff (OPTCHAT_LAN_RETRY_S) passes
    let t = wait_path(&c, PathKind::Lan, Duration::from_secs(20)).await.expect("back to lan after the freeze");
    eprintln!("lan again {t:.0} ms after thaw");

    // 4. the front dies (socket closed) → tunnel at once
    tokio::time::sleep(Duration::from_millis(1000)).await;
    let t0 = Instant::now();
    sig("-KILL");
    wait_path(&c, PathKind::Tunnel, Duration::from_secs(10)).await.expect("no fallback on a dead front");
    eprintln!("dead front -> tunnel after {:.0} ms", t0.elapsed().as_secs_f64() * 1000.0);
    let rtt = c.ping().await.expect("engine answers on the tunnel");
    eprintln!("engine round trip on the tunnel {rtt:.1} ms; pings {:?}", c.ping_stats.lock());
    assert_eq!(c.ping_stats.lock().dup, 0);
}
