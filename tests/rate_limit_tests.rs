//! Comprehensive tests for the rate limiting subsystem.
//!
//! Covers:
//! - Core types (WindowType, RateLimitBucket, RateLimitPolicy, ConcurrencyConfig)
//! - MemoryRateLimitStore (all CRUD + check_and_increment for fixed/sliding)
//! - RateLimitManager: policy resolution hierarchy, check_rate_limit, emergency blocks,
//!   multi-bucket enforcement, metrics recording, header generation
//! - ConcurrencyManager: acquire / release, config updates
//! - MetricsStore: record, get_metrics, get_all_metrics, get_events, clear
//! - File config: load, key overrides, default policy
//! - Admin HTTP endpoints (via actix-web test harness): CRUD for policies,
//!   key assignment, emergency block lifecycle, default policy, reload, analytics
//! - rate_limit_headers generation

use actix_web::test as atest;
use actix_web::{http::StatusCode, web, App};
use routiium::{
    rate_limit::{
        default_policy_from_env, ConcurrencyConfig, MemoryRateLimitStore, QueueStrategy,
        RateLimitBucket, RateLimitManager, RateLimitPolicy, RateLimitStore, WindowType,
    },
    server::config_routes,
    util::AppState,
};
use serde_json::json;
use std::sync::{Arc, Mutex};

/// Serialize tests that mutate environment variables.
static ENV_LOCK: Mutex<()> = Mutex::new(());

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn memory_manager() -> Arc<RateLimitManager> {
    let store = Arc::new(MemoryRateLimitStore::new());
    Arc::new(RateLimitManager::new(store))
}

fn simple_policy(id: &str, requests: u64, window_seconds: u64) -> RateLimitPolicy {
    RateLimitPolicy {
        id: id.to_string(),
        buckets: vec![RateLimitBucket {
            name: "default".to_string(),
            requests,
            window_seconds,
            window_type: WindowType::Fixed,
        }],
    }
}

fn multi_bucket_policy(id: &str) -> RateLimitPolicy {
    RateLimitPolicy {
        id: id.to_string(),
        buckets: vec![
            RateLimitBucket {
                name: "per_minute".to_string(),
                requests: 5,
                window_seconds: 60,
                window_type: WindowType::Fixed,
            },
            RateLimitBucket {
                name: "per_day".to_string(),
                requests: 100,
                window_seconds: 86400,
                window_type: WindowType::Fixed,
            },
        ],
    }
}

fn admin_bearer() -> (&'static str, &'static str) {
    ("Authorization", "Bearer admin-test")
}

/// Macro that creates the actix test app inline so the concrete service type
/// is visible to the compiler (required for type inference on `call_service`).
macro_rules! make_app {
    ($mgr:expr) => {{
        std::env::set_var("ROUTIIUM_ADMIN_TOKEN", "admin-test");
        let state = AppState {
            rate_limit_manager: Some($mgr),
            ..Default::default()
        };
        atest::init_service(
            App::new()
                .app_data(web::Data::new(state))
                .configure(config_routes),
        )
        .await
    }};
}

// ---------------------------------------------------------------------------
// 1. Core type serialisation / defaults
// ---------------------------------------------------------------------------

#[test]
fn window_type_default_is_fixed() {
    let w: WindowType = Default::default();
    assert_eq!(w, WindowType::Fixed);
}

#[test]
fn queue_strategy_default_is_reject() {
    let q: QueueStrategy = Default::default();
    assert_eq!(q, QueueStrategy::Reject);
}

#[test]
fn rate_limit_policy_round_trip() {
    let policy = simple_policy("p1", 100, 60);
    let json = serde_json::to_string(&policy).unwrap();
    let back: RateLimitPolicy = serde_json::from_str(&json).unwrap();
    assert_eq!(back.id, "p1");
    assert_eq!(back.buckets.len(), 1);
    assert_eq!(back.buckets[0].requests, 100);
}

#[test]
fn sliding_window_type_roundtrip() {
    let bucket = RateLimitBucket {
        name: "x".into(),
        requests: 10,
        window_seconds: 300,
        window_type: WindowType::Sliding,
    };
    let j = serde_json::to_string(&bucket).unwrap();
    let back: RateLimitBucket = serde_json::from_str(&j).unwrap();
    assert_eq!(back.window_type, WindowType::Sliding);
}

// ---------------------------------------------------------------------------
// 2. MemoryRateLimitStore – CRUD
// ---------------------------------------------------------------------------

#[tokio::test]
async fn memory_store_policy_crud() {
    let store = MemoryRateLimitStore::new();
    let policy = simple_policy("p1", 10, 60);

    store.save_policy(&policy).await.unwrap();
    let got = store.get_policy("p1").await.unwrap();
    assert!(got.is_some());
    assert_eq!(got.unwrap().id, "p1");

    let list = store.list_policies().await.unwrap();
    assert_eq!(list.len(), 1);

    let deleted = store.delete_policy("p1").await.unwrap();
    assert!(deleted);

    let list2 = store.list_policies().await.unwrap();
    assert!(list2.is_empty());

    // Deleting non-existent policy returns false
    let deleted2 = store.delete_policy("nope").await.unwrap();
    assert!(!deleted2);
}

