// =============================================================================
// WellbeingManager — D-Bus org.wellbeing.v1.Manager interface
//
// Implements Overlay(v) / UserAction / FocusChanged / ActivityChanged /
// CurrentSession. See docs/architecture/04-plugin-ipc.md and 05-daemon-auth.md.
// =============================================================================

#include "wellbeing_manager.hpp"

#include <chrono>
#include <cstdint>
#include <memory>
#include <optional>
#include <string>
#include <tuple>
#include <utility>
#include <vector>

#include <sdbus-c++/sdbus-c++.h>

#include "lockdown.hpp"
#include "logging.hpp"
#include "plugin_state.hpp"

using wellbeing::ActionType;
using wellbeing::AppId;
using wellbeing::BlockReason;
using wellbeing::FocusVariantTag;
using wellbeing::g_ctx;
using wellbeing::logErr;
using wellbeing::logInfo;

namespace {

// =============================================================================
// WindowInfo helpers (D-Bus variant encoding)
// =============================================================================

/// Encode an Option<WindowInfo> as a D-Bus variant for FocusChanged.
///
/// Encoding:
///   None          → variant(uint32 FocusVariantTag::Desktop)
///   Some{...}     → variant(struct{FocusVariantTag::App, app_id, title, pid,
///                                   uid, overlay_shown})
auto windowInfoToVariant(const std::optional<WindowInfo> &info) -> sdbus::Variant {
    if (!info.has_value()) {
        return sdbus::Variant{static_cast<uint32_t>(FocusVariantTag::Desktop)};
    }
    return sdbus::Variant{std::tuple{
        static_cast<uint32_t>(FocusVariantTag::App),
        info->appId.value(),
        info->title,
        info->pid,
        info->uid,
        info->overlayShown,
    }};
}

/// Build the CurrentSession variant.
///
/// The CurrentSession readable property MUST encode exactly what the
/// FocusChanged signal carries, so a late-joining client that missed the
/// ephemeral signal can read identical state from the property. Both call
/// windowInfoToVariant(currentFocus): no focus → Desktop (tag 1); focused app
/// → App (tag 2) with overlay_shown reflecting whether that app is blocked.
auto buildSessionVariant() -> sdbus::Variant { return windowInfoToVariant(g_ctx->currentFocus); }

// =============================================================================
// Ed25519 verification (see docs/architecture/05-daemon-auth.md)
//
// FOLLOWS ZERO-TRUST RULE: verification FAILS CLOSED until the full crypto
// chain (fetchDaemonPublicKey + crypto_sign_verify_detached) is wired.
// =============================================================================

/// Fetch the daemon's current Ed25519 public key from
/// org.wellbeing.v1.Daemon.DaemonPublicKey property.
///
/// Returns (key_id, public_key_bytes).
///
/// TODO: Implement full D-Bus property read.
///   sdbus::IProxy& daemon = *sdbus::createProxy(
///       conn, "org.wellbeing.v1.Daemon", "/org/wellbeing/Daemon");
///   std::tuple<std::string, std::vector<uint8_t>> result;
///   daemon.callMethod("Get")
///       .onInterface("org.freedesktop.DBus.Properties")
///       .withArguments("org.wellbeing.v1.Daemon", "DaemonPublicKey")
///       .storeResultsTo(result);
///   return result;
auto fetchDaemonPublicKey(sdbus::IConnection &conn) -> std::pair<std::string, std::vector<uint8_t>> {
    (void)conn;
    // STUB: returns empty key so verification predictably fails.
    return {"", std::vector<uint8_t>()};
}

/// Serialize a sdbus::Variant payload to bytes for Ed25519 signing.
///
/// TODO: Implement proper D-Bus marshalling (zvariant-like) of the payload.
auto serializeVariant(const sdbus::Variant &payload) -> std::vector<uint8_t> {
    (void)payload;
    // STUB: In production, use sdbus-c++ to serialize the variant body.
    return {};
}

/// Append a uint64_t in big-endian to the byte vector (needed for issued_at).
/// Uses size_t for loop counter to avoid signed/unsigned mismatch (int → size_t).
void appendBE(std::vector<uint8_t> &buf, uint64_t val) {
    for (size_t i = 8; i > 0; --i) {
        buf.push_back(static_cast<uint8_t>((val >> ((i - 1) * 8)) & 0xFF));
    }
}

/// Return unix-ms wall clock.
auto nowMs() -> uint64_t {
    return static_cast<uint64_t>(
        std::chrono::duration_cast<std::chrono::milliseconds>(std::chrono::system_clock::now().time_since_epoch())
            .count());
}

/// Verify the outer envelope of an Overlay call.
///
/// FAILS CLOSED: Ed25519 verification is not yet implemented. ALL envelopes
/// are rejected until the full crypto chain is wired. This prevents deploying
/// an unauthenticated plugin (see docs/architecture/05-daemon-auth.md).
///
/// Checks when implemented:
///   1. Ed25519 signature over (payload_bytes ‖ issued_at_be) against
///      the daemon's public key.
///   2. Timestamp freshness window (±30 s, stateless, no nonce cache).
///
///   Requires: libsodium (crypto_sign_verify_detached).
auto verifyEnvelope(sdbus::IConnection &conn, const sdbus::Variant &payload, uint64_t issuedAt,
                    const std::vector<uint8_t> &sig) -> bool {
    // ── Freshness check (independent of crypto stub) ───────────────────
    constexpr int64_t SKEW_MS = 30'000;
    const auto now = static_cast<int64_t>(nowMs());
    const auto ia = static_cast<int64_t>(issuedAt);
    if (ia < now - SKEW_MS || ia > now + SKEW_MS) {
        logErr("verifyEnvelope: timestamp out of window (issuedAt=" + std::to_string(ia) +
               ", now=" + std::to_string(now) + ")");
        return false;
    }

    // ── Ed25519 verification (stub — FAIL CLOSED) ─────────────────────
    (void)conn;
    (void)payload;
    (void)sig;

    logErr("verifyEnvelope: Ed25519 verification not implemented — "
           "rejecting envelope (fail closed)");
    return false;
}

} // anonymous namespace

