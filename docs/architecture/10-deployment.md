# Deployment

The daemon ships in two modes: a **system daemon** (root, multi-user
enforcement) and a **session daemon** (non-root, single-user enforcement). Mode
is selected at startup by effective uid (overridable by `--bus`) — see
[13-deployment-modes.md](./13-deployment-modes.md). This document covers the
root systemd system service plus the session-mode additions (session D-Bus
service, XDG state path). The binaries and interfaces are introduced in
[06-daemon-dbus.md](./06-daemon-dbus.md) and
[04-plugin-ipc.md](./04-plugin-ipc.md).

## systemd Service

```ini
# /etc/systemd/system/digital-wellbeing-daemon.service
[Unit]
Description=Digital Wellbeing Daemon
Documentation=https://github.com/omar/digital-wellbeing
After=dbus.service
Requires=dbus.service

[Service]
Type=dbus
BusName=org.wellbeing.v1.Daemon
ExecStart=/usr/libexec/digital-wellbeing/wellbeing-daemon
Restart=on-failure
RestartSec=3
User=root
Group=root

# Security hardening
NoNewPrivileges=true
ProtectSystem=strict
ReadWritePaths=/var/lib/digital-wellbeing
PrivateTmp=true
PrivateDevices=true
ProtectHome=true
ProtectKernelTunables=true
ProtectKernelModules=true
ProtectControlGroups=true

# State directory
StateDirectory=digital-wellbeing
StateDirectoryMode=700

[Install]
WantedBy=multi-user.target
```

`Type=dbus` means systemd considers the service "ready" when
`org.wellbeing.v1.Daemon` appears on the system bus. The daemon registers its
well-known name in main.rs before entering the main loop.

> This unit is the **system** mode only (`uid == 0`, claims the name on the
> system bus). A non-root user runs the **session** daemon instead — see
> [Session Daemon](#session-daemon-non-root) below.

## Session Daemon (non-root)

When the daemon runs as a normal user it claims `org.wellbeing.v1.Daemon` on the
**session bus** and stores its database under the user's data home. No root, no
systemd system service, no system-bus policy file is required.

### Session D-Bus service (for activation)

D-Bus activation lets the GUI auto-start the session daemon when it resolves the
bus (step 4 of [13-deployment-modes.md](./13-deployment-modes.md)). Place under
the session service dir (system-wide or per-user):

```ini
# /usr/share/dbus-1/services/org.wellbeing.v1.Daemon.service
[D-BUS Service]
Name=org.wellbeing.v1.Daemon
Exec=/usr/libexec/digital-wellbeing/wellbeing-daemon
# No User= — runs as the session owner. systemd-service= optional for
# user-service activation.
```

### State directory (XDG)

```
$XDG_DATA_HOME/digital-wellbeing/      # e.g. ~/.local/share/digital-wellbeing
└── db.sqlite                           # mode 600, owned by the user
```

The daemon resolves this path at startup when in session mode
([13-deployment-modes.md](./13-deployment-modes.md#mode-selection)); override
with `--db-path` for testing.

### Optional user systemd unit

For autostart, a user unit may be installed at
`~/.config/systemd/user/digital-wellbeing-daemon.service` (no `User=`, no
`BusName=`), `WantedBy=default.target`. This is optional — D-Bus activation
alone is sufficient.

> **Session-bus policy:** the session bus lets the owning user `own` any
> well-known name by default, so no `session.d` policy file is needed. Add one
> only if stricter scoping is desired.

## D-Bus System Policy Files

### Daemon policy

```xml
<!-- /usr/share/dbus-1/system.d/org.wellbeing.v1.Daemon.conf -->
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE busconfig PUBLIC
 "-//freedesktop//DTD D-BUS Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <!-- Root owns the daemon -->
  <policy user="root">
    <allow own="org.wellbeing.v1.Daemon"/>
  </policy>

  <!-- Any process can communicate with the daemon -->
  <policy context="default">
    <allow send_destination="org.wellbeing.v1.Daemon"/>
    <allow receive_sender="org.wellbeing.v1.Daemon"/>
    <allow own="org.wellbeing.v1.Daemon"/>
  </policy>
</busconfig>
```

Note: `own` is also granted to `context="default"` because the daemon runs as
root yet may need the policy active before root's session is fully initialized.
Root's `own` is the primary rule; `context="default"` is a fallback.

### Plugin policy

```xml
<!-- /usr/share/dbus-1/system.d/org.wellbeing.v1.Manager.conf -->
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE busconfig PUBLIC
 "-//freedesktop//DTD D-BUS Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <!-- Allow any user session to own the plugin name -->
  <!-- The compositor plugin runs in the user's session (Hyprland) -->
  <policy context="default">
    <allow own="org.wellbeing.v1.Manager"/>
    <allow send_destination="org.wellbeing.v1.Manager"/>
    <allow receive_sender="org.wellbeing.v1.Manager"/>
  </policy>

  <!-- Allow daemon (root) to communicate with plugin -->
  <policy user="root">
    <allow send_destination="org.wellbeing.v1.Manager"/>
    <allow receive_sender="org.wellbeing.v1.Manager"/>
  </policy>
</busconfig>
```

## Directory Structure

```
/var/lib/digital-wellbeing/      # StateDirectory (SYSTEM mode), mode 700
└── db.sqlite                     # SQLite database, mode 600, root-owned

$XDG_DATA_HOME/digital-wellbeing/ # State dir (SESSION mode), mode 700
└── db.sqlite                     # SQLite database, mode 600, user-owned

/usr/libexec/digital-wellbeing/   # Binaries
├── wellbeing-daemon              # Daemon binary, mode 755
└── wellbeing-gui                 # GUI binary, mode 755 (optional, user installs)

/usr/share/dbus-1/system.d/       # D-Bus policy (system bus)
├── org.wellbeing.v1.Daemon.conf
└── org.wellbeing.v1.Manager.conf

/usr/share/dbus-1/system-services/ # D-Bus activation, system daemon (optional)
└── org.wellbeing.v1.Daemon.service

/usr/share/dbus-1/services/        # D-Bus activation, session daemon (optional)
└── org.wellbeing.v1.Daemon.service

/etc/systemd/system/               # systemd unit (system daemon)
└── digital-wellbeing-daemon.service

~/.config/systemd/user/            # user systemd unit (session daemon, optional)
└── digital-wellbeing-daemon.service
```

## D-Bus Activation (Optional)

When the GUI tries to call a method on `org.wellbeing.v1.Daemon` and the name
doesn't exist, systemd can auto-start the daemon:

```ini
# /usr/share/dbus-1/system-services/org.wellbeing.v1.Daemon.service
[D-BUS Service]
Name=org.wellbeing.v1.Daemon
Exec=/usr/libexec/digital-wellbeing/wellbeing-daemon
User=root
systemd-service=digital-wellbeing-daemon.service
```

A matching **session** activation file (`/usr/share/dbus-1/services/...`, no
`User=`) auto-starts the session daemon when the GUI falls back to the session
bus
([13-deployment-modes.md](./13-deployment-modes.md#gui-daemon-resolution-client-side)).
Both are optional — the daemon can also be launched directly (user systemd unit
or manual exec).