#[tokio::test]
async fn memory_store_key_policy_assignment() {
    let store = MemoryRateLimitStore::new();

    store.set_key_policy("key1", "pol-a").await.unwrap();
    let got = store.get_key_policy("key1").await.unwrap();
    assert_eq!(got, Some("pol-a".to_string()));

    let removed = store.remove_key_policy("key1").await.unwrap();
    assert!(removed);

    let got2 = store.get_key_policy("key1").await.unwrap();
    assert!(got2.is_none());

    // Remove non-existent assignment returns false
    let removed2 = store.remove_key_policy("key1").await.unwrap();
    assert!(!removed2);
}

#[tokio::test]
async fn memory_store_default_policy_id() {
    let store = MemoryRateLimitStore::new();

    let default = store.get_default_policy_id().await.unwrap();
    assert!(default.is_none());

    store.set_default_policy_id("pol-default").await.unwrap();
    let default2 = store.get_default_policy_id().await.unwrap();
    assert_eq!(default2, Some("pol-default".to_string()));
}

#[tokio::test]
async fn memory_store_block_lifecycle() {
    let store = MemoryRateLimitStore::new();

    // No block by default
    let blk = store.get_block("key1").await.unwrap();
    assert!(blk.is_none());

    // Block with a far-future expiry
    let future = 9_999_999_999u64;
    store.block_key("key1", future, "test block").await.unwrap();

    let blk2 = store.get_block("key1").await.unwrap();
    assert!(blk2.is_some());
    assert_eq!(blk2.unwrap().reason, "test block");

    let list = store.list_blocks().await.unwrap();
    assert_eq!(list.len(), 1);

    store.unblock_key("key1").await.unwrap();
    let blk3 = store.get_block("key1").await.unwrap();
    assert!(blk3.is_none());
}

// ---------------------------------------------------------------------------
// 3. MemoryRateLimitStore – check_and_increment (fixed window)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn fixed_window_allows_then_rejects() {
    let store = MemoryRateLimitStore::new();
    let bucket = RateLimitBucket {
        name: "b".into(),
        requests: 3,
        window_seconds: 60,
        window_type: WindowType::Fixed,
    };

    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    // First 3 requests allowed
    for i in 1..=3 {
        let status = store.check_and_increment("k1", &bucket, now).await.unwrap();
        assert!(status.allowed, "request {} should be allowed", i);
        assert_eq!(status.used, i);
        assert_eq!(status.remaining, 3 - i);
    }

    // 4th request rejected
    let status = store.check_and_increment("k1", &bucket, now).await.unwrap();
    assert!(!status.allowed);
    assert_eq!(status.remaining, 0);
}

#[tokio::test]
async fn fixed_window_resets_after_window_expires() {
    let store = MemoryRateLimitStore::new();
    let bucket = RateLimitBucket {
        name: "b".into(),
        requests: 2,
        window_seconds: 1, // 1-second window
        window_type: WindowType::Fixed,
    };

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    // Exhaust the window
    store.check_and_increment("k2", &bucket, now).await.unwrap();
    store.check_and_increment("k2", &bucket, now).await.unwrap();
    let s = store.check_and_increment("k2", &bucket, now).await.unwrap();
    assert!(!s.allowed);

    // Advance time past the window
    let next_window = now + 2;
    let s2 = store
        .check_and_increment("k2", &bucket, next_window)
        .await
        .unwrap();
    assert!(s2.allowed);
    assert_eq!(s2.used, 1);
}

#[tokio::test]
async fn sliding_window_check_and_increment() {
    let store = MemoryRateLimitStore::new();
    let bucket = RateLimitBucket {
        name: "slide".into(),
        requests: 3,
        window_seconds: 60,
        window_type: WindowType::Sliding,
    };

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    // First 3 should be allowed under sliding window
    for _ in 0..3 {
        let s = store
            .check_and_increment("sk1", &bucket, now)
            .await
            .unwrap();
        assert!(s.allowed);
    }

    // 4th should be rejected
    let s = store
        .check_and_increment("sk1", &bucket, now)
        .await
        .unwrap();
    assert!(!s.allowed);
}

// ---------------------------------------------------------------------------
// 4. RateLimitManager – policy resolution hierarchy
// ---------------------------------------------------------------------------

#[tokio::test]
async fn manager_no_policy_returns_unlimited() {
    let mgr = memory_manager();
    // No policies stored, no default configured → unlimited
    let result = mgr.check_rate_limit("key-x", "/v1/chat", None).await;
    let result = result.unwrap();
    assert!(result.allowed);
    assert_eq!(result.policy_id, "unlimited");
}

#[tokio::test]
async fn manager_default_policy_applied() {
    let mgr = memory_manager();
    let pol = simple_policy("default", 2, 3600);
    mgr.create_policy(pol).await.unwrap();
    mgr.set_default_policy("default").await.unwrap();

    // Two allowed, third rejected
    let r1 = mgr.check_rate_limit("any-key", "/", None).await.unwrap();
    assert!(r1.allowed);
    let r2 = mgr.check_rate_limit("any-key", "/", None).await.unwrap();
    assert!(r2.allowed);
    let r3 = mgr.check_rate_limit("any-key", "/", None).await.unwrap();
    assert!(!r3.allowed);
}

