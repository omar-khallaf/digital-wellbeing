# Deployment Modes — System Daemon vs User Session Daemon

The daemon supports two runtime modes. Mode is selected at startup from the
effective uid, overridable by CLI flags. This document owns the bus/scope
selection, the GUI client resolution algorithm, the plugin resolution rule, and
the degraded-mode behavior. The D-Bus interface itself is unchanged
([06-daemon-dbus.md](./06-daemon-dbus.md)); RBAC is covered in
[07-rbac.md](./07-rbac.md); deployment artifacts in
[10-deployment.md](./10-deployment.md).

## Modes

| Mode        | Selected when | Bus for `org.wellbeing.v1.Daemon` | Database path                                | Scope       | RBAC                        |
| ----------- | ------------- | --------------------------------- | -------------------------------------------- | ----------- | --------------------------- |
| **System**  | `uid == 0`    | **system** bus                    | `/var/lib/digital-wellbeing/db.sqlite` (600) | Multi-user  | Full root-vs-user matrix    |
| **Session** | `uid > 0`     | **session** bus                   | `$XDG_DATA_HOME/digital-wellbeing/db.sqlite` | Single-user | Pass-through (own uid only) |

Both modes expose the **identical** `org.wellbeing.v1.Daemon` interface and the
same `org.wellbeing.v1.Manager` plugin contract. Clients and plugins are
bus-agnostic: they resolve _which_ bus the daemon is on at runtime (below),
never hardcode it.

### Why a session mode?

The original design assumes a root-owned system daemon enforcing wellbeing for
**all** users on the device. That requires root install + a systemd system
service + a root-owned state directory. A user who cannot (or does not want to)
install as root should still be able to run wellbeing for **themselves only**.
Session mode makes the daemon enforce exactly one user — the one it runs as —
and claim its well-known name on that user's session bus. This matches the
device-local, per-user nature of wellbeing and removes the root requirement for
single-user use.

## Mode Selection

```rust
/// Resolved at daemon startup, before any bus name is claimed.
pub enum DaemonMode {
    /// root daemon: system bus, /var/lib DB, multi-user RBAC.
    System { db_path: PathBuf },
    /// user daemon: session bus, XDG DB, single-user scope.
    Session { db_path: PathBuf, uid: Uid },
}

pub fn resolve_daemon_mode(cli: &Cli) -> DaemonMode {
    // Explicit override wins (used for testing / unusual layouts).
    if let Some(bus) = cli.bus {
        return match bus {
            Bus::System  => DaemonMode::System  { db_path: SYSTEM_DB_PATH.into() },
            Bus::Session => DaemonMode::Session {
                db_path: xdg_data_home().join("digital-wellbeing/db.sqlite"),
                uid: nix::unistd::getuid(),
            },
        };
    }
    // Default: decide by effective uid.
    if nix::unistd::geteuid().is_root() {
        DaemonMode::System { db_path: SYSTEM_DB_PATH.into() }
    } else {
        DaemonMode::Session {
            db_path: xdg_data_home().join("digital-wellbeing/db.sqlite"),
            uid: nix::unistd::getuid(),
        }
    }
}
```

`DaemonMode` is constructed once in `main.rs` and threaded (by value) into the
store builder, the D-Bus server (which bus to `Connection::*()` on), and the
`DaemonScope` used by RBAC (below). No runtime re-detection — the type makes the
mode unrepresentable as "unknown".

### logind stays on the system bus

A session-mode daemon still needs power/session state (`Slept`/`ShutDown`/
`Locked`/`LoggedOut`). `systemd-logind` lives on the **system bus** and is
reachable by any user, so `PowerStateWatcher`
([03-linux-platform.md](./03-linux-platform.md)) always connects to the system
bus regardless of daemon mode. Only the daemon's **own**
`org.wellbeing.v1.Daemon` name and the plugin `RegisterPlugin` target move to
the session bus. A session-mode daemon therefore holds **two** connections:

- **session bus** — owns `org.wellbeing.v1.Daemon`, talks to the plugin.
- **system bus** — logind only.

## Daemon Scope (RBAC)

```rust
/// Drives RBAC branching in the D-Bus handlers ([07-rbac.md](./07-rbac.md)).
pub enum DaemonScope {
    /// System mode: full root-vs-user matrix, any uid's rows may be present.
    MultiUser,
    /// Session mode: exactly one user. Caller uid == this. RBAC is pass-through.
    SingleUser(Uid),
}
```

In **`SingleUser`** scope the caller is always the daemon's own user, so the
RBAC matrix collapses to pass-through:

- `owner_id` / `created_by` are **always** written with the daemon's uid.
- All policy CRUD and usage queries are permitted (equivalent to the "root" row
  of the [07-rbac.md](./07-rbac.md) matrix), but scoped to the single user's
  rows — the daemon never holds another user's data.
- The `ListPolicies` `filter_owner` argument is ignored (there is only one
  owner).
- `SO_PEERCRED` authentication is still performed (it's free and guards against
  a different local user reaching the session bus), but the _authorization_ step
  short-circuits to "allow".

## GUI Daemon Resolution (client side)

The GUI never assumes a bus. It resolves the daemon with the following priority,
activated by `org.freedesktop.DBus.StartServiceByName` only when no daemon is
already present:

```rust
/// Returns the bus connection that hosts org.wellbeing.v1.Daemon, or None if
/// no daemon can be reached after activation attempts.
async fn resolve_daemon_bus() -> Option<zbus::Connection> {
    let sys = zbus::Connection::system().await.ok();
    let sess = zbus::Connection::session().await.ok();

    // 1. System bus already has the daemon? (root/system service running)
    if let Some(c) = &sys {
        if name_owner_present(c, "org.wellbeing.v1.Daemon").await {
            return Some(c.clone());
        }
    }
    // 2. Session bus already has the daemon? (user session daemon running)
    if let Some(c) = &sess {
        if name_owner_present(c, "org.wellbeing.v1.Daemon").await {
            return Some(c.clone());
        }
    }
    // 3. Activate the SYSTEM daemon (root systemd service).
    if let Some(c) = &sys {
        if start_service(c, "org.wellbeing.v1.Daemon").await
            && name_owner_present(c, "org.wellbeing.v1.Daemon").await
        {
            return Some(c.clone());
        }
    }
    // 4. Activate the SESSION daemon (user service).
    if let Some(c) = &sess {
        if start_service(c, "org.wellbeing.v1.Daemon").await
            && name_owner_present(c, "org.wellbeing.v1.Daemon").await
        {
            return Some(c.clone());
        }
    }
    None
}
```

`name_owner_present` calls `org.freedesktop.DBus.NameHasOwner`; `start_service`
calls `org.freedesktop.DBus.StartServiceByName` (which triggers D-Bus activation
— see [10-deployment.md](./10-deployment.md)). The proxy is then built against
whichever connection resolved. The reverse-registration pattern is
**unchanged**: the plugin still calls `Daemon.RegisterPlugin` — it just does so
on the resolved bus (next section).

### Degraded mode

If all four steps fail, the GUI does **not** crash or block. It:

1. Shows a non-fatal **warning banner** ("Wellbeing daemon unavailable —
   tracking and blocking disabled").
2. Opens normally; dashboards/reports render whatever is cached (likely empty).
3. Disables policy mutations and any action requiring the daemon.

This resolves
[open question #1](./12-open-questions.md#1-gui-startup-when-daemon-is-not-running).

## Plugin Resolution

The compositor plugin runs inside the user's session. It resolves the daemon by
running the **identical** `resolve_daemon_bus()` algorithm the GUI uses (above):

1. system bus has `org.wellbeing.v1.Daemon`? → use it;
2. else session bus has it? → use it;
3. else `StartServiceByName` on the **system** bus (activate root daemon) → use
   it;
4. else `StartServiceByName` on the **session** bus (activate user daemon) → use
   it.

The plugin then calls `Daemon.RegisterPlugin(instance_id)` on the **resolved**
bus. It does **not** pick a bus by a simpler "prefer system / fall back session"
rule and does **not** register on both buses — it reuses the exact same
`NameHasOwner` / `StartServiceByName` 4-step resolution, so it always lands on
the same daemon the GUI did.

This guarantees **exactly one enforcing daemon per user**: if a root system
daemon is active, both GUI and plugin use it (multi-user enforcement); if not,
both use the user's session daemon (single-user enforcement). Registering on
both buses is explicitly **avoided** — if both daemons happened to run, double
enforcement would draw conflicting overlays for the same user.

If neither daemon is present at plugin start, the plugin logs a warning and
**re-runs the 4-step resolution** on the next `NameOwnerChanged` for
`org.wellbeing.v1.Daemon` and on the next focus change, then (re-)calls
`RegisterPlugin` on the resolved bus. No plugin-side bus caching is needed.

### C++ Side: `resolveDaemonBus()`

The plugin (Hyprland, sdbus-c++) implements resolution as a static free function
that returns the connection to the bus hosting `org.wellbeing.v1.Daemon`:

````cpp
#include <sdbus-c++/sdbus-c++.h>
#include <string>
#include <memory>

/// Probe whether `name` is owned on `conn` via org.freedesktop.DBus.NameHasOwner.
/// Returns true if a name owner exists (non-empty unique name).
static auto nameHasOwner(sdbus::IConnection &conn, const std::string &name) -> bool {
    try {
        auto proxy = sdbus::createProxy(conn, sdbus::ServiceName{"org.freedesktop.DBus"},
                                         sdbus::ObjectPath{"/org/freedesktop/DBus"});
        bool owned = false;
        proxy->callMethod("NameHasOwner")
            .onInterface("org.freedesktop.DBus")
            .withArguments(name)
            .storeResultsTo(owned);
        return owned;
    } catch (const sdbus::Error &) {
        return false;
    }
}

/// Activate a service by calling org.freedesktop.DBus.StartServiceByName.
/// Returns true if activation succeeded (the name is now owned).
static auto startServiceByName(sdbus::IConnection &conn, const std::string &name) -> bool {
    try {
        auto proxy = sdbus::createProxy(conn, sdbus::ServiceName{"org.freedesktop.DBus"},
                                         sdbus::ObjectPath{"/org/freedesktop/DBus"});
        uint32_t result = 0;
        proxy->callMethod("StartServiceByName")
            .onInterface("org.freedesktop.DBus")
            .withArguments(name, 0u)  // flags = 0
            .storeResultsTo(result);
        // result 1 = DBUS_START_REPLY_SUCCESS, 2 = DBUS_START_REPLY_ALREADY_RUNNING
        return result == 1 || result == 2;
    } catch (const sdbus::Error &) {
        return false;
    }
}

### C++ Side: `resolveDaemonBus()`

The plugin (Hyprland, sdbus-c++) implements resolution as a static free function
that probes both buses ephemerally and returns a new connection to the bus that
hosts `org.wellbeing.v1.Daemon`:

```cpp
#include <sdbus-c++/sdbus-c++.h>
#include <string>
#include <memory>

/// Probe whether `name` is owned on `conn` via org.freedesktop.DBus.NameHasOwner.
/// Returns true if a name owner exists (non-empty unique name).
static auto nameHasOwner(sdbus::IConnection &conn, const std::string &name) -> bool {
    try {
        auto proxy = sdbus::createProxy(conn, sdbus::ServiceName{"org.freedesktop.DBus"},
                                         sdbus::ObjectPath{"/org/freedesktop/DBus"});
        bool owned = false;
        proxy->callMethod("NameHasOwner")
            .onInterface("org.freedesktop.DBus")
            .withArguments(name)
            .storeResultsTo(owned);
        return owned;
    } catch (const sdbus::Error &) {
        return false;
    }
}

/// Activate a service by calling org.freedesktop.DBus.StartServiceByName.
/// Returns true if activation succeeded (the name is now owned).
static auto startServiceByName(sdbus::IConnection &conn, const std::string &name) -> bool {
    try {
        auto proxy = sdbus::createProxy(conn, sdbus::ServiceName{"org.freedesktop.DBus"},
                                         sdbus::ObjectPath{"/org/freedesktop/DBus"});
        uint32_t result = 0;
        proxy->callMethod("StartServiceByName")
            .onInterface("org.freedesktop.DBus")
            .withArguments(name, 0u)  // flags = 0
            .storeResultsTo(result);
        // result 1 = DBUS_START_REPLY_SUCCESS, 2 = DBUS_START_REPLY_ALREADY_RUNNING
        return result == 1 || result == 2;
    } catch (const sdbus::Error &) {
        return false;
    }
}

/// Create an ephemeral (no well-known name) bus connection for probing.
static auto createProbeConnection(bool system) -> std::shared_ptr<sdbus::IConnection> {
    try {
        auto conn = system ? sdbus::createSystemBusConnection()
                           : sdbus::createSessionBusConnection();
        return std::shared_ptr<sdbus::IConnection>(conn.release());
    } catch (const sdbus::Error &) {
        return nullptr;
    }
}

/// 4-step daemon bus resolution (identical algorithm to the GUI Rust side).
///
/// Returns BusVariant::System or BusVariant::Session for the bus hosting
/// org.wellbeing.v1.Daemon, or std::nullopt if no daemon can be reached.
///
/// The probe connections are discarded after resolution. The caller MUST then
/// create a NEW named connection against the resolved bus type, claiming
/// org.wellbeing.v1.Manager.*.
static auto resolveDaemonBus() -> std::optional<WellbeingManager::BusVariant> {
    constexpr auto DAEMON_NAME = "org.wellbeing.v1.Daemon";

    // Probe connection for the system bus.
    auto sysConn = createProbeConnection(true);

    // 1. System bus already has the daemon?
    if (sysConn && nameHasOwner(*sysConn, DAEMON_NAME))
        return WellbeingManager::BusVariant::System;

    // Probe connection for the session bus.
    auto sessConn = createProbeConnection(false);

    // 2. Session bus already has the daemon?
    if (sessConn && nameHasOwner(*sessConn, DAEMON_NAME))
        return WellbeingManager::BusVariant::Session;

    // 3. Activate the SYSTEM daemon.
    if (sysConn && startServiceByName(*sysConn, DAEMON_NAME))
        return WellbeingManager::BusVariant::System;

    // 4. Activate the SESSION daemon.
    if (sessConn && startServiceByName(*sessConn, DAEMON_NAME))
        return WellbeingManager::BusVariant::Session;

    return std::nullopt;  // all four steps failed — degraded mode
}
````

The plugin's `PLUGIN_INIT` replaces the current hardcoded
`sdbus::createSystemBusConnection(name)` with:

1. Call `resolveDaemonBus()` to determine which bus hosts the daemon.
2. If resolution succeeds, create a **single** named connection against the
   resolved bus type, claiming `org.wellbeing.v1.Manager.<uid>.<session>`.
3. If resolution returns `std::nullopt`, the plugin logs a warning, opens a
   system bus connection as fallback, and runs degraded (no overlay enforcement
   until the daemon appears). Retry is triggered on `NameOwnerChanged` + focus
   switch.

The probe connections from `resolveDaemonBus()` are **discarded** after
resolution — they exist only to run `NameHasOwner` / `StartServiceByName` calls.
The plugin holds exactly **one** D-Bus connection at a time.

## Deploy Artifacts

See [10-deployment.md](./10-deployment.md) for the full file tree. Summary of
the session-mode additions:

| Artifact                                        | Mode    | Notes                                                                                                                            |
| ----------------------------------------------- | ------- | -------------------------------------------------------------------------------------------------------------------------------- |
| `digital-wellbeing-daemon.service` (systemd)    | System  | `User=root`, `BusName=org.wellbeing.v1.Daemon` (unchanged).                                                                      |
| `dbus-1/system-services/...Daemon.service`      | System  | `User=root`, activates the system daemon.                                                                                        |
| `dbus-1/system.d/org.wellbeing.v1.Daemon.conf`  | System  | Root `own`; `default` send/receive (unchanged).                                                                                  |
| `dbus-1/services/...Daemon.service`             | Session | Session activation; `User` omitted (current user).                                                                               |
| `~/.config/systemd/user/...daemon.service`      | Session | Optional user systemd unit for autostart.                                                                                        |
| `dbus-1/session.d/org.wellbeing.v1.Daemon.conf` | Session | Usually **unnecessary** — the session bus lets the owning user `own` any name by default. Add only if stricter policy is wanted. |
| `/var/lib/digital-wellbeing/db.sqlite`          | System  | mode 600, root-owned (unchanged).                                                                                                |
| `$XDG_DATA_HOME/digital-wellbeing/db.sqlite`    | Session | mode 600, user-owned.                                                                                                            |

## Security Notes

- **No request signing needed.** The plugin reads block state from the daemon's
  own D-Bus name (`org.wellbeing.v1.Daemon.ActiveBlocks`), which is protected by
  the D-Bus daemon. The plugin never accepts commands, so there is no command
  method to spoof. See [05-daemon-auth.md](./05-daemon-auth.md).
- **Session bus is per-user.** On the session bus only the owning user can claim
  `org.wellbeing.v1.Manager.*` names, so the already-open `own` policy is
  naturally tighter than on the system bus.
- **No cross-user enforcement in session mode — by design.** A session daemon
  cannot see or enforce other users; that capability is reserved for the system
  daemon (root).
- **`SO_PEERCRED` still checked** even in session mode (cheap, and blocks a
  different local user from reaching the session bus), but authorization is
  pass-through.

## References

- [06-daemon-dbus.md](./06-daemon-dbus.md) — interface, unchanged; GUI proxy bus
  resolution.
- [07-rbac.md](./07-rbac.md) — `DaemonScope` / `SingleUser` pass-through.
- [10-deployment.md](./10-deployment.md) — service files, DB paths, policies.
- [04-plugin-ipc.md](./04-plugin-ipc.md) — plugin resolution mirrors GUI.
- [03-linux-platform.md](./03-linux-platform.md) — logind on system bus in both
  modes.
- [05-daemon-auth.md](./05-daemon-auth.md) — signing unchanged across buses.
- [12-open-questions.md](./12-open-questions.md) — #1 resolved (degraded mode).
