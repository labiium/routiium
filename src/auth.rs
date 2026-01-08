/* routiium/src/auth.rs

Refactored Auth module with pluggable storage backends and async-ready API.

- Default backend: sled (embedded) when compiled with feature "auth-sled".
- Optional backend: Redis (when compiled with feature "auth-redis").
- Fallback (builds without extra deps): in-memory store (for tests and non-feature builds).

Token design:
- Opaque bearer tokens: "sk_<id>.<secret>"
  • id: 32-char hex (UUID v4 as hex)
  • secret: 64-char hex (32 random bytes)
- Storage never keeps plaintext secrets; per-key random salt (16 bytes) +
  SHA-256(salt || secret) hex digest is stored.
- Expiration is enforced at creation-time by default (configurable).

Environment variables:
- ROUTIIUM_KEYS_REQUIRE_EXPIRATION = 1|true|yes|on (default: false)
- ROUTIIUM_KEYS_ALLOW_NO_EXPIRATION = 1|true|yes|on (default: false)
- ROUTIIUM_KEYS_DEFAULT_TTL_SECONDS = <u64 seconds> (optional default)
- ROUTIIUM_SLED_PATH = ./data/keys.db (path for sled)
- ROUTIIUM_REDIS_URL = redis://127.0.0.1/ (url for Redis)

Public API (kept compatible with previous module where possible):
- Types: ApiKeyInfo, GeneratedKey, Verification
- Manager: AuthManager (type alias ApiKeyManager = AuthManager)
- Functions (sync for compatibility, async variants provided with *_async suffix):
    generate_key / generate_key_async
    verify / verify_async
    revoke / revoke_async
    set_expiration / set_expiration_async
    list_keys / list_keys_async
    purge / purge_async
    verify_bearer (helper for Authorization header)

Note:
- The async methods are "async-ready" wrappers over synchronous backend calls.
  sled is synchronous; Redis implementation (when enabled) executes blocking commands
  synchronously here as a simple placeholder and should be adapted to use async Redis
  drivers in an async executor if desired.

*/

#![forbid(unsafe_code)]

use anyhow::{anyhow, Result};
use tracing::{debug, info, warn};

use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use uuid::Uuid;

// ==============================
// Public model
// ==============================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyInfo {
    pub id: String,
    pub label: Option<String>,
    pub created_at: u64,
    pub expires_at: Option<u64>,
    pub revoked_at: Option<u64>,
    pub scopes: Option<Vec<String>>,
}

impl From<ApiKeyRecord> for ApiKeyInfo {
    fn from(rec: ApiKeyRecord) -> Self {
        Self {
            id: rec.id,
            label: rec.label,
            created_at: rec.created_at,
            expires_at: rec.expires_at,
            revoked_at: rec.revoked_at,
            scopes: rec.scopes,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratedKey {
    pub id: String,
    pub token: String, // "sk_<id>.<secret>"
    pub created_at: u64,
    pub expires_at: Option<u64>,
    pub label: Option<String>,
    pub scopes: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Verification {
    Valid {
        id: String,
        label: Option<String>,
        expires_at: Option<u64>,
        scopes: Option<Vec<String>>,
    },
    InvalidTokenFormat,
    NotFound,
    Revoked {
        revoked_at: u64,
    },
    Expired {
        expired_at: u64,
    },
    HashMismatch,
}

// Internal record persisted in the backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ApiKeyRecord {
    id: String,
    label: Option<String>,
    created_at: u64,
    expires_at: Option<u64>,
    revoked_at: Option<u64>,
    salt_hex: String,
    hash_hex: String,
    scopes: Option<Vec<String>>,
}

// ==============================
// Storage trait
// ==============================

trait KeyStore: Send + Sync {
    fn put(&self, rec: &ApiKeyRecord) -> Result<()>;
    fn get(&self, id: &str) -> Result<Option<ApiKeyRecord>>;
    fn list(&self) -> Result<Vec<ApiKeyRecord>>;
    fn purge(&self, cutoff_epoch: u64) -> Result<usize>;
}

// ==============================
// sled backend (feature: auth-sled)
// ==============================

mod sled_store_impl {
    use super::*;
    use std::path::PathBuf;

    pub struct SledStore {
        _db: sled::Db,
        tree: sled::Tree,
    }

    impl SledStore {
        pub fn open_default() -> Result<Self> {
            let path = std::env::var("ROUTIIUM_SLED_PATH")
                .ok()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| "./data/keys.db".to_string());
            Self::open_path(PathBuf::from(path))
        }

        pub fn open_path(path: PathBuf) -> Result<Self> {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).ok();
            }
            let db = sled::open(path)?;
            let tree = db.open_tree("keys")?;
            Ok(Self { _db: db, tree })
        }
    }