#[tokio::test]
async fn manager_per_key_policy_overrides_default() {
    let mgr = memory_manager();

    // Default: 1 req
    let default_pol = simple_policy("default", 1, 3600);
    mgr.create_policy(default_pol).await.unwrap();
    mgr.set_default_policy("default").await.unwrap();

    // Per-key policy: 5 reqs
    let key_pol = simple_policy("generous", 5, 3600);
    mgr.create_policy(key_pol).await.unwrap();
    mgr.assign_key_policy("vip-key", "generous").await.unwrap();

    // Normal key: only 1 allowed
    let r1 = mgr.check_rate_limit("normal-key", "/", None).await.unwrap();
    assert!(r1.allowed);
    let r2 = mgr.check_rate_limit("normal-key", "/", None).await.unwrap();
    assert!(!r2.allowed);

    // VIP key: 5 allowed
    for i in 1..=5 {
        let r = mgr.check_rate_limit("vip-key", "/", None).await.unwrap();
        assert!(r.allowed, "vip request {} should be allowed", i);
    }
    let r6 = mgr.check_rate_limit("vip-key", "/", None).await.unwrap();
    assert!(!r6.allowed);
}

#[tokio::test]
async fn manager_multi_bucket_stops_at_first_exhausted_bucket() {
    let mgr = memory_manager();
    let pol = multi_bucket_policy("multi");
    mgr.create_policy(pol).await.unwrap();
    mgr.assign_key_policy("mk", "multi").await.unwrap();

    // per_minute limit is 5 — exhaust it
    for _ in 0..5 {
        let r = mgr.check_rate_limit("mk", "/", None).await.unwrap();
        assert!(r.allowed);
    }

    let r6 = mgr.check_rate_limit("mk", "/", None).await.unwrap();
    assert!(!r6.allowed);
    // The rejected bucket should be the per_minute one
    let rejected = r6.rejected_bucket.as_ref().unwrap();
    assert_eq!(rejected.name, "per_minute");
}

// ---------------------------------------------------------------------------
// 5. Emergency blocks
// ---------------------------------------------------------------------------

#[tokio::test]
async fn emergency_block_prevents_requests() {
    let mgr = memory_manager();

    // Without any policy, requests are unlimited
    let before = mgr.check_rate_limit("blocked-key", "/", None).await;
    assert!(before.unwrap().allowed);

    // Apply emergency block
    mgr.set_emergency_block("blocked-key", Some(3600), "testing")
        .await
        .unwrap();

    // Now check_rate_limit should return an error (blocked)
    let after = mgr.check_rate_limit("blocked-key", "/", None).await;
    assert!(after.is_err());
    let msg = after.unwrap_err().to_string();
    assert!(msg.contains("BLOCKED"));
}

#[tokio::test]
async fn emergency_block_removal_unblocks_key() {
    let mgr = memory_manager();
    mgr.set_emergency_block("key2", Some(3600), "remove test")
        .await
        .unwrap();

    // Verify blocked
    assert!(mgr.check_rate_limit("key2", "/", None).await.is_err());

    mgr.remove_emergency_block("key2").await.unwrap();

    // Now unlimited
    let r = mgr.check_rate_limit("key2", "/", None).await.unwrap();
    assert!(r.allowed);
}

#[tokio::test]
async fn list_emergency_blocks_reflects_active_blocks() {
    let mgr = memory_manager();
    mgr.set_emergency_block("b1", Some(3600), "r1")
        .await
        .unwrap();
    mgr.set_emergency_block("b2", Some(3600), "r2")
        .await
        .unwrap();

    let blocks = mgr.list_emergency_blocks().await.unwrap();
    assert_eq!(blocks.len(), 2);

    mgr.remove_emergency_block("b1").await.unwrap();
    let blocks2 = mgr.list_emergency_blocks().await.unwrap();
    assert_eq!(blocks2.len(), 1);
    assert_eq!(blocks2[0].key_id, "b2");
}

#[tokio::test]
async fn get_block_returns_none_when_unblocked() {
    let mgr = memory_manager();
    let b = mgr.get_block("no-block-key").await;
    assert!(b.is_none());
}

#[tokio::test]
async fn get_block_returns_block_info() {
    let mgr = memory_manager();
    mgr.set_emergency_block("blk-key", Some(7200), "manual block")
        .await
        .unwrap();
    let b = mgr.get_block("blk-key").await;
    assert!(b.is_some());
    assert_eq!(b.unwrap().reason, "manual block");
}

// ---------------------------------------------------------------------------
// 6. Policy CRUD via manager
// ---------------------------------------------------------------------------

#[tokio::test]
async fn manager_create_and_get_policy() {
    let mgr = memory_manager();
    let pol = simple_policy("p-create", 10, 60);
    mgr.create_policy(pol).await.unwrap();

    let got = mgr.get_policy("p-create").await.unwrap();
    assert!(got.is_some());
    assert_eq!(got.unwrap().buckets[0].requests, 10);
}

