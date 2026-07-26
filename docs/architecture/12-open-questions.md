# Open Questions

## 1. GUI Startup When Daemon Is Not Running

Resolved. The GUI resolves the daemon through a multi-step fallback instead of
relying solely on systemd activation. It first checks whether the system bus
already has the daemon running; if not, it checks the session bus. If the daemon
is not present on either bus, the GUI attempts to start it via D-Bus activation
on the system bus, then on the session bus. If all four steps fail, the GUI
opens in a degraded mode with tracking and enforcement disabled and the UI
read-only, displaying a warning banner rather than failing outright. This
behavior covers both root-installed and user-only installations.

## 2. Daemon Crash Recovery with Active Overlay

Resolved. On startup the daemon reads the most recent event from the events
table and compares it with the compositor plugin's current focus state obtained
from the CurrentFocus property. From that comparison it determines whether the
previous session ended while a window was focused, whether focus changed while
the daemon was down, and whether any overlay was active at the time of the
crash. If an overlay was active for an app that is still focused, the plugin
re-renders the overlay on reconnect by reading the current BlockedApps state.
Policy is re-derived by id from the daemon's own database rather than
re-adopting old in-memory block state.

## 3. gpui Version Compatibility

Resolved. The GUI pins gpui and companion crates to a specific git commit hash
rather than a branch name. Branch names can move, while commit hashes are
immutable. The pin is advanced only after verifying that the new revision
compiles and the UI renders correctly. Tagged releases are preferred when
available.

## 4. Window-Handle Set Tracking for Multi-App Blocking

Resolved. The per-app overlay model is implemented in the compositor plugin.
When an app_id appears in BlockedApps, the plugin renders a block overlay over
every window owned by the app and traps both mouse and keyboard input on each
blocked window. The overlay is managed by the compositor plugin, not the daemon.

## 5. Signal Subscription in the gpui Main Loop

Resolved. The GUI uses a background tokio thread for D-Bus connections and
signal subscriptions, with updates forwarded to gpui via an mpsc channel. The
gpui main loop consumes that channel each render frame via a stored Task handle.
This integration pattern is implemented and verified.