    impl KeyStore for SledStore {
        fn put(&self, rec: &ApiKeyRecord) -> Result<()> {
            let key = rec.id.as_bytes().to_vec();
            let val = serde_json::to_vec(rec)?;
            self.tree.insert(key, val)?;
            self.tree.flush()?;
            Ok(())
        }

        fn get(&self, id: &str) -> Result<Option<ApiKeyRecord>> {
            if let Some(ivec) = self.tree.get(id.as_bytes())? {
                let rec: ApiKeyRecord = serde_json::from_slice(&ivec)?;
                Ok(Some(rec))
            } else {
                Ok(None)
            }
        }

        /* delete removed */

        fn list(&self) -> Result<Vec<ApiKeyRecord>> {
            let mut out = Vec::new();
            for item in self.tree.iter() {
                let (_k, v) = item?;
                let rec: ApiKeyRecord = serde_json::from_slice(&v)?;
                out.push(rec);
            }
            Ok(out)
        }

        fn purge(&self, cutoff_epoch: u64) -> Result<usize> {
            let mut removed = 0usize;
            for item in self.tree.iter() {
                let (k, v) = item?;
                let rec: ApiKeyRecord = serde_json::from_slice(&v)?;
                let expired = rec.expires_at.map(|e| e <= cutoff_epoch).unwrap_or(false);
                let revoked = rec.revoked_at.map(|r| r <= cutoff_epoch).unwrap_or(false);
                if expired || revoked {
                    self.tree.remove(k)?;
                    removed += 1;
                }
            }
            self.tree.flush()?;
            Ok(removed)
        }
    }

    pub fn make_store() -> Result<Arc<dyn KeyStore>> {
        Ok(Arc::new(SledStore::open_default()?))
    }
}

// ==============================
// Redis backend (feature: auth-redis)
// ==============================

mod redis_store_impl {
    use super::*;
    // This placeholder uses blocking Redis commands; replace with async driver if desired.
    // Requires the "redis" crate with the "tokio-comp" or async features for production use.
    // Custom r2d2 manager using redis 0.24
    pub struct RedisConnectionManager {
        client: redis::Client,
    }

    impl r2d2::ManageConnection for RedisConnectionManager {
        type Connection = redis::Connection;
        type Error = redis::RedisError;

        fn connect(&self) -> Result<Self::Connection, Self::Error> {
            self.client.get_connection()
        }

        fn is_valid(&self, conn: &mut Self::Connection) -> Result<(), Self::Error> {
            let _: String = redis::cmd("PING").query(conn)?;
            Ok(())
        }

        fn has_broken(&self, _conn: &mut Self::Connection) -> bool {
            // Conservatively assume connection is fine; r2d2 will recycle on errors.
            false
        }
    }

    pub struct RedisStore {
        pool: r2d2::Pool<RedisConnectionManager>,
        key_ns: String,
    }

    impl RedisStore {
        pub fn connect_default() -> Result<Self> {
            let url = std::env::var("ROUTIIUM_REDIS_URL")
                .unwrap_or_else(|_| "redis://127.0.0.1/".to_string());
            Self::connect_url(&url)
        }

        pub fn connect_url(url: &str) -> Result<Self> {
            let client = redis::Client::open(url)?;
            let manager = RedisConnectionManager { client };
            let max_size = std::env::var("ROUTIIUM_REDIS_POOL_MAX")
                .ok()
                .and_then(|s| s.trim().parse::<u32>().ok())
                .unwrap_or(16);
            let pool = r2d2::Pool::builder().max_size(max_size).build(manager)?;
            Ok(Self {
                pool,
                key_ns: "routiium:keys:".to_string(),
            })
        }

        fn key_for(&self, id: &str) -> String {
            format!("{}{}", self.key_ns, id)
        }
    }

    impl KeyStore for RedisStore {
        fn put(&self, rec: &ApiKeyRecord) -> Result<()> {
            let mut conn = self.pool.get()?;
            let key = self.key_for(&rec.id);
            let val = serde_json::to_string(rec)?;
            let _: () = redis::cmd("SET").arg(&key).arg(val).query(&mut *conn)?;
            Ok(())
        }

        fn get(&self, id: &str) -> Result<Option<ApiKeyRecord>> {
            let mut conn = self.pool.get()?;
            let key = self.key_for(id);
            let val: Option<String> = redis::cmd("GET").arg(&key).query(&mut *conn)?;
            match val {
                Some(s) => Ok(Some(serde_json::from_str(&s)?)),
                None => Ok(None),
            }
        }

