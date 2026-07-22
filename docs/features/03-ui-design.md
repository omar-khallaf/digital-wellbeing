# UI Design — gpui-component Screens

The GUI renders with gpui's retained-mode component tree. All data arrives via
the in-memory cache; this file covers screen layout, component identification,
view models, and the read paths that build them. No raw database access happens
in the GUI — all data flows through D-Bus and cache invalidation is driven by
reactive notifications.

## Screen Layout

The dashboard uses a top tab bar to switch between three screens: Dashboard,
Policies, and Reports. On narrow viewports the tab bar moves to the bottom; on
medium and wide viewports it stays at the top. A TimeRangeSelector component
appears on the Dashboard and Reports screens to control the date window.

The visible screens are:

- Dashboard — usage overview with charts, per-app and per-category breakdowns,
  top apps, and any currently active block cards.
- Policies — app and category policy management with editor forms.
- Reports — usage history and export for the selected time range.

## Component Mapping

Dashboard renders BarChart, PieChart, AppList, and BlockCard widgets. Policies
renders AppSelector input plus the PolicyEditor and CategoryEditor forms.
Reports renders TimeRangeSelector plus BarChart, PieChart, and ExportDialog.

## TimeRangeSelector

This shared component renders a row of preset buttons — 7d, 30d, 90d — plus a
Custom button that opens a date-picker in range mode. Selecting any preset or
confirming a custom range updates the app state date range, which triggers a
re-fetch of usage data and a ViewModel rebuild. The underlying DateRange type
validates that start is less than or equal to end at construction time and
computes presets relative to today.

## Dashboard Screen

### Total Screen Time

The dashboard shows daily totals as a simple column chart with day labels on the
X-axis and hours on the Y-axis. Each bar represents one day in the selected
range. The chart is built from daily summary groups returned by the usage range
query.

### Per-App Screen Time

A pie chart shows the top apps by usage in the selected date range. Each slice
is labeled with the app display name, colored by category color when available,
and annotated with the percentage of total time. Clicking a slice filters other
visualizations on the dashboard to that specific app.

### Per-Category Screen Time

A second pie chart groups the same usage by category instead of by app. It joins
daily usage with app_categories and categories to obtain category names and
colors, then aggregates usage per category for the selected date range.

### Top Apps List

A ranked list shows the top applications by usage in the selected date range,
displaying rank, display name, icon path when available, total minutes,
percentage of total time, category color, and a blocked indicator. The list is
sortable by time, alphabetically, or by category.

### Currently Blocked Apps

A collapsible card renders for each currently blocked app, showing the app name,
how long the block has been active, and which daily limit was reached. Block
resolution is handled exclusively by the compositor overlay; the dashboard
displays information only and cannot grant time or close an app.

## Policies Screen

### App Selector

A combined free-text input and select dropdown lets the user choose an app_id
for policy targeting. Typing filters the dropdown of known app IDs from the
event log, and typing a new app ID followed by Enter creates a new target.

### Policy Configuration Forms

Policy data is constructed from database rows into domain types before the UI
sees it. The Policy enum distinguishes App and Category targets. Each target
carries an id, name, and a PolicyConfig that specifies kind, optional
time_limit_minutes, extra_minutes, and schedule.

Block kind is unconditional; it blocks whenever the policy is active and has no
time limit. TimeLimit kind blocks when elapsed minutes reach or exceed
time_limit_minutes. Notify kind never blocks; it only alerts when the limit is
reached.

TimeWindow wraps schedule rules, which can be either daily (start and end times
each day) or weekly (a set of weekdays plus start and end times). Empty or null
schedule rules mean the policy is active all the time.

The editor renders labeled controls for kind, target, time limit, extra minutes,
schedule rules, and active flag. Time_limit_minutes is shown only when kind is
TimeLimit or Notify; extra minutes are shown only for TimeLimit kind.

Categories are managed through a separate editor that lets users create, rename,
assign colors, and assign icons. The editor operates on the categories table
directly and on app_categories rows to map apps to categories.

## Reports Screen

### Usage Charts

The reports screen shows the same chart shapes as the dashboard but for the
selected report range. Total minutes are computed from daily_usage over the date
range, and per-app or per-category breakdowns are built from joins equivalent to
the dashboard read paths.

### Export

An export dialog writes the current range's DailySummary data to CSV or JSON.
The data comes from the same daemon queries the reports charts use, so export
always matches what is shown.

## View Models

The GUI constructs ViewModels from two sources: the cache of recent D-Bus method
responses, and reactive signals that invalidate cache entries. ViewModels are
pure data structures assembled each render frame; gpui rendering consumes them
without touching D-Bus or actors.

DashboardViewModel carries the selected date range, bar chart data, per-app and
per-category pie slices, the top apps list, and any block cards. It is built
from DailySummary groups returned by GetUsageRange.

ReportsViewModel carries the date range, bar chart data, pie slices, total
minutes, and the top app identifier. It is also built from DailySummary groups.

PoliciesViewModel carries the distinct app list available for targeting, the
currently selected policy target and its editable configuration, all categories,
and any validation errors from the editor forms. The selected policy target is
either an AppId or a CategoryId with its apps.

## DB Read Paths for Screens

All screen data originates from daemon-side data modules. The dashboard issues
GetUsageRange(start, end, uid) and receives DailySummary groups. The policy
screen reads distinct app_ids from the events table and all categories and
app_categories rows for the user. The reports screen issues the same
GetUsageRange call as the dashboard.

Daily totals for the selected date range come from summing total_minutes in
daily_usage grouped by date. Per-app breakdown adds display_name, icon_path,
category name, and category color by left joining app_categories and categories.
Per-category breakdown joins through app_categories to categories and sums per
category. Distinct app_ids for the policy selector are read from events where
app_id is not null.
