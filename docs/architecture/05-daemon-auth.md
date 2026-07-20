# Daemon-Plugin Trust Model

The architecture uses no cryptographic signing between daemon and plugin. Trust
derives from D-Bus name ownership and kernel-authenticated credentials.

**The plugin never accepts commands.** The daemon only writes state to its own
D-Bus interface (`ActiveBlocks` property, `BlockStateChanged` signal). The
plugin reads state from the daemon's well-known D-Bus name
(`org.wellbeing.v1.Daemon`). Only the daemon process can own that name, so reads
are authenticated by the D-Bus daemon itself.

**`UserAction` carries no daemon-owned data.** The plugin sends `app_id` +
`action` (its window-domain assertion). The daemon looks up the corresponding
`policy_id` from its own `ActiveBlocks` state — it does not trust a value from
the plugin.

**Plugin identity is authenticated by `SO_PEERCRED`.** The daemon reads the
caller's kernel-authenticated uid at `RegisterPlugin` time and scopes all events
from that instance to that user's policies.

## References

- [04-plugin-ipc.md](./04-plugin-ipc.md) — declarative IPC architecture
- [07-rbac.md](./07-rbac.md) — `SO_PEERCRED` uid authentication