        /* delete removed */

        fn list(&self) -> Result<Vec<ApiKeyRecord>> {
            let mut conn = self.pool.get()?;
            let pattern = format!("{}*", self.key_ns);
            let keys: Vec<String> = redis::cmd("KEYS").arg(pattern).query(&mut *conn)?;
            let mut out = Vec::new();
            for k in keys {
                if let Ok(s) = redis::cmd("GET").arg(&k).query::<String>(&mut *conn) {
                    if let Ok(rec) = serde_json::from_str::<ApiKeyRecord>(&s) {
                        out.push(rec);
                    }
                }
            }
            Ok(out)
        }

        fn purge(&self, cutoff_epoch: u64) -> Result<usize> {
            let recs = self.list()?;
            let mut conn = self.pool.get()?;
            let mut removed = 0usize;
            for rec in recs {
                let expired = rec.expires_at.map(|e| e <= cutoff_epoch).unwrap_or(false);
                let revoked = rec.revoked_at.map(|r| r <= cutoff_epoch).unwrap_or(false);
                if expired || revoked {
                    let key = self.key_for(&rec.id);
                    let n: i64 = redis::cmd("DEL").arg(&key).query(&mut *conn)?;
                    if n > 0 {
                        removed += 1;
                    }
                }
            }
            Ok(removed)
        }
    }

    pub fn make_store() -> Result<Arc<dyn KeyStore>> {
        Ok(Arc::new(RedisStore::connect_default()?))
    }
    pub fn make_store_with_url(url: &str) -> Result<Arc<dyn KeyStore>> {
        Ok(Arc::new(RedisStore::connect_url(url)?))
    }
}

// ==============================
// In-memory fallback (no features)
// ==============================

mod memory_store_impl {
    use super::*;
    use std::collections::HashMap;
    use std::sync::RwLock;

    #[derive(Default)]
    pub struct MemoryStore {
        inner: RwLock<HashMap<String, ApiKeyRecord>>,
    }

    impl KeyStore for MemoryStore {
        fn put(&self, rec: &ApiKeyRecord) -> Result<()> {
            self.inner
                .write()
                .expect("lock")
                .insert(rec.id.clone(), rec.clone());
            Ok(())
        }

        fn get(&self, id: &str) -> Result<Option<ApiKeyRecord>> {
            Ok(self.inner.read().expect("lock").get(id).cloned())
        }

        /* delete removed */

        fn list(&self) -> Result<Vec<ApiKeyRecord>> {
            Ok(self.inner.read().expect("lock").values().cloned().collect())
        }

        fn purge(&self, cutoff_epoch: u64) -> Result<usize> {
            let mut g = self.inner.write().expect("lock");
            let before = g.len();
            g.retain(|_, rec| {
                let expired = rec.expires_at.map(|e| e <= cutoff_epoch).unwrap_or(false);
                let revoked = rec.revoked_at.map(|r| r <= cutoff_epoch).unwrap_or(false);
                !(expired || revoked)
            });
            Ok(before.saturating_sub(g.len()))
        }
    }

    pub fn make_store() -> Result<Arc<dyn KeyStore>> {
        Ok(Arc::new(MemoryStore::default()))
    }
}

// Select the default make_store() according to enabled features (priority: redis > sled > memory)
fn make_default_store() -> Result<Arc<dyn KeyStore>> {
    if let Ok(url) = std::env::var("ROUTIIUM_REDIS_URL") {
        let u = url.trim();
        if !u.is_empty() {
            return redis_store_impl::make_store_with_url(u);
        }
    }
    sled_store_impl::make_store()
}

// ==============================
// Manager
// ==============================

pub struct AuthManager {
    store: Arc<dyn KeyStore>,
    cache: Option<Arc<RwLock<HashMap<String, ApiKeyRecord>>>>,
}

impl AuthManager {
    fn from_store(store: Arc<dyn KeyStore>) -> Result<Self> {
        let cache = if Self::cache_enabled() {
            match store.list() {
                Ok(records) => {
                    let len = records.len();
                    let mut map = HashMap::with_capacity(len);
                    for rec in records {
                        map.insert(rec.id.clone(), rec);
                    }
                    if len > 0 {
                        info!(
                            "API key cache warmed with {} entr{}",
                            len,
                            if len == 1 { "y" } else { "ies" }
                        );
                    } else {
                        debug!("API key cache warmed (store empty)");
                    }
                    Some(Arc::new(RwLock::new(map)))
                }
                Err(err) => {
                    warn!(
                        "Failed to preload API key cache (continuing without cache): {}",
                        err
                    );
                    None
                }
            }
        } else {
            debug!("API key cache disabled via ROUTIIUM_KEYS_DISABLE_CACHE");
            None
        };

        Ok(Self { store, cache })
    }