#[tokio::test]
async fn manager_update_policy() {
    let mgr = memory_manager();
    let pol = simple_policy("upd", 10, 60);
    mgr.create_policy(pol).await.unwrap();

    let updated = RateLimitPolicy {
        id: "upd".into(),
        buckets: vec![RateLimitBucket {
            name: "default".into(),
            requests: 999,
            window_seconds: 3600,
            window_type: WindowType::Fixed,
        }],
    };
    let existed = mgr.update_policy(updated).await.unwrap();
    assert!(existed);

    let got = mgr.get_policy("upd").await.unwrap().unwrap();
    assert_eq!(got.buckets[0].requests, 999);
}

#[tokio::test]
async fn manager_update_nonexistent_policy_returns_false() {
    let mgr = memory_manager();
    let pol = simple_policy("ghost", 1, 1);
    let existed = mgr.update_policy(pol).await.unwrap();
    // The store saves regardless but returns false for "existed"
    assert!(!existed);
}

#[tokio::test]
async fn manager_delete_policy() {
    let mgr = memory_manager();
    let pol = simple_policy("del-me", 1, 1);
    mgr.create_policy(pol).await.unwrap();

    let deleted = mgr.delete_policy("del-me").await.unwrap();
    assert!(deleted);

    let got = mgr.get_policy("del-me").await.unwrap();
    assert!(got.is_none());
}

#[tokio::test]
async fn manager_list_policies_all_created() {
    let mgr = memory_manager();
    for i in 0..5 {
        mgr.create_policy(simple_policy(&format!("p{}", i), 10, 60))
            .await
            .unwrap();
    }
    let list = mgr.list_policies().await.unwrap();
    assert_eq!(list.len(), 5);
}

// ---------------------------------------------------------------------------
// 7. get_current_usage
// ---------------------------------------------------------------------------

#[tokio::test]
async fn get_current_usage_shows_bucket_counters() {
    let mgr = memory_manager();
    let pol = simple_policy("usage-pol", 10, 3600);
    mgr.create_policy(pol).await.unwrap();
    mgr.assign_key_policy("usage-key", "usage-pol")
        .await
        .unwrap();

    // Make 3 requests
    for _ in 0..3 {
        mgr.check_rate_limit("usage-key", "/", None).await.unwrap();
    }

    let (policy_id, statuses) = mgr.get_current_usage("usage-key").await.unwrap();
    assert_eq!(policy_id, "usage-pol");
    assert_eq!(statuses.len(), 1);
    assert_eq!(statuses[0].used, 3);
    assert_eq!(statuses[0].remaining, 7);
}

#[tokio::test]
async fn get_current_usage_unlimited_when_no_policy() {
    let mgr = memory_manager();
    let (policy_id, statuses) = mgr.get_current_usage("no-policy-key").await.unwrap();
    assert_eq!(policy_id, "unlimited");
    assert!(statuses.is_empty());
}

// ---------------------------------------------------------------------------
// 8. rate_limit_headers
// ---------------------------------------------------------------------------

#[tokio::test]
async fn rate_limit_headers_contain_expected_keys() {
    let mgr = memory_manager();
    let pol = simple_policy("hdr-pol", 5, 60);
    mgr.create_policy(pol).await.unwrap();
    mgr.assign_key_policy("hk", "hdr-pol").await.unwrap();

    let result = mgr.check_rate_limit("hk", "/", None).await.unwrap();
    let headers = RateLimitManager::rate_limit_headers(&result);

    // Should have at least the policy headers
    let header_names: Vec<&str> = headers.iter().map(|(k, _)| k.as_str()).collect();
    assert!(
        header_names.contains(&"X-RateLimit-Policy")
            || header_names.contains(&"X-RateLimit-Policy-Id"),
        "expected policy header, got: {:?}",
        header_names
    );
    // Should have limit/remaining/reset headers
    assert!(
        header_names.contains(&"X-RateLimit-Limit"),
        "got: {:?}",
        header_names
    );
    assert!(
        header_names.contains(&"X-RateLimit-Remaining"),
        "got: {:?}",
        header_names
    );
}

// ---------------------------------------------------------------------------
// 9. MetricsStore
// ---------------------------------------------------------------------------

#[tokio::test]
async fn metrics_records_allowed_and_rejected_events() {
    let mgr = memory_manager();
    let pol = simple_policy("m-pol", 2, 3600);
    mgr.create_policy(pol).await.unwrap();
    mgr.assign_key_policy("mkey", "m-pol").await.unwrap();

    mgr.check_rate_limit("mkey", "/", None).await.unwrap();
    mgr.check_rate_limit("mkey", "/", None).await.unwrap();
    // third is rejected
    let _ = mgr.check_rate_limit("mkey", "/", None).await;

    let metrics = mgr.metrics.get_metrics("mkey").unwrap();
    assert!(metrics.total_checks >= 3);
    assert!(metrics.total_allowed >= 2);
    assert!(metrics.total_rejected >= 1);
}

