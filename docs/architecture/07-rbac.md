# RBAC Authorization Model

Authorization is enforced by the daemon at the D-Bus method handler level using
the caller's uid from SO_PEERCRED (kernel-authenticated, cannot be spoofed). The
daemon D-Bus interface itself is in [06-daemon-dbus.md](./06-daemon-dbus.md);
the per-event enforcement path is in the
[overview event pipeline](./README.md#event-processing-pipeline).

SO_PEERCRED authentication is always performed, even in session mode. The
authorization step branches on the daemon's scope — see
[Daemon Scope Modes](#daemon-scope-modes).

## Rules

| Operation                      | Root (uid=0)                               | Normal user (uid > 0)                                          |
| ------------------------------ | ------------------------------------------ | -------------------------------------------------------------- |
| ListPolicies(filter)           | Returns policies for filter uid (all if 0) | Returns policies where owner_id == caller_uid. filter ignored. |
| CreatePolicy(input)            | Any owner_id                               | Only if input.owner_id == caller_uid                           |
| UpdatePolicy(id, data)         | Any policy                                 | Only if created_by == caller_uid                               |
| DeletePolicy(id)               | Any policy                                 | Only if created_by == caller_uid                               |
| GetDailyUsage(date, uid)       | Any uid                                    | Only if uid == caller_uid                                      |
| GetUsageRange(start, end, uid) | Any uid                                    | Only if uid == caller_uid                                      |
| ListCategories()               | All (unrestricted)                         | All (unrestricted)                                             |
| GetAppCategories()             | All                                        | All                                                            |
| SetAppCategory(app_id, cat_id) | Applies                                    | Applies                                                        |

Single-user (session) daemon: the "Normal user" column applies with caller_uid
fixed to the daemon's own uid, and the filter_owner argument to ListPolicies is
ignored (there is exactly one owner). See
[Daemon Scope Modes](#daemon-scope-modes).

## Daemon Scope Modes

The daemon runs in one of two scopes, selected at startup
([13-deployment-modes.md](./13-deployment-modes.md)):

| Scope           | Mode    | owner_id / created_by      | Authorization                                                     |
| --------------- | ------- | -------------------------- | ----------------------------------------------------------------- |
| MultiUser       | System  | Any uid (root sets freely) | Full root-vs-user matrix (table above)                            |
| SingleUser(uid) | Session | Always uid (the daemon's)  | Pass-through: caller is always uid -> allow; filter_owner ignored |

In SingleUser scope the RBAC matrix collapses to pass-through: the caller is
always the daemon's own user, so every operation is permitted (equivalent to the
"root" row of the matrix) but only ever touches that one user's rows. The daemon
never holds another user's data, and cannot enforce other users — that
capability is reserved for the MultiUser system daemon (root).

## Policy Visibility Model

Root creates a policy for user 1000: owner_id = 1000, created_by = 0

User 1000 sees this policy in ListPolicies (owner_id matches) but CANNOT update
or delete it (created_by != own uid) -> user sees the limit applied to them but
cannot remove it

User 1000 creates their own policy: owner_id = 1000, created_by = 1000

User 1000 CAN update and delete this policy

Root sees all policies, can manage any

## Enforcer Actor Per-User Policy Application

The EnforcerActor runs one evaluation cycle per FocusChanged event. It:

1. Gets the uid from the plugin's D-Bus connection credentials
2. Queries policies WHERE owner_id = uid AND active = 1
3. Queries daily_usage WHERE user_id = uid
4. Evaluates policies -> applies block / notify / ok

This gives per-user enforcement without per-user actor instances: the single
EnforcerActor scopes all queries by uid.

## Data Model Changes

### events table

An ALTER TABLE adds a user_id column to track which user generated each event.
Stored generated columns timestamp and app_id are unchanged. The user_id allows
per-user event queries. A covering index on (user_id, id) supports the reactive
pattern — "events for user X since last seen event id Y" — with index-only
scans.

### daily_usage table

The daily_usage table adds a user_id column. The primary key changes from (date,
app_id) to (date, user_id, app_id) to support per-user scoping.

### policies table

ALTER TABLE adds created_by and owner_id columns. owner_id is the uid the policy
applies to (RBAC scoping). created_by is the uid that created the policy (RBAC
ownership). An index on owner_id supports the per-user policy query.

### app_categories table

Unchanged. App-to-category mappings are global (shared across users).