    fn cache_enabled() -> bool {
        !env_truthy("ROUTIIUM_KEYS_DISABLE_CACHE", false)
    }

    pub fn new_default() -> Result<Self> {
        let store = make_default_store()?;
        Self::from_store(store)
    }

    pub fn new_with_redis_url(url: &str) -> Result<Self> {
        let store = redis_store_impl::make_store_with_url(url)?;
        Self::from_store(store)
    }

    pub fn new_with_sled_path(path: &std::path::Path) -> Result<Self> {
        let store = sled_store_impl::SledStore::open_path(path.to_path_buf())
            .map(|s| Arc::new(s) as Arc<dyn KeyStore>)?;
        Self::from_store(store)
    }

    pub fn new_sled_default() -> Result<Self> {
        let store = sled_store_impl::make_store()?;
        Self::from_store(store)
    }

    pub fn new_redis_default() -> Result<Self> {
        let store = redis_store_impl::make_store()?;
        Self::from_store(store)
    }

    fn cache_lookup(&self, id: &str) -> Option<ApiKeyRecord> {
        self.cache
            .as_ref()
            .and_then(|cache| cache.read().ok().and_then(|map| map.get(id).cloned()))
    }

    fn cache_upsert(&self, rec: &ApiKeyRecord) {
        if let Some(cache) = &self.cache {
            if let Ok(mut map) = cache.write() {
                map.insert(rec.id.clone(), rec.clone());
            }
        }
    }

    fn cache_snapshot(&self) -> Option<Vec<ApiKeyRecord>> {
        self.cache.as_ref().and_then(|cache| {
            cache
                .read()
                .ok()
                .map(|map| map.values().cloned().collect::<Vec<_>>())
        })
    }

    fn store_get_with_metrics(&self, id: &str) -> Result<Option<ApiKeyRecord>> {
        let start = Instant::now();
        let result = self.store.get(id);
        Self::log_store_latency("get", start.elapsed());
        result
    }

    fn fetch_record(&self, id: &str) -> Option<ApiKeyRecord> {
        match self.store_get_with_metrics(id) {
            Ok(Some(rec)) => {
                self.cache_upsert(&rec);
                Some(rec)
            }
            Ok(None) => None,
            Err(err) => {
                warn!("API key store lookup failed: {}", err);
                None
            }
        }
    }

    fn log_store_latency(operation: &str, elapsed: Duration) {
        if elapsed >= Duration::from_millis(250) {
            warn!(
                duration_ms = elapsed.as_millis() as u64,
                "API key store {} latency", operation
            );
        } else if elapsed >= Duration::from_millis(50) {
            debug!(
                duration_ms = elapsed.as_millis() as u64,
                "API key store {} latency", operation
            );
        }
    }

    // --------------------------
    // Key lifecycle (sync API)
    // --------------------------

    pub fn generate_key(
        &self,
        label: Option<String>,
        ttl: Option<Duration>,
        scopes: Option<Vec<String>>,
    ) -> Result<GeneratedKey> {
        let created_at = now_epoch();
        // Policy: enforce expiration by default; allow override by env flags
        let require_exp = env_truthy("ROUTIIUM_KEYS_REQUIRE_EXPIRATION", false);
        let allow_no_exp = env_truthy("ROUTIIUM_KEYS_ALLOW_NO_EXPIRATION", false);

        // Optional default TTL
        let default_ttl = std::env::var("ROUTIIUM_KEYS_DEFAULT_TTL_SECONDS")
            .ok()
            .and_then(|s| s.trim().parse::<u64>().ok())
            .map(Duration::from_secs);

        let ttl_eff = ttl.or(default_ttl);

        if require_exp && ttl_eff.is_none() && !allow_no_exp {
            return Err(anyhow!(
                "expiration required: provide ttl_seconds (or set default TTL via env)"
            ));
        }
        if let Some(d) = ttl_eff {
            if d.as_secs() == 0 {
                return Err(anyhow!("ttl must be > 0 seconds"));
            }
        }

        let id = uuid_hex();
        let expires_at = ttl_eff.map(|d| created_at.saturating_add(d.as_secs()));

        // Generate secret and salted hash
        let secret_hex = random_secret_hex(32);
        let salt = random_bytes(16);
        let salt_hex = hex_encode(&salt);

        let mut data = Vec::with_capacity(salt.len() + secret_hex.len() / 2);
        data.extend_from_slice(&salt);
        let secret_bytes = hex_decode(&secret_hex)?;
        data.extend_from_slice(&secret_bytes);
        let digest = sha256(&data);
        let hash_hex = hex_encode(&digest);

        let rec = ApiKeyRecord {
            id: id.clone(),
            label: label.clone(),
            created_at,
            expires_at,
            revoked_at: None,
            salt_hex,
            hash_hex,
            scopes: scopes.clone(),
        };

        // Store (blocking sync under the hood of our async trait wrapper)
        let put_start = Instant::now();
        self.store.put(&rec)?;
        Self::log_store_latency("put", put_start.elapsed());
        self.cache_upsert(&rec);

        Ok(GeneratedKey {
            id,
            token: format!("sk_{}.{}", rec.id, secret_hex),
            created_at,
            expires_at,
            label,
            scopes,
        })
    }