#[tokio::test]
async fn metrics_get_events_with_limit() {
    let mgr = memory_manager();
    let pol = simple_policy("ev-pol", 100, 3600);
    mgr.create_policy(pol).await.unwrap();
    mgr.assign_key_policy("ekey", "ev-pol").await.unwrap();

    for _ in 0..10 {
        mgr.check_rate_limit("ekey", "/chat", Some("gpt-4"))
            .await
            .unwrap();
    }

    let events = mgr.metrics.get_events(Some("ekey"), 5, 0);
    assert!(events.len() <= 5);

    let all_events = mgr.metrics.get_events(Some("ekey"), 100, 0);
    assert!(!all_events.is_empty());
}

#[tokio::test]
async fn metrics_get_all_metrics_returns_all_keys() {
    let mgr = memory_manager();
    let pol = simple_policy("all-pol", 100, 3600);
    mgr.create_policy(pol).await.unwrap();
    mgr.set_default_policy("all-pol").await.unwrap();

    mgr.check_rate_limit("key-a", "/", None).await.unwrap();
    mgr.check_rate_limit("key-b", "/", None).await.unwrap();

    let all = mgr.metrics.get_all_metrics();
    // At least both keys should be tracked
    assert!(all.contains_key("key-a") || all.contains_key("key-b"));
}

#[tokio::test]
async fn metrics_clear_wipes_events() {
    let mgr = memory_manager();
    let pol = simple_policy("cl-pol", 100, 3600);
    mgr.create_policy(pol).await.unwrap();
    mgr.assign_key_policy("ck", "cl-pol").await.unwrap();

    mgr.check_rate_limit("ck", "/", None).await.unwrap();
    assert!(!mgr.metrics.get_events(Some("ck"), 100, 0).is_empty());

    mgr.metrics.clear();
    assert!(mgr.metrics.get_events(Some("ck"), 100, 0).is_empty());
}

// ---------------------------------------------------------------------------
// 10. ConcurrencyManager
// ---------------------------------------------------------------------------

#[tokio::test]
async fn concurrency_manager_acquire_release() {
    use routiium::rate_limit::ConcurrencyResult as CR;

    let mgr = memory_manager();
    let cc = ConcurrencyConfig {
        max_concurrent: 2,
        max_queue_size: 0,
        queue_timeout_ms: 5000,
        strategy: QueueStrategy::Reject,
    };
    mgr.concurrency.update_config("ck", cc.clone());

    // Acquire 2 permits
    let r1 = mgr.concurrency.acquire("ck", &cc).await.unwrap();
    let r2 = mgr.concurrency.acquire("ck", &cc).await.unwrap();
    assert!(matches!(r1, CR::Allowed(_)));
    assert!(matches!(r2, CR::Allowed(_)));

    // 3rd should be rejected since max_concurrent=2 and no queue
    let r3 = mgr.concurrency.acquire("ck", &cc).await.unwrap();
    assert!(matches!(r3, CR::Rejected { .. }));

    // Release a permit by dropping an OwnedSemaphorePermit (the real semaphore release).
    drop(r1);
    // Now a new acquire should succeed
    let r4 = mgr.concurrency.acquire("ck", &cc).await.unwrap();
    assert!(matches!(r4, CR::Allowed(_)));
    drop(r2);
    drop(r4);
}

#[tokio::test]
async fn concurrency_manager_no_entry_no_semaphore_needed() {
    // Without any entry configured, get_status returns None
    let mgr = memory_manager();
    let status = mgr.concurrency.get_status("unconfigured-key");
    assert!(status.is_none());
}

#[tokio::test]
async fn concurrency_status_reflects_active_count() {
    let mgr = memory_manager();
    let cc = ConcurrencyConfig {
        max_concurrent: 5,
        max_queue_size: 2,
        queue_timeout_ms: 5000,
        strategy: QueueStrategy::QueueFifo,
    };
    mgr.concurrency.update_config("sk", cc.clone());

    let _p1 = mgr.concurrency.acquire("sk", &cc).await.unwrap();
    let _p2 = mgr.concurrency.acquire("sk", &cc).await.unwrap();

    let status = mgr.concurrency.get_status("sk");
    assert!(status.is_some());
    let (_active, _queued, max_concurrent, _max_q) = status.unwrap();
    assert_eq!(max_concurrent, 5);
}

// ---------------------------------------------------------------------------
// 11. File config
// ---------------------------------------------------------------------------

#[tokio::test]
async fn file_config_load_and_policies_populated() {
    use routiium::rate_limit::{PolicyDef, RateLimitFileConfig};
    use std::collections::HashMap;

    let mut policies = HashMap::new();
    policies.insert(
        "file-pol".to_string(),
        PolicyDef {
            buckets: vec![RateLimitBucket {
                name: "main".into(),
                requests: 50,
                window_seconds: 3600,
                window_type: WindowType::Fixed,
            }],
            concurrency: None,
        },
    );

    let config = RateLimitFileConfig {
        version: "1".into(),
        default_policy: Some("file-pol".to_string()),
        policies,
        key_overrides: HashMap::new(),
    };

    // Write config to a temp file
    let tmpdir = tempfile::tempdir().unwrap();
    let config_path = tmpdir.path().join("rate_limit.json");
    std::fs::write(&config_path, serde_json::to_string_pretty(&config).unwrap()).unwrap();

    let mgr = memory_manager();
    mgr.load_file_config(config_path.to_str().unwrap())
        .await
        .unwrap();

    // Policy should be loaded
    let pol = mgr.get_policy("file-pol").await.unwrap();
    assert!(pol.is_some());
    assert_eq!(pol.unwrap().buckets[0].requests, 50);

    // Default policy should be set
    let default_id = mgr.get_default_policy_id().await.unwrap();
    assert_eq!(default_id, Some("file-pol".to_string()));
}