// =============================================================================
// Daemon bus resolution (4-step)
// =============================================================================

namespace {

/// Probe whether `name` is owned on `conn` via org.freedesktop.DBus.NameHasOwner.
auto nameHasOwner(sdbus::IConnection &conn, const std::string &name) -> bool {
    try {
        auto proxy = sdbus::createProxy(conn, sdbus::ServiceName{"org.freedesktop.DBus"},
                                        sdbus::ObjectPath{"/org/freedesktop/DBus"});
        bool owned = false;
        proxy->callMethod("NameHasOwner").onInterface("org.freedesktop.DBus").withArguments(name).storeResultsTo(owned);
        return owned;
    } catch (const sdbus::Error &) {
        return false;
    }
}

/// Activate a service by calling org.freedesktop.DBus.StartServiceByName.
auto startServiceByName(sdbus::IConnection &conn, const std::string &name) -> bool {
    try {
        auto proxy = sdbus::createProxy(conn, sdbus::ServiceName{"org.freedesktop.DBus"},
                                        sdbus::ObjectPath{"/org/freedesktop/DBus"});
        uint32_t result = 0;
        proxy->callMethod("StartServiceByName")
            .onInterface("org.freedesktop.DBus")
            .withArguments(name, 0u)
            .storeResultsTo(result);
        return result == 1 || result == 2;
    } catch (const sdbus::Error &) {
        return false;
    }
}

/// Create an ephemeral (no well-known name) bus connection for probing.
auto createProbeConnection(bool system) -> std::shared_ptr<sdbus::IConnection> {
    try {
        auto conn = system ? sdbus::createSystemBusConnection() : sdbus::createSessionBusConnection();
        return std::shared_ptr<sdbus::IConnection>(conn.release());
    } catch (const sdbus::Error &) {
        return nullptr;
    }
}

} // anonymous namespace

auto WellbeingManager::resolveDaemonBus() -> std::optional<WellbeingManager::BusVariant> {
    constexpr auto DAEMON_NAME = "org.wellbeing.v1.Daemon";

    // Probe connection for the system bus.
    auto sysConn = createProbeConnection(true);

    // 1. System bus already has the daemon?
    if (sysConn && nameHasOwner(*sysConn, DAEMON_NAME)) return BusVariant::System;

    // Probe connection for the session bus.
    auto sessConn = createProbeConnection(false);

    // 2. Session bus already has the daemon?
    if (sessConn && nameHasOwner(*sessConn, DAEMON_NAME)) return BusVariant::Session;

    // 3. Activate the SYSTEM daemon.
    if (sysConn && startServiceByName(*sysConn, DAEMON_NAME)) return BusVariant::System;

    // 4. Activate the SESSION daemon.
    if (sessConn && startServiceByName(*sessConn, DAEMON_NAME)) return BusVariant::Session;

    return std::nullopt; // all four steps failed — degraded mode
}

// =============================================================================
// WellbeingManager
// =============================================================================

