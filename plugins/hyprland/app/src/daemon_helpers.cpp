// =============================================================================
// Daemon bus resolution helpers — implementation.
//
// Extracted from wellbeing_manager.cpp to separate shared D-Bus utility
// code from the WellbeingManager class orchestration.
// =============================================================================

#include "daemon_helpers.hpp"

#include <cstdint>
#include <optional>
#include <string>

#include <sdbus-c++/sdbus-c++.h>

#include "types.hpp"

namespace wellbeing {

auto windowInfoToVariant(const std::optional<WindowInfo> &info, bool blocked) -> sdbus::Variant {
    if (!info.has_value()) {
        return sdbus::Variant{static_cast<uint32_t>(FocusVariantTag::Desktop)};
    }
    auto tag = blocked ? FocusVariantTag::Blocked : FocusVariantTag::App;
    return sdbus::Variant{sdbus::Struct{
        static_cast<uint32_t>(tag),
        info->appId.value(),
        info->title,
        info->pid,
        info->uid,
    }};
}

auto nameHasOwner(sdbus::IConnection &conn, const std::string &name) -> bool {
    try {
        auto proxy = sdbus::createProxy(conn, sdbus::ServiceName{DBUS_INTERFACE},
                                        sdbus::ObjectPath{DBUS_OBJECT_PATH});
        bool owned = false;
        proxy->callMethod(NAME_HAS_OWNER_METHOD)
            .onInterface(DBUS_INTERFACE)
            .withArguments(name)
            .storeResultsTo(owned);
        return owned;
    } catch (const sdbus::Error &) {
        return false;
    }
}

auto startServiceByName(sdbus::IConnection &conn, const std::string &name) -> bool {
    try {
        auto proxy = sdbus::createProxy(conn, sdbus::ServiceName{DBUS_INTERFACE},
                                        sdbus::ObjectPath{DBUS_OBJECT_PATH});
        uint32_t result = 0;
        proxy->callMethod(START_SERVICE_BY_NAME_METHOD)
            .onInterface(DBUS_INTERFACE)
            .withArguments(name, 0U)
            .storeResultsTo(result);
        return result == 1 || result == 2;
    } catch (const sdbus::Error &) {
        return false;
    }
}

} // namespace wellbeing