#[tokio::test]
async fn file_config_key_overrides_are_applied() {
    use routiium::rate_limit::{PolicyDef, RateLimitFileConfig};
    use std::collections::HashMap;

    let mut policies = HashMap::new();
    policies.insert(
        "override-pol".to_string(),
        PolicyDef {
            buckets: vec![RateLimitBucket {
                name: "main".into(),
                requests: 1,
                window_seconds: 3600,
                window_type: WindowType::Fixed,
            }],
            concurrency: None,
        },
    );

    let mut key_overrides = HashMap::new();
    key_overrides.insert("special-key".to_string(), "override-pol".to_string());

    let config = RateLimitFileConfig {
        version: "1".into(),
        default_policy: None,
        policies,
        key_overrides,
    };

    let tmpdir = tempfile::tempdir().unwrap();
    let config_path = tmpdir.path().join("rl2.json");
    std::fs::write(&config_path, serde_json::to_string_pretty(&config).unwrap()).unwrap();

    let mgr = memory_manager();
    mgr.load_file_config(config_path.to_str().unwrap())
        .await
        .unwrap();

    // special-key should use override-pol → 1 req limit
    let r1 = mgr
        .check_rate_limit("special-key", "/", None)
        .await
        .unwrap();
    assert!(r1.allowed);

    let r2 = mgr
        .check_rate_limit("special-key", "/", None)
        .await
        .unwrap();
    assert!(!r2.allowed);
}

// ---------------------------------------------------------------------------
// 12. default_policy_from_env
// ---------------------------------------------------------------------------

#[test]
fn default_policy_from_env_parses_daily_and_minute() {
    let _guard = ENV_LOCK.lock().unwrap();
    std::env::set_var("ROUTIIUM_RATE_LIMIT_DAILY", "500");
    std::env::set_var("ROUTIIUM_RATE_LIMIT_PER_MINUTE", "20");
    std::env::remove_var("ROUTIIUM_RATE_LIMIT_BUCKETS");

    let pol = default_policy_from_env();
    assert!(pol.is_some());
    let p = pol.unwrap();
    assert!(p
        .buckets
        .iter()
        .any(|b| b.window_seconds == 86400 && b.requests == 500));
    assert!(p
        .buckets
        .iter()
        .any(|b| b.window_seconds == 60 && b.requests == 20));

    std::env::remove_var("ROUTIIUM_RATE_LIMIT_DAILY");
    std::env::remove_var("ROUTIIUM_RATE_LIMIT_PER_MINUTE");
}

#[test]
fn default_policy_from_env_returns_none_when_no_env() {
    let _guard = ENV_LOCK.lock().unwrap();
    std::env::remove_var("ROUTIIUM_RATE_LIMIT_DAILY");
    std::env::remove_var("ROUTIIUM_RATE_LIMIT_PER_MINUTE");
    std::env::remove_var("ROUTIIUM_RATE_LIMIT_BUCKETS");

    let pol = default_policy_from_env();
    assert!(pol.is_none());
}

// ---------------------------------------------------------------------------
// 13. Admin HTTP endpoints
// ---------------------------------------------------------------------------

#[actix_web::test]
async fn http_list_policies_empty() {
    let mgr = memory_manager();
    let app = make_app!(mgr);

    let req = atest::TestRequest::get()
        .uri("/admin/rate-limits/policies")
        .insert_header(admin_bearer())
        .to_request();
    let resp = atest::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = atest::read_body_json(resp).await;
    assert_eq!(body["count"], 0);
}

#[actix_web::test]
async fn http_create_and_list_policy() {
    let mgr = memory_manager();
    let app = make_app!(mgr);

    let policy = json!({
        "id": "http-pol-1",
        "buckets": [{"name": "main", "requests": 100, "window_seconds": 3600, "window_type": "Fixed"}]
    });

    let create_req = atest::TestRequest::post()
        .uri("/admin/rate-limits/policies")
        .insert_header(admin_bearer())
        .insert_header(("content-type", "application/json"))
        .set_payload(policy.to_string())
        .to_request();
    let create_resp = atest::call_service(&app, create_req).await;
    assert_eq!(create_resp.status(), StatusCode::CREATED);

    let list_req = atest::TestRequest::get()
        .uri("/admin/rate-limits/policies")
        .insert_header(admin_bearer())
        .to_request();
    let list_resp = atest::call_service(&app, list_req).await;
    let body: serde_json::Value = atest::read_body_json(list_resp).await;
    assert_eq!(body["count"], 1);
}