    pub fn verify(&self, token: &str) -> Verification {
        let (id, secret_hex) = match parse_token(token) {
            Some(p) => p,
            None => return Verification::InvalidTokenFormat,
        };

        let rec = match self.cache_lookup(&id).or_else(|| self.fetch_record(&id)) {
            Some(r) => r,
            None => return Verification::NotFound,
        };

        let now = now_epoch();
        if let Some(ts) = rec.revoked_at {
            return Verification::Revoked { revoked_at: ts };
        }
        if let Some(exp) = rec.expires_at {
            if now >= exp {
                warn!(
                    "API key {} expired (now={}, expires_at={})",
                    rec.id, now, exp
                );
                return Verification::Expired { expired_at: exp };
            }
        }

        let salt = match hex_decode(&rec.salt_hex) {
            Ok(s) => s,
            Err(_) => return Verification::HashMismatch,
        };
        let secret_bytes = match hex_decode(&secret_hex) {
            Ok(b) => b,
            Err(_) => return Verification::InvalidTokenFormat,
        };

        let mut data = Vec::with_capacity(salt.len() + secret_bytes.len());
        data.extend_from_slice(&salt);
        data.extend_from_slice(&secret_bytes);
        let digest = sha256(&data);

        match hex_decode(&rec.hash_hex) {
            Ok(expected) if ct_eq(&digest, &expected) => Verification::Valid {
                id: rec.id,
                label: rec.label,
                expires_at: rec.expires_at,
                scopes: rec.scopes,
            },
            _ => Verification::HashMismatch,
        }
    }

    pub fn revoke(&self, id: &str) -> Result<bool> {
        if let Some(mut rec) = self.store_get_with_metrics(id)? {
            if rec.revoked_at.is_none() {
                rec.revoked_at = Some(now_epoch());
                let put_start = Instant::now();
                self.store.put(&rec)?;
                Self::log_store_latency("put", put_start.elapsed());
                self.cache_upsert(&rec);
                return Ok(true);
            }
            self.cache_upsert(&rec);
            return Ok(false);
        }
        Ok(false)
    }

    pub fn set_expiration(&self, id: &str, expires_at: Option<u64>) -> Result<bool> {
        if let Some(mut rec) = self.store_get_with_metrics(id)? {
            rec.expires_at = expires_at;
            let put_start = Instant::now();
            self.store.put(&rec)?;
            Self::log_store_latency("put", put_start.elapsed());
            self.cache_upsert(&rec);
            return Ok(true);
        }
        Ok(false)
    }

    pub fn list_keys(&self) -> Result<Vec<ApiKeyInfo>> {
        if let Some(records) = self.cache_snapshot() {
            return Ok(records.into_iter().map(ApiKeyInfo::from).collect());
        }

        let recs = self.store.list()?;
        if let Some(cache) = &self.cache {
            if let Ok(mut map) = cache.write() {
                map.clear();
                for rec in &recs {
                    map.insert(rec.id.clone(), rec.clone());
                }
            }
        }

        Ok(recs.into_iter().map(ApiKeyInfo::from).collect())
    }

    pub fn purge(&self, cutoff_epoch: u64) -> Result<usize> {
        let removed = self.store.purge(cutoff_epoch)?;
        if let Some(cache) = &self.cache {
            if let Ok(mut map) = cache.write() {
                map.retain(|_, rec| {
                    let expired = rec.expires_at.map(|e| e <= cutoff_epoch).unwrap_or(false);
                    let revoked = rec.revoked_at.map(|r| r <= cutoff_epoch).unwrap_or(false);
                    !(expired || revoked)
                });
            }
        }
        Ok(removed)
    }

    // --------------------------
    // Async wrappers
    // --------------------------

    pub async fn generate_key_async(
        &self,
        label: Option<String>,
        ttl: Option<Duration>,
        scopes: Option<Vec<String>>,
    ) -> Result<GeneratedKey> {
        self.generate_key(label, ttl, scopes)
    }

