# Deployment

The daemon ships in two modes: a system daemon (root, multi-user enforcement)
and a session daemon (non-root, single-user enforcement). Mode is selected at
startup by effective uid (overridable by --bus) — see
[13-deployment-modes.md](./13-deployment-modes.md). This document covers the
root systemd system service plus the session-mode additions (session D-Bus
service, XDG state path). The binaries and interfaces are introduced in
[06-daemon-dbus.md](./06-daemon-dbus.md) and
[04-plugin-ipc.md](./04-plugin-ipc.md).

## systemd Service

Install deploy/systemd/digital-wellbeing-daemon.service to /etc/systemd/system/.
Source of truth lives in the repo, not this document.

Type=dbus means systemd considers the service "ready" when
org.wellbeing.v1.Controller appears on the system bus. The daemon registers its
well-known name in main.rs before entering the main loop.

This unit is the system mode only (uid == 0, claims the name on the system bus).
A non-root user runs the session daemon instead — see Session Daemon (non-root)
below.

## Session Daemon (non-root)

When the daemon runs as a normal user it claims org.wellbeing.v1.Controller on
the session bus and stores its database under the user's data home. No root, no
systemd system service, no system-bus policy file is required.

### Session D-Bus service (for activation)

D-Bus activation lets the GUI auto-start the session daemon when it resolves the
bus (step 4 of 13-deployment-modes.md). Place under the session service dir
(system-wide or per-user):

A D-Bus service activation file specifies the well-known name
org.wellbeing.v1.Controller and the Exec path to the wellbeing-daemon binary. No
User field is set — it runs as the session owner. A systemd-service field is
optional for user-service activation.

### State directory (XDG)

$XDG_DATA_HOME/digital-wellbeing/ # e.g. ~/.local/share/digital-wellbeing └──
db.sqlite # mode 600, owned by the user

The daemon resolves this path at startup when in session mode
(13-deployment-modes.md#mode-selection); override with --db-path for testing.

### Optional user systemd unit

For autostart, a user unit may be installed at
~/.config/systemd/user/digital-wellbeing-daemon.service (no User=, no BusName=),
WantedBy=default.target. This is optional — D-Bus activation alone is
sufficient.

Session-bus policy: the session bus lets the owning user own any well-known name
by default, so no session.d policy file is needed. Add one only if stricter
scoping is desired.

## D-Bus System Policy Files

Install from repo; do not copy-paste this section into system files.

- deploy/dbus/org.wellbeing.v1.Controller.conf -> /usr/share/dbus-1/system.d/
- deploy/dbus/org.wellbeing.v1.Manager.conf -> /usr/share/dbus-1/system.d/

### Daemon policy

The controller policy file grants own and send/receive to the owning user and
root. The own is also granted to context=default because the daemon runs as root
yet may need the policy active before root's session is fully initialized.
Root's own is the primary rule; context=default is a fallback.

### Plugin policy

The manager policy file grants send/receive to the owning user.

## Directory Structure

/var/lib/digital-wellbeing/ # StateDirectory (SYSTEM mode), mode 700 └──
db.sqlite # SQLite database, mode 600, root-owned

$XDG_DATA_HOME/digital-wellbeing/ # State dir (SESSION mode), mode 700 └──
db.sqlite # SQLite database, mode 600, user-owned

/usr/libexec/digital-wellbeing/ # Binaries ├── wellbeing-daemon # Daemon binary,
mode 755 └── wellbeing-gui # GUI binary, mode 755 (optional, user installs)

deploy/ ├── dbus/ # D-Bus policy (system bus) │ ├──
org.wellbeing.v1.Controller.conf │ └── org.wellbeing.v1.Manager.conf ├──
systemd/ │ └── digital-wellbeing-daemon.service └── system-services/ └──
org.wellbeing.v1.Controller.service

/usr/share/dbus-1/system.d/ # installed policy ├──
org.wellbeing.v1.Controller.conf └── org.wellbeing.v1.Manager.conf

/usr/share/dbus-1/system-services/ # D-Bus activation, system daemon (optional)
└── org.wellbeing.v1.Controller.service

/usr/share/dbus-1/services/ # D-Bus activation, session daemon (optional) └──
org.wellbeing.v1.Controller.service

/etc/systemd/system/ # systemd unit (system daemon) └──
digital-wellbeing-daemon.service

~/.config/systemd/user/ # user systemd unit (session daemon, optional) └──
digital-wellbeing-daemon.service

## D-Bus Activation (Optional)

Install the controller service file to /usr/share/dbus-1/system-services/.

A matching session activation file (under /usr/share/dbus-1/services/, no User=)
auto-starts the session daemon when the GUI falls back to the session bus
(13-deployment-modes.md#gui-daemon-resolution-client-side). Both are optional —
the daemon can also be launched directly (user systemd unit or manual exec).