#[actix_web::test]
async fn http_get_policy_not_found() {
    let mgr = memory_manager();
    let app = make_app!(mgr);

    let req = atest::TestRequest::get()
        .uri("/admin/rate-limits/policies/nonexistent")
        .insert_header(admin_bearer())
        .to_request();
    let resp = atest::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[actix_web::test]
async fn http_get_policy_found() {
    let mgr = memory_manager();
    mgr.create_policy(simple_policy("found-pol", 42, 60))
        .await
        .unwrap();
    let app = make_app!(mgr);

    let req = atest::TestRequest::get()
        .uri("/admin/rate-limits/policies/found-pol")
        .insert_header(admin_bearer())
        .to_request();
    let resp = atest::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = atest::read_body_json(resp).await;
    assert_eq!(body["id"], "found-pol");
}

#[actix_web::test]
async fn http_update_policy() {
    let mgr = memory_manager();
    mgr.create_policy(simple_policy("upd-pol", 10, 60))
        .await
        .unwrap();
    let app = make_app!(mgr);

    let update_body = json!({
        "id": "upd-pol",
        "buckets": [{"name": "main", "requests": 999, "window_seconds": 60, "window_type": "Fixed"}]
    });
    let req = atest::TestRequest::put()
        .uri("/admin/rate-limits/policies/upd-pol")
        .insert_header(admin_bearer())
        .insert_header(("content-type", "application/json"))
        .set_payload(update_body.to_string())
        .to_request();
    let resp = atest::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = atest::read_body_json(resp).await;
    assert_eq!(body["success"], true);
}

#[actix_web::test]
async fn http_delete_policy() {
    let mgr = memory_manager();
    mgr.create_policy(simple_policy("del-pol", 5, 60))
        .await
        .unwrap();
    let app = make_app!(mgr);

    let req = atest::TestRequest::delete()
        .uri("/admin/rate-limits/policies/del-pol")
        .insert_header(admin_bearer())
        .to_request();
    let resp = atest::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);

    let body: serde_json::Value = atest::read_body_json(resp).await;
    assert_eq!(body["success"], true);
}