    pub async fn verify_async(&self, token: &str) -> Verification {
        self.verify(token)
    }

    pub async fn revoke_async(&self, id: &str) -> Result<bool> {
        self.revoke(id)
    }

    pub async fn set_expiration_async(&self, id: &str, expires_at: Option<u64>) -> Result<bool> {
        self.set_expiration(id, expires_at)
    }

    pub async fn list_keys_async(&self) -> Result<Vec<ApiKeyInfo>> {
        self.list_keys()
    }

    pub async fn purge_async(&self, cutoff_epoch: u64) -> Result<usize> {
        self.purge(cutoff_epoch)
    }
}

// Backends selectable at runtime via CLI/env
#[derive(Debug, Clone)]
pub enum KeyBackend {
    Redis { url: String },
    Sled { path: std::path::PathBuf },
    Memory,
}

impl AuthManager {
    /// Build a manager from an explicit backend selection.
    pub fn from_backend(backend: KeyBackend) -> Result<Self> {
        match backend {
            KeyBackend::Redis { url } => Self::new_with_redis_url(&url),
            KeyBackend::Sled { path } => Self::new_with_sled_path(&path),
            KeyBackend::Memory => {
                let store = memory_store_impl::make_store()?;
                Self::from_store(store)
            }
        }
    }

    /// Parse a backend spec string:
    /// - "redis://..." → Redis at URL
    /// - "sled:<path>" → Sled database at path
    /// - "memory"      → In-memory backend
    pub fn backend_from_arg_spec(spec: &str) -> Option<KeyBackend> {
        if spec.starts_with("redis://") {
            return Some(KeyBackend::Redis {
                url: spec.to_string(),
            });
        }
        if let Some(rest) = spec.strip_prefix("sled:") {
            return Some(KeyBackend::Sled {
                path: std::path::PathBuf::from(rest),
            });
        }
        if spec.eq_ignore_ascii_case("memory") {
            return Some(KeyBackend::Memory);
        }
        None
    }

    /// Scan CLI args for a flag like "--keys-backend=<spec>" and return the parsed backend if found.
    pub fn backend_from_args(args: &[String]) -> Option<KeyBackend> {
        for a in args {
            if let Some(spec) = a.strip_prefix("--keys-backend=") {
                if let Some(b) = Self::backend_from_arg_spec(spec) {
                    return Some(b);
                }
            }
        }
        None
    }
}

// Backwards-compatible alias
pub type ApiKeyManager = AuthManager;

// ==============================
// - Helpers: token parsing, hashing, header verification
// ==============================

pub fn verify_bearer(manager: &AuthManager, auth_header: Option<&str>) -> Verification {
    let token = match auth_header {
        Some(raw) => {
            let s = raw.trim();
            if s.len() < 7 {
                return Verification::InvalidTokenFormat;
            }
            let (scheme, rest) = s.split_at(6);
            if !scheme.eq_ignore_ascii_case("bearer") {
                return Verification::InvalidTokenFormat;
            }
            let t = rest.trim();
            if t.is_empty() {
                return Verification::InvalidTokenFormat;
            }
            t
        }
        None => return Verification::InvalidTokenFormat,
    };
    manager.verify(token)
}

fn env_truthy(name: &str, default: bool) -> bool {
    match std::env::var(name) {
        Ok(v) => {
            let s = v.trim().to_ascii_lowercase();
            s == "1" || s == "true" || s == "yes" || s == "on"
        }
        Err(_) => default,
    }
}

fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn uuid_hex() -> String {
    let u = Uuid::new_v4().as_u128();
    format!("{:032x}", u)
}

fn parse_token(token: &str) -> Option<(String, String)> {
    if !token.starts_with("sk_") {
        return None;
    }
    let rest = &token[3..];
    let (id_part, secret_part) = rest.split_once('.')?;
    if id_part.len() != 32 || !is_hex(id_part) {
        return None;
    }
    if secret_part.len() < 32 || !is_hex(secret_part) {
        return None;
    }
    Some((id_part.to_string(), secret_part.to_string()))
}

fn is_hex(s: &str) -> bool {
    s.bytes()
        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b) || (b'A'..=b'F').contains(&b))
}

fn random_bytes(n: usize) -> Vec<u8> {
    let mut v = vec![0u8; n];
    OsRng.fill_bytes(&mut v);
    v
}

