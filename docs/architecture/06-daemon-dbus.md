# Daemon D-Bus Interface — `org.wellbeing.v1.Daemon`

The daemon exposes a D-Bus interface (`org.wellbeing.v1.Daemon`) for the GUI to
query and mutate state. In **system mode** (root) the daemon is on the **system
bus**; in **session mode** (non-root) it is on the **session bus** — see
[13-deployment-modes.md](./13-deployment-modes.md) for bus/scope selection. All
methods authenticate the caller via D-Bus credentials (`uid`) and enforce RBAC
(see [07-rbac.md](./07-rbac.md)). The plugin side of the same bus is documented
in [04-plugin-ipc.md](./04-plugin-ipc.md).

## Interface Definition

```xml
<node name="/org/wellbeing/Daemon">
  <interface name="org.wellbeing.v1.Daemon">

    <!-- ═══ Policy CRUD ═══ -->

    <method name="ListPolicies">
      <arg name="filter_owner" type="u" direction="in"/>
      <!-- 0 = caller's policies; non-zero: root only, filter by uid -->
      <arg name="policies" type="a(v)" direction="out"/>
    </method>

    <method name="CreatePolicy">
      <arg name="input" type="v" direction="in"/>
      <arg name="id" type="t" direction="out"/>
    </method>

    <method name="UpdatePolicy">
      <arg name="id" type="t" direction="in"/>
      <arg name="input" type="v" direction="in"/>
    </method>

    <method name="DeletePolicy">
      <arg name="id" type="t" direction="in"/>
    </method>

    <!-- ═══ Plugin Registration (open to any plugin) ═══ -->

    <!-- Called by a compositor plugin on startup to advertise itself. The
         daemon reads the caller's SO_PEERCRED uid and unique bus name, stores
         a per-instance ManagerClient, and subscribes to its signals. Open to
         any caller — no RBAC; identity comes from the kernel, not the call. -->
    <method name="RegisterPlugin">
      <arg name="instance_id" type="s" direction="in"/>
    </method>

    <!-- ═══ Daemon identity (for plugin request signing) ═══ -->

    <!-- Ed25519 public key, regenerated in memory each daemon start.
         Open to all (it is a public key) — no RBAC, no SO_PEERCRED gate.
         The compositor plugin reads this property on demand to verify the
         signed Overlay requests the daemon sends it. See
         [05-daemon-auth.md](./05-daemon-auth.md) for the full signing design.
         key_id changes every start so a stale key is detectable. -->
    <property name="DaemonPublicKey" type="(sy)" access="read">
      <!-- (key_id: s, public_key: ay) -->
    </property>

    <!-- ═══ Usage Data ═══ -->

    <method name="GetDailyUsage">
      <arg name="date" type="s" direction="in"/>
      <arg name="user_id" type="u" direction="in"/>
      <arg name="entries" type="a(v)" direction="out"/>
    </method>

    <method name="GetUsageRange">
      <arg name="start_date" type="s" direction="in"/>
      <arg name="end_date" type="s" direction="in"/>
      <arg name="user_id" type="u" direction="in"/>
      <arg name="summaries" type="a(v)" direction="out"/>
    </method>

    <!-- ═══ Categories ═══ -->

    <method name="ListCategories">
      <arg name="categories" type="a(v)" direction="out"/>
    </method>

    <method name="GetAppCategories">
      <arg name="entries" type="a(v)" direction="out"/>
    </method>

    <method name="SetAppCategory">
      <arg name="app_id" type="s" direction="in"/>
      <arg name="category_id" type="x" direction="in"/>
    </method>

    <!-- ═══ Signals ═══ -->

    <signal name="BlockStateChanged">
      <arg name="uid" type="u"/>
      <arg name="app_id" type="s"/>
      <arg name="blocked" type="b"/>
      <arg name="reason" type="u"/>
    </signal>

    <signal name="DailyUsageChanged">
      <arg name="uid" type="u"/>
    </signal>

    <signal name="PolicyMutated">
      <arg name="uid" type="u"/>
    </signal>

  </interface>
</node>
```

## Rust zbus Server Sketch

```rust
use wellbeing_core::*;
use zbus::connection;
use zbus::interface;

pub struct DaemonInterface {
    pool: DbPool,
    signal_ctx: zbus::SignalContext,
}

#[interface(name = "org.wellbeing.v1.Daemon")]
impl DaemonInterface {
    async fn list_policies(
        &self,
        filter_owner: u32,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> zbus::Result<Vec<Policy>> {
        let caller = Self::authenticate(conn).await?;
        // RBAC filter applied here
        self.query_policies(caller, filter_owner).await
    }

    // ... other methods follow same pattern
}

impl DaemonInterface {
    /// Returns the caller's uid. All methods require authentication.
    async fn authenticate(conn: &zbus::Connection) -> zbus::Result<u32> {
        let creds = conn.caller_credentials().await?;
        creds.uid.ok_or_else(|| zbus::Error::Failure(
            "unauthenticated: no uid in credentials".into()
        ))
    }
}
```

### D-Bus Message Size Limits

All D-Bus calls are local AF_UNIX. Default zbus message limit is 128 MB. Typical
payload sizes:

| Call            | Typical size                 | Max expected |
| --------------- | ---------------------------- | ------------ |
| `ListPolicies`  | 200 B – 2 KB                 | 50 KB        |
| `GetDailyUsage` | 2 KB – 8 KB (50–200 entries) | 50 KB        |
| `GetUsageRange` | 10 KB – 100 KB               | 1 MB         |
| Signals         | < 200 B                      | 1 KB         |

All well within limits.

### D-Bus Error Mapping

Domain errors from the daemon's business logic MUST be mapped to well-known
D-Bus error replies, not returned as generic
`org.freedesktop.DBus.Error.Failed`.