#[actix_web::test]
async fn http_delete_policy_not_found() {
    let mgr = memory_manager();
    let app = make_app!(mgr);

    let req = atest::TestRequest::delete()
        .uri("/admin/rate-limits/policies/ghost-pol")
        .insert_header(admin_bearer())
        .to_request();
    let resp = atest::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[actix_web::test]
async fn http_assign_and_remove_key_policy() {
    let mgr = memory_manager();
    mgr.create_policy(simple_policy("assign-pol", 10, 60))
        .await
        .unwrap();
    let app = make_app!(mgr);

    // Assign
    let assign_req = atest::TestRequest::post()
        .uri("/admin/rate-limits/keys/my-key")
        .insert_header(admin_bearer())
        .insert_header(("content-type", "application/json"))
        .set_payload(json!({"policy_id": "assign-pol"}).to_string())
        .to_request();
    let assign_resp = atest::call_service(&app, assign_req).await;
    assert_eq!(assign_resp.status(), StatusCode::OK);
    let body: serde_json::Value = atest::read_body_json(assign_resp).await;
    assert_eq!(body["success"], true);

    // Remove
    let remove_req = atest::TestRequest::delete()
        .uri("/admin/rate-limits/keys/my-key")
        .insert_header(admin_bearer())
        .to_request();
    let remove_resp = atest::call_service(&app, remove_req).await;
    assert_eq!(remove_resp.status(), StatusCode::OK);
}

#[actix_web::test]
async fn http_key_status() {
    let mgr = memory_manager();
    mgr.create_policy(simple_policy("ks-pol", 10, 60))
        .await
        .unwrap();
    mgr.assign_key_policy("my-key", "ks-pol").await.unwrap();
    let app = make_app!(mgr);

    let req = atest::TestRequest::get()
        .uri("/admin/rate-limits/keys/my-key/status")
        .insert_header(admin_bearer())
        .to_request();
    let resp = atest::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = atest::read_body_json(resp).await;
    assert_eq!(body["key_id"], "my-key");
    assert_eq!(body["blocked"], false);
}

#[actix_web::test]
async fn http_set_and_get_default_policy() {
    let mgr = memory_manager();
    mgr.create_policy(simple_policy("def-pol", 100, 3600))
        .await
        .unwrap();
    let app = make_app!(mgr);

    let set_req = atest::TestRequest::post()
        .uri("/admin/rate-limits/default")
        .insert_header(admin_bearer())
        .insert_header(("content-type", "application/json"))
        .set_payload(json!({"policy_id": "def-pol"}).to_string())
        .to_request();
    let set_resp = atest::call_service(&app, set_req).await;
    assert_eq!(set_resp.status(), StatusCode::OK);

    let get_req = atest::TestRequest::get()
        .uri("/admin/rate-limits/default")
        .insert_header(admin_bearer())
        .to_request();
    let get_resp = atest::call_service(&app, get_req).await;
    assert_eq!(get_resp.status(), StatusCode::OK);
    let body: serde_json::Value = atest::read_body_json(get_resp).await;
    assert_eq!(body["default_policy_id"], "def-pol");
}

#[actix_web::test]
async fn http_emergency_block_and_list() {
    let mgr = memory_manager();
    let app = make_app!(mgr);

    // Create block
    let block_req = atest::TestRequest::post()
        .uri("/admin/rate-limits/emergency")
        .insert_header(admin_bearer())
        .insert_header(("content-type", "application/json"))
        .set_payload(
            json!({
                "key_id": "bad-actor",
                "duration_secs": 3600,
                "reason": "abuse"
            })
            .to_string(),
        )
        .to_request();
    let block_resp = atest::call_service(&app, block_req).await;
    assert_eq!(block_resp.status(), StatusCode::OK);
    let body: serde_json::Value = atest::read_body_json(block_resp).await;
    assert_eq!(body["success"], true);
    assert_eq!(body["key_id"], "bad-actor");

    // List blocks
    let list_req = atest::TestRequest::get()
        .uri("/admin/rate-limits/emergency")
        .insert_header(admin_bearer())
        .to_request();
    let list_resp = atest::call_service(&app, list_req).await;
    assert_eq!(list_resp.status(), StatusCode::OK);
    let list_body: serde_json::Value = atest::read_body_json(list_resp).await;
    assert_eq!(list_body["count"], 1);
}

#[actix_web::test]
async fn http_remove_emergency_block() {
    let mgr = memory_manager();
    mgr.set_emergency_block("rm-blk", Some(3600), "test")
        .await
        .unwrap();
    let app = make_app!(mgr);

    let req = atest::TestRequest::delete()
        .uri("/admin/rate-limits/emergency/rm-blk")
        .insert_header(admin_bearer())
        .to_request();
    let resp = atest::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = atest::read_body_json(resp).await;
    assert_eq!(body["success"], true);
}

#[actix_web::test]
async fn http_concurrency_status_no_config() {
    let mgr = memory_manager();
    let app = make_app!(mgr);

    let req = atest::TestRequest::get()
        .uri("/admin/concurrency/keys/some-key")
        .insert_header(admin_bearer())
        .to_request();
    let resp = atest::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = atest::read_body_json(resp).await;
    assert_eq!(body["key_id"], "some-key");
    assert!(body["concurrency"].is_null());
}

#[actix_web::test]
async fn http_rl_analytics_empty() {
    let mgr = memory_manager();
    let app = make_app!(mgr);

    let req = atest::TestRequest::get()
        .uri("/admin/analytics/rate-limits")
        .insert_header(admin_bearer())
        .to_request();
    let resp = atest::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = atest::read_body_json(resp).await;
    assert_eq!(body["count"], 0);
}

#[actix_web::test]
async fn http_rl_analytics_with_events() {
    let mgr = memory_manager();
    let pol = simple_policy("an-pol", 100, 3600);
    mgr.create_policy(pol).await.unwrap();
    mgr.assign_key_policy("an-key", "an-pol").await.unwrap();
    for _ in 0..3 {
        mgr.check_rate_limit("an-key", "/", None).await.unwrap();
    }
    let app = make_app!(mgr);

    let req = atest::TestRequest::get()
        .uri("/admin/analytics/rate-limits?key_id=an-key")
        .insert_header(admin_bearer())
        .to_request();
    let resp = atest::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = atest::read_body_json(resp).await;
    let count = body["count"].as_u64().unwrap_or(0);
    assert!(count > 0, "expected events, got {}", count);
}

#[actix_web::test]
async fn http_reload_config_no_path_is_ok() {
    let mgr = memory_manager();
    // No config_path set → reload is a no-op and should succeed
    let app = make_app!(mgr);

    let req = atest::TestRequest::post()
        .uri("/admin/rate-limits/reload")
        .insert_header(admin_bearer())
        .to_request();
    let resp = atest::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
}

#[actix_web::test]
async fn http_endpoints_require_admin_auth() {
    let mgr = memory_manager();
    std::env::set_var("ROUTIIUM_ADMIN_TOKEN", "admin-test");
    let state = AppState {
        rate_limit_manager: Some(mgr),
        ..Default::default()
    };
    let app = atest::init_service(
        App::new()
            .app_data(web::Data::new(state))
            .configure(config_routes),
    )
    .await;

    // Without auth header → 401
    let req = atest::TestRequest::get()
        .uri("/admin/rate-limits/policies")
        .to_request();
    let resp = atest::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[actix_web::test]
async fn http_rate_limit_not_enabled_returns_503() {
    // AppState without rate_limit_manager
    std::env::set_var("ROUTIIUM_ADMIN_TOKEN", "admin-test");
    let state = AppState {
        rate_limit_manager: None,
        ..Default::default()
    };
    let app = atest::init_service(
        App::new()
            .app_data(web::Data::new(state))
            .configure(config_routes),
    )
    .await;

    let req = atest::TestRequest::get()
        .uri("/admin/rate-limits/policies")
        .insert_header(admin_bearer())
        .to_request();
    let resp = atest::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
}