WellbeingManager::WellbeingManager(std::shared_ptr<LockManager> lockManager,
                                   std::shared_ptr<sdbus::IConnection> connection)
    : m_conn(std::move(connection)),
      m_object(sdbus::createObject(*m_conn, sdbus::ObjectPath{"/org/wellbeing/Manager"})),
      m_lockManager(std::move(lockManager)) {
    m_object
        ->addVTable(sdbus::registerMethod("Overlay").implementedAs(
                        [this](const sdbus::Variant &envelope) -> bool { return handleOverlay(envelope); }),
                    sdbus::registerProperty("CurrentSession").withGetter([]() -> sdbus::Variant {
                        return buildSessionVariant();
                    }),
                    sdbus::registerSignal("UserAction")
                        .withParameters<std::string, uint32_t, uint64_t, uint64_t, std::vector<uint8_t>>(
                            {"app_id", "action", "policy_id", "blocked_since", "signature"}),
                    sdbus::registerSignal("FocusChanged").withParameters<sdbus::Variant>({"window"}),
                    sdbus::registerSignal("ActivityChanged").withParameters<bool>({"idle"}))
        .forInterface("org.wellbeing.v1.Manager");

    // Wire LockManager button clicks → our emitUserAction.
    // Conversion from typed enums to raw D-Bus types happens at this boundary.
    m_lockManager->setUserActionCallback(
        [this](const AppId &appId, ActionType action) -> void { emitUserAction(appId.value(), action); });

    // Reverse-discovery: tell the daemon we exist.
    registerWithDaemon();
}

// ── Reverse discovery ──────────────────────────────────────────────

/// Advertise this plugin instance to the daemon by calling
/// Daemon.RegisterPlugin(instanceId) on the system bus.
/// Logs failure but does not prevent plugin operation — the daemon
/// discovers us via NameOwnerChanged if unavailable at startup.
void WellbeingManager::registerWithDaemon() {
    try {
        auto daemon = sdbus::createProxy(*m_conn, sdbus::ServiceName{"org.wellbeing.v1.Daemon"},
                                         sdbus::ObjectPath{"/org/wellbeing/Daemon"});
        daemon->callMethod("RegisterPlugin").onInterface("org.wellbeing.v1.Daemon").withArguments(instanceId());
    } catch (const sdbus::Error &e) {
        // Daemon may not be running yet — plugin functions without it
        // (focus signals still emit for GUI). Daemon discovers us via
        // NameOwnerChanged. Log the failure instead of swallowing.
        logInfo("registerWithDaemon: daemon not reachable (" + std::string(e.what()) +
                ") — will be discovered via NameOwnerChanged");
    }
}

// ── Signal emission ────────────────────────────────────────────────

/// Emit UserAction(app_id, action, policy_id, blocked_since, signature).
/// Echoes the daemon-issued signed token verbatim for the clicked app.
void WellbeingManager::emitUserAction(const std::string &appId, ActionType action) {
    auto id = AppId::from_unchecked(appId); // validated at D-Bus boundary in tryShowOverlay
    // Convert ActionType → uint32_t at the D-Bus emission boundary.
    m_object->emitSignal("UserAction")
        .onInterface("org.wellbeing.v1.Manager")
        .withArguments(appId, static_cast<uint32_t>(action), m_lockManager->activePolicyId(id),
                       m_lockManager->blockedSince(id), m_lockManager->activeSignature(id));
}

/// Emit FocusChanged(variant) reflecting the current focused window.
void WellbeingManager::emitFocusChanged(const std::optional<WindowInfo> &info) {
    m_object->emitSignal("FocusChanged")
        .onInterface("org.wellbeing.v1.Manager")
        .withArguments(windowInfoToVariant(info));
}

/// Emit ActivityChanged(idle) when user activity state changes.
void WellbeingManager::emitActivityChanged(bool idle) {
    m_object->emitSignal("ActivityChanged").onInterface("org.wellbeing.v1.Manager").withArguments(idle);
}

// ── Instance identity ──────────────────────────────────────────────

/// Return a stable unique id for this plugin instance.
/// Format: "<uid>@<session>" where <session> is the logind session ID
/// from environment (XDG_SESSION_ID) or a fallback.
auto WellbeingManager::instanceId() -> std::string {
    // TODO: use logind session for a stable, unique id.
    const char *uidStr = std::getenv("UID");
    const char *sess = std::getenv("XDG_SESSION_ID");
    std::string result;
    result += (uidStr != nullptr) ? uidStr : "0";
    result += "@";
    result += (sess != nullptr) ? sess : "unknown";
    return result;
}