**Mapping table:**

| Domain error variant                  | D-Bus error name                          | HTTP analogy |
| ------------------------------------- | ----------------------------------------- | ------------ |
| `PolicyNotFound`                      | `org.wellbeing.Error.PolicyNotFound`      | 404          |
| `PolicyConflict` (duplicate)          | `org.wellbeing.Error.PolicyConflict`      | 409          |
| `PermissionDenied`                    | `org.freedesktop.DBus.Error.AccessDenied` | 403          |
| `ValidationError` (newtype rejection) | `org.wellbeing.Error.InvalidArgument`     | 400          |
| `StorageError` (DB connection)        | `org.freedesktop.DBus.Error.Failed`       | 500          |
| `PluginNotConnected`                  | `org.wellbeing.Error.PluginNotConnected`  | 503          |
| `InternalError`                       | `org.freedesktop.DBus.Error.Failed`       | 500          |

**Implementation pattern** — each D-Bus method handler catches domain errors and
converts them to `zbus::Error` with the mapped name:

```rust
use zbus::fdo;

fn map_domain_to_dbus(error: DomainError) -> zbus::Error {
    match error {
        DomainError::PolicyNotFound(id) => {
            zbus::Error::Failure(format!("org.wellbeing.Error.PolicyNotFound: policy {} not found", id))
        }
        DomainError::PermissionDenied { .. } => {
            fdo::Error::ACCESS_DENIED("caller not authorized".into()).into()
        }
        DomainError::ValidationError(msg) => {
            zbus::Error::Failure(format!("org.wellbeing.Error.InvalidArgument: {}", msg))
        }
        DomainError::StorageError(e) => {
            zbus::Error::Failure(format!("org.freedesktop.DBus.Error.Failed: {}", e))
        }
        // ...
    }
}
```

This ensures D-Bus clients (the GUI, CLI tools) can discriminate error types
programmatically by matching on the error name string, rather than parsing
generic failure messages.

## GUI D-Bus Client Architecture

The GUI maintains an **in-memory stale-while-revalidate cache** and talks
exclusively to the daemon over D-Bus (never directly to SQLite). See
[09-state-flow.md](./09-state-flow.md#gui-cache-architecture) for the GUI-side
cache lifecycle, TTLs, and runtime model.

### Signal Coalescing

D-Bus signals can fire rapidly (e.g., `BlockStateChanged` for every app,
`DailyUsageChanged` on every focus switch). The GUI coalesces them:

```rust
/// Coalesces rapid-fire D-Bus signals into periodic cache invalidations.
pub struct SignalCoalescer {
    blocked_dirty: Arc<AtomicBool>,
    usage_dirty: Arc<AtomicBool>,
    policy_dirty: Arc<AtomicBool>,
}

impl SignalCoalescer {
    pub fn mark_blocked_dirty(&self) {
        self.blocked_dirty.store(true, Ordering::Release);
    }

    pub fn mark_daily_usage_dirty(&self) {
        self.usage_dirty.store(true, Ordering::Release);
    }

    pub fn drain(&self) -> CoalescedNotifications {
        CoalescedNotifications {
            blocked: self.blocked_dirty.swap(false, Ordering::AcqRel),
            usage: self.usage_dirty.swap(false, Ordering::AcqRel),
            policy: self.policy_dirty.swap(false, Ordering::AcqRel),
        }
    }
}
```

### Client Cache

```rust
/// Time-based stale-while-revalidate cache for D-Bus sourced data.
/// No SQLite, no persistence — purely in-memory.
pub struct ClientCache<K: Eq + Hash + Clone, V: Clone> {
    inner: Arc<Mutex<HashMap<K, CacheEntry<V>>>>,
    ttl: Duration,
}

struct CacheEntry<V> {
    value: V,
    fetched_at: Instant,
}

impl<K: Eq + Hash + Clone, V: Clone> ClientCache<K, V> {
    pub fn new(ttl: Duration) -> Self;

    /// Returns cached value if fresh, or `None` if stale/missing.
    pub fn get(&self, key: &K) -> Option<V>;

    /// Insert or update a cache entry.
    pub fn set(&self, key: K, value: V);

    /// Invalidate a specific key.
    pub fn invalidate(&self, key: &K);
}
```

Each D-Bus response is cached for the configured TTL; on a signal the relevant
entries are invalidated and the next render cycle re-fetches from the daemon.

### GUI D-Bus Proxy

```rust
/// Thin zbus proxy wrapping the daemon's D-Bus API.
/// Calls are cached via ClientCache on the read side.
pub struct DaemonClient {
    proxy: DaemonProxy<'static>,
    conn: zbus::Connection,
    cache: ClientCache<String, Vec<u8>>,  // serialized responses
}

impl DaemonClient {
    pub async fn connect() -> Result<Self> {
        // Resolve which bus hosts the daemon (system → session → activate →
        // degrade). Never hardcodes a bus — see 13-deployment-modes.md.
        let conn = resolve_daemon_bus().await
            .ok_or_else(|| anyhow!("wellbeing daemon unavailable"))?;
        let proxy = DaemonProxy::new(&conn).await?;
        Ok(Self { proxy, conn, cache: ClientCache::new(Duration::from_secs(5)) })
    }

    pub async fn get_daily_usage(&self, date: &str)
        -> Result<Vec<DailyUsageEntry>>
    {
        let key = format!("usage:{}", date);
        if let Some(cached) = self.cache.get(&key) {
            return Ok(bincode::deserialize(&cached)?);
        }
        let entries = self.proxy.get_daily_usage(date, nix::unistd::getuid().into()).await?;
        self.cache.set(key.clone(), bincode::serialize(&entries)?);
        Ok(entries)
    }
}
```