fn random_secret_hex(bytes: usize) -> String {
    hex_encode(&random_bytes(bytes))
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

fn hex_decode(s: &str) -> Result<Vec<u8>> {
    let s = s.trim();
    if !s.len().is_multiple_of(2) {
        return Err(anyhow!("odd-length hex"));
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    for pair in s.as_bytes().chunks(2) {
        let hi = hex_val(pair[0]).ok_or_else(|| anyhow!("invalid hex"))?;
        let lo = hex_val(*pair.get(1).ok_or_else(|| anyhow!("invalid hex"))?)
            .ok_or_else(|| anyhow!("invalid hex"))?;
        out.push((hi << 4) | lo);
    }
    Ok(out)
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Constant-time equality for two byte slices.
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut acc: u8 = 0;
    for i in 0..a.len() {
        acc |= a[i] ^ b[i];
    }
    acc == 0
}

// Minimal pure-Rust SHA-256 (streaming)
const K256: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

#[inline(always)]
fn rotr(x: u32, n: u32) -> u32 {
    x.rotate_right(n)
}
#[inline(always)]
fn ch(x: u32, y: u32, z: u32) -> u32 {
    (x & y) ^ ((!x) & z)
}
#[inline(always)]
fn maj(x: u32, y: u32, z: u32) -> u32 {
    (x & y) ^ (x & z) ^ (y & z)
}
#[inline(always)]
fn big_sigma0(x: u32) -> u32 {
    rotr(x, 2) ^ rotr(x, 13) ^ rotr(x, 22)
}
#[inline(always)]
fn big_sigma1(x: u32) -> u32 {
    rotr(x, 6) ^ rotr(x, 11) ^ rotr(x, 25)
}
#[inline(always)]
fn small_sigma0(x: u32) -> u32 {
    rotr(x, 7) ^ rotr(x, 18) ^ (x >> 3)
}
#[inline(always)]
fn small_sigma1(x: u32) -> u32 {
    rotr(x, 17) ^ rotr(x, 19) ^ (x >> 10)
}

struct Sha256 {
    state: [u32; 8],
    buffer: [u8; 64],
    buflen: usize,
    bit_len: u64,
}

impl Sha256 {
    fn new() -> Self {
        Self {
            state: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
                0x5be0cd19,
            ],
            buffer: [0u8; 64],
            buflen: 0,
            bit_len: 0,
        }
    }
    fn update(&mut self, data: &[u8]) {
        let mut off = 0usize;
        self.bit_len = self.bit_len.wrapping_add((data.len() as u64) * 8);

        if self.buflen > 0 {
            let need = 64 - self.buflen;
            let take = need.min(data.len());
            self.buffer[self.buflen..self.buflen + take].copy_from_slice(&data[off..off + take]);
            self.buflen += take;
            off += take;
            if self.buflen == 64 {
                let mut block = [0u8; 64];
                block.copy_from_slice(&self.buffer);
                self.compress(&block);
                self.buflen = 0;
            }
        }

        while off + 64 <= data.len() {
            self.compress(&data[off..off + 64]);
            off += 64;
        }

        let rem = data.len() - off;
        if rem > 0 {
            self.buffer[..rem].copy_from_slice(&data[off..]);
            self.buflen = rem;
        }
    }
    fn finalize(mut self) -> [u8; 32] {
        let mut block = [0u8; 128];
        let n = self.buflen;
        block[..n].copy_from_slice(&self.buffer[..n]);
        block[n] = 0x80;

        if n + 1 + 8 <= 64 {
            let bit_len_be = self.bit_len.to_be_bytes();
            block[64 - 8..64].copy_from_slice(&bit_len_be);
            self.compress(&block[..64]);
        } else {
            let bit_len_be = self.bit_len.to_be_bytes();
            block[128 - 8..128].copy_from_slice(&bit_len_be);
            self.compress(&block[..64]);
            self.compress(&block[64..128]);
        }

        let mut out = [0u8; 32];
        for (i, v) in self.state.iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&v.to_be_bytes());
        }
        out
    }
    fn compress(&mut self, block: &[u8]) {
        debug_assert_eq!(block.len(), 64);
        let mut w = [0u32; 64];

        for (i, chunk) in block.chunks(4).take(16).enumerate() {
            w[i] = u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        }
        for i in 16..64 {
            w[i] = small_sigma1(w[i - 2])
                .wrapping_add(w[i - 7])
                .wrapping_add(small_sigma0(w[i - 15]))
                .wrapping_add(w[i - 16]);
        }

        let mut a = self.state[0];
        let mut b = self.state[1];
        let mut c = self.state[2];
        let mut d = self.state[3];
        let mut e = self.state[4];
        let mut f = self.state[5];
        let mut g = self.state[6];
        let mut h = self.state[7];

        for i in 0..64 {
            let t1 = h
                .wrapping_add(big_sigma1(e))
                .wrapping_add(ch(e, f, g))
                .wrapping_add(K256[i])
                .wrapping_add(w[i]);
            let t2 = big_sigma0(a).wrapping_add(maj(a, b, c));
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }

        self.state[0] = self.state[0].wrapping_add(a);
        self.state[1] = self.state[1].wrapping_add(b);
        self.state[2] = self.state[2].wrapping_add(c);
        self.state[3] = self.state[3].wrapping_add(d);
        self.state[4] = self.state[4].wrapping_add(e);
        self.state[5] = self.state[5].wrapping_add(f);
        self.state[6] = self.state[6].wrapping_add(g);
        self.state[7] = self.state[7].wrapping_add(h);
    }
}

