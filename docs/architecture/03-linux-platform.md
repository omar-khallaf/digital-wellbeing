# Linux Platform

The Linux implementation lives in `platform/linux/`. On Linux, window events
come from the compositor plugin via D-Bus `FocusChanged` signals. The plugin
D-Bus contract it talks to is documented in
[04-plugin-ipc.md](./04-plugin-ipc.md); the `Platform` trait it implements is in
[02-platform.md](./02-platform.md).

```
platform/linux/
├── mod.rs             # impl Platform for Linux
├── manager.rs         # D-Bus ManagerClient — Overlay(v), FocusChanged stream
└── suspend.rs         # systemd-logind D-Bus (PrepareForSleep/Shutdown)
```

## App Metadata Resolution

App display name and icon are resolved from the `app_categories` table (see
[persistence/01-database.md](../persistence/01-database.md)). By default,
`app_id` is used as the display name. The `display_name` and `icon_path` columns
in `app_categories` allow per-app overrides configured by the user or seeded as
defaults.

The categorizer resolution chain:

```
1. app_categories (DB — seeded defaults + user edits)
2. AI classification (unmapped apps)
3. Uncategorized
```

## Power & Session State Handling

When the system is about to suspend, hibernate, shut down, lock, or end a
session, the open focus interval must be closed so wall-clock time during that
state is not counted against the app limit. The Linux platform handles this via
D-Bus integration with systemd-logind (`platform/linux/suspend.rs`). It emits
**real** events — never a synthetic `Unfocused`:

- `PrepareForSleep(TRUE)` → `Slept` (covers both suspend and hibernate — logind
  cannot distinguish them at signal time)
- `PrepareForShutdown(TRUE)` → `ShutDown`
- Session `Lock` signal → `Locked`
- Session removed → `LoggedOut`

`Slept`/`ShutDown`/`Locked`/`LoggedOut` are close events: they credit the active
interval (minus any enclosed idle) via `accumulate_interval` and clear the
in-memory `FocusState`. They carry no `app_id`.

```rust
pub struct PowerStateWatcher;

impl PowerStateWatcher {
    /// Connect to logind, register a delay inhibitor for sleep+shutdown, and
    /// subscribe to PrepareForSleep + PrepareForShutdown + Session Lock /
    /// removal signals. On each, emit the corresponding REAL close event
    /// (Slept / ShutDown / Locked / LoggedOut) to close the open interval,
    /// then release the inhibitor.
    pub async fn watch(pool: DbPool, notifier: ReactiveNotifier) -> Result<()> {
        // logind lives on the SYSTEM bus regardless of daemon mode — a
        // session-mode daemon (non-root, on the session bus for its own
        // interface + plugin) still reaches logind here. See
        // 13-deployment-modes.md (logind stays on the system bus).
        let conn = zbus::Connection::system().await?;
        let manager = logind1::ManagerProxy::new(&conn).await?;
        let session = logind1::SessionProxy::new(&conn).await?;

        // Inhibit both sleep (suspend/hibernate) and shutdown (poweroff/reboot)
        let _fd = manager.inhibit(
            "sleep:shutdown", "digital-wellbeing",
            "Flush session data before power state change", "delay",
        ).await?;

        let mut sleep_stream = manager.receive_prepare_for_sleep().await?;
        let mut shutdown_stream = manager.receive_prepare_for_shutdown().await?;
        let mut lock_stream = session.receive_lock().await?;
        let mut unlock_stream = session.receive_unlock().await?;

        loop {
            tokio::select! {
                Some(signal) = sleep_stream.next() => {
                    if signal.get().await? {
                        Self::flush_event(&pool, &notifier, EventType::Slept).await;
                    }
                }
                Some(signal) = shutdown_stream.next() => {
                    if signal.get().await? {
                        Self::flush_event(&pool, &notifier, EventType::ShutDown).await;
                    }
                }
                Some(signal) = lock_stream.next() => {
                    Self::flush_event(&pool, &notifier, EventType::Locked).await;
                }
                // Unlock is a no-op for the interval: the next WindowFocused
                // reopens a fresh interval. No event is emitted.
                Some(_) = unlock_stream.next() => {}
            }
        }
    }

    async fn flush_event(pool: &DbPool, notifier: &ReactiveNotifier, event_type: EventType) {
        if let Ok(mut conn) = pool.get().await {
            // flush_close_event() (see persistence/01-database.md) accumulates the active
            // interval and inserts the real event in one transaction. focus_state
            // and user_id are captured from the owning actor.
            flush_close_event(&mut conn, &focus_state, user_id, event_type).await.ok();
            notifier.notify_event_written();
        }
    }
}
```

Flow (suspend/shutdown example; lock/logout are analogous):

```
logind signal (PrepareForSleep / PrepareForShutdown = TRUE)
        │
        ▼
PowerStateWatcher (platform/linux/suspend.rs)
        │
        │  INSERT Slept / ShutDown into events table
        │  (accumulate_interval credits active time, clears focus state)
        ▼
Dropped inhibitor → power state change proceeds
```

| State     | Signal                     | Event emitted |
| --------- | -------------------------- | ------------- |
| Suspend   | `PrepareForSleep(TRUE)`    | `Slept`       |
| Hibernate | `PrepareForSleep(TRUE)`    | `Slept`       |
| Shutdown  | `PrepareForShutdown(TRUE)` | `ShutDown`    |
| Lock      | Session `Lock`             | `Locked`      |
| Logout    | Session removed            | `LoggedOut`   |

The `"delay"` mode gives us a few seconds to flush before the system pauses.
`LoggedOut` is also emitted by the SIGTERM/SIGHUP handler on daemon termination
(see [persistence/01-database.md](../persistence/01-database.md)).

**Error handling:** If the DB flush fails, log the error and release the
inhibitor anyway. Losing a few seconds of usage data is acceptable; blocking a
power state change is not.

## Compositor Support

| Compositor      | Plugin                  | D-Bus Implementation   | Status        |
| --------------- | ----------------------- | ---------------------- | ------------- |
| **Hyprland**    | `wellbeing-lockdown.so` | sdbus-cpp in C++       | **v1 target** |
| **KWin**        | `wellbeing-effect`      | `KWin::Effect` + D-Bus | Roadmap       |
| **Wayfire**     | `wellbeing-plugin`      | Wayfire plugin + D-Bus | Roadmap       |
| **GNOME Shell** | `wellbeing-extension`   | GJS + D-Bus            | Roadmap       |

All compositors implement the same `org.wellbeing.v1.Manager` D-Bus
**interface** at the same object path (`/org/wellbeing/Manager`), but each
plugin instance claims a **unique well-known bus name** — e.g.
`org.wellbeing.v1.Manager.<uid>.<sess>` — because a D-Bus well-known name is
unique per connection. Discovery is **reverse**: at startup each plugin calls
`Daemon.RegisterPlugin(instance_id)` on the system bus, so the daemon learns the
caller's real `uid` (via `SO_PEERCRED`) and unique bus name and tracks it in
`PluginRegistry`. The daemon does **not** probe a single
`org.wellbeing.v1.Manager` name; it watches `NameOwnerChanged` for
`org.wellbeing.v1.Manager.*` to detect connect/disconnect (see
[04-plugin-ipc.md](./04-plugin-ipc.md#multi-instance-plugin-support)).