/// Return a D-Bus well-known bus name for this instance.
/// Claimed via sdbus::createSystemBusConnection(wellKnownName).
auto WellbeingManager::wellKnownBusName() -> std::string { return "org.wellbeing.v1.Manager." + instanceId(); }

// ── Overlay handler (dispatches to show/hide helpers) ──────────────

auto WellbeingManager::handleOverlay(const sdbus::Variant &envelope) -> bool {
    // Parse SignedEnvelope { payload(v), issued_at(t), signature(ay) }
    sdbus::Variant payload;
    uint64_t issuedAt = 0;
    std::vector<uint8_t> sig;

    try {
        auto env = envelope.get<std::tuple<sdbus::Variant, uint64_t, std::vector<uint8_t>>>();
        payload = std::get<0>(env);
        issuedAt = std::get<1>(env);
        sig = std::get<2>(env);
    } catch (const sdbus::Error &) {
        logErr("handleOverlay: malformed envelope");
        return false;
    }

    // Verify Ed25519 signature + freshness window (FAILS CLOSED).
    if (!verifyEnvelope(*m_conn, payload, issuedAt, sig)) {
        return false;
    }

    // Dispatch on inner payload variant tag via try/catch probe.
    return tryShowOverlay(payload) || tryHideOverlay(payload);
}

/// Attempt to parse and apply a "show" overlay variant.
/// Returns true on success, false if payload is not a show variant.
auto WellbeingManager::tryShowOverlay(sdbus::Variant &payload) -> bool {
    try {
        auto show = payload.get<
            std::tuple<std::string, uint64_t, uint32_t, uint64_t, std::vector<uint32_t>, std::vector<uint8_t>>>();
        std::string rawAppId = std::get<0>(show);
        uint64_t policyId = std::get<1>(show);
        uint32_t reason = std::get<2>(show);
        uint64_t blockedSince = std::get<3>(show);
        std::vector<uint32_t> actions = std::get<4>(show);
        std::vector<uint8_t> innerSig = std::get<5>(show);

        // ── Zero-Trust Boundary Gate ──────────────────────────────────
        // Validate ALL D-Bus-deserialized values before they enter domain
        // logic. Reject the entire command if any value is out of range.
        auto appId = AppId::from_raw(rawAppId);
        if (!appId.has_value()) {
            logErr("tryShowOverlay: invalid (empty/null) appId rejected");
            return false;
        }

        auto br = wellbeing::raw_to_block_reason(reason);
        if (!br.has_value()) {
            logErr("tryShowOverlay: invalid BlockReason " + std::to_string(reason) + " rejected");
            return false;
        }

        std::vector<ActionType> typedActions;
        typedActions.reserve(actions.size());
        for (const auto &act : actions) {
            auto at = wellbeing::raw_to_action_type(act);
            if (!at.has_value()) {
                logErr("tryShowOverlay: invalid ActionType " + std::to_string(act) + " rejected");
                return false;
            }
            typedActions.push_back(*at);
        }

        m_lockManager->showOverlay(*appId, policyId, *br, blockedSince, typedActions, innerSig);

        // Update cached WindowInfo overlay_shown flag.
        if (g_ctx->currentFocus.has_value() && g_ctx->currentFocus->appId == *appId) {
            g_ctx->currentFocus->overlayShown = true;
        }
        return true;
    } catch (const sdbus::Error &) {
        return false; // not a show variant
    }
}

/// Attempt to parse and apply a "hide" overlay variant.
/// Returns true on success, false if payload is not a hide variant.
auto WellbeingManager::tryHideOverlay(sdbus::Variant &payload) -> bool {
    try {
        auto rawAppId = payload.get<std::string>();

        auto appId = AppId::from_raw(rawAppId);
        if (!appId.has_value()) {
            logErr("tryHideOverlay: invalid (empty/null) appId rejected");
            return false;
        }

        LockManagerError err = m_lockManager->hideOverlay(*appId);
        if (err != LockManagerError::None) {
            logInfo("tryHideOverlay: hideOverlay returned error for appId=" + rawAppId);
            // Return true anyway — the overlay is in a consistent state
            // (the caller asked to hide, and the overlay is effectively
            // not shown for the requested app).
        }

        if (g_ctx->currentFocus.has_value() && g_ctx->currentFocus->appId == *appId) {
            g_ctx->currentFocus->overlayShown = false;
        }
        return true;
    } catch (const sdbus::Error &) {
        logErr("tryHideOverlay: malformed hide payload");
        return false;
    }
}