fn sha256(data: &[u8]) -> [u8; 32] {
    let mut s = Sha256::new();
    s.update(data);
    s.finalize()
}

// ==============================
// Tests
// ==============================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct CountingStore {
        inner: RwLock<HashMap<String, ApiKeyRecord>>,
        get_calls: AtomicUsize,
    }

    impl CountingStore {
        fn new() -> Self {
            Self::default()
        }

        fn drain_gets(&self) -> usize {
            self.get_calls.swap(0, Ordering::SeqCst)
        }
    }

    impl KeyStore for CountingStore {
        fn put(&self, rec: &ApiKeyRecord) -> Result<()> {
            self.inner
                .write()
                .expect("counting store put")
                .insert(rec.id.clone(), rec.clone());
            Ok(())
        }

        fn get(&self, id: &str) -> Result<Option<ApiKeyRecord>> {
            self.get_calls.fetch_add(1, Ordering::SeqCst);
            Ok(self
                .inner
                .read()
                .expect("counting store get")
                .get(id)
                .cloned())
        }

        fn list(&self) -> Result<Vec<ApiKeyRecord>> {
            Ok(self
                .inner
                .read()
                .expect("counting store list")
                .values()
                .cloned()
                .collect())
        }

        fn purge(&self, cutoff_epoch: u64) -> Result<usize> {
            let mut guard = self.inner.write().expect("counting store purge");
            let before = guard.len();
            guard.retain(|_, rec| {
                let expired = rec.expires_at.map(|e| e <= cutoff_epoch).unwrap_or(false);
                let revoked = rec.revoked_at.map(|r| r <= cutoff_epoch).unwrap_or(false);
                !(expired || revoked)
            });
            Ok(before.saturating_sub(guard.len()))
        }
    }

    #[tokio::test]
    async fn round_trip_memory_backend() {
        let mgr = AuthManager::from_backend(KeyBackend::Memory).unwrap();

        // generate with TTL
        let out = mgr
            .generate_key_async(Some("unit".into()), Some(Duration::from_secs(3)), None)
            .await
            .unwrap();

        // verify ok
        match mgr.verify_async(&out.token).await {
            Verification::Valid { id, .. } => assert_eq!(id, out.id),
            x => panic!("expected valid, got {:?}", x),
        }

        // revoke then verify
        assert!(mgr.revoke_async(&out.id).await.unwrap());
        match mgr.verify_async(&out.token).await {
            Verification::Revoked { .. } => {}
            x => panic!("expected revoked, got {:?}", x),
        }
    }

    #[tokio::test]
    async fn verify_hits_cache_without_store_read() {
        let store = Arc::new(CountingStore::new());
        let mgr = AuthManager::from_store(store.clone()).unwrap();
        store.drain_gets(); // warm-up list

        let out = mgr
            .generate_key(Some("cached".into()), None, None)
            .expect("generate key");
        assert_eq!(
            store.drain_gets(),
            0,
            "generate should not require store reads"
        );

        match mgr.verify(&out.token) {
            Verification::Valid { .. } => {}
            other => panic!("expected valid token, got {:?}", other),
        }
        assert_eq!(
            store.drain_gets(),
            0,
            "cache should satisfy verification without hitting the store"
        );
    }

    #[tokio::test]
    async fn cache_reflects_revocation() {
        let store = Arc::new(CountingStore::new());
        let mgr = AuthManager::from_store(store.clone()).unwrap();
        store.drain_gets();

        let out = mgr
            .generate_key(Some("revoke-cache".into()), None, None)
            .expect("generate key");
        assert!(mgr.revoke(&out.id).expect("revoke succeeds"));
        store.drain_gets(); // discard the store read performed during revoke

        match mgr.verify(&out.token) {
            Verification::Revoked { .. } => {}
            other => panic!("expected revoked token, got {:?}", other),
        }
        assert_eq!(
            store.drain_gets(),
            0,
            "post-revoke verification should still use the cache"
        );
    }
}
