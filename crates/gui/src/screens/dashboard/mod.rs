//! Dashboard screen — daily usage overview with charts and app list.
//!
//! Data flow: D-Bus cache → `build_dashboard_viewmodel()` → `DashboardViewModel`
//! → `render_dashboard_view()` (gpui element tree).
//!
//! Each chart/component (BarChart, PieChart, AppList, BlockCard) implements
//! `Render` for View-based usage, plus a convenience rendering helper for
//! inline use in the dashboard layout.

use chrono::{DateTime, NaiveDate, Utc};
use gpui::prelude::*;
use gpui::px;
use gpui::*;
use gpui_component::chart::{BarChart, PieChart};
use gpui_component::{h_flex, v_flex};
use wellbeing_core::{AppCategoryRow, Category, DailyUsageEntry};

use crate::components::{card, stat_card};
use crate::theme::{self, rad, resolve_color, sp};
use gpui::Hsla;
use gpui_component::ActiveTheme;

// ---------------------------------------------------------------------------
// Data types (pure data, no gpui)
// ---------------------------------------------------------------------------

/// One bar in the daily screen-time bar chart.
#[derive(Debug, Clone)]
pub struct Bar {
    pub date: NaiveDate,
    pub total_minutes: f64,
}

/// One slice in a usage breakdown pie chart.
#[derive(Debug, Clone)]
pub struct Slice {
    pub app_id: String,
    pub display_name: String,
    pub color: String,
    pub percentage: f64,
}

/// A single row in the top-apps list.
#[derive(Debug, Clone)]
pub struct AppListEntry {
    pub rank: usize,
    pub app_id: String,
    pub display_name: String,
    pub total_minutes: i64,
    pub percentage: f64,
    pub category_color: Option<String>,
    pub is_blocked: bool,
}

/// Information about a currently-blocked application.
#[derive(Debug, Clone)]
pub struct BlockCardInfo {
    pub app_id: String,
    pub display_name: String,
    pub blocked_since: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// ViewModel
// ---------------------------------------------------------------------------

/// Pure-data ViewModel for the Dashboard screen.
///
/// No gpui types, `Send + 'static`.  Built by `build_dashboard_viewmodel()`
/// from raw D-Bus cache data and consumed by gpui component constructors.
#[derive(Debug, Clone)]
pub struct DashboardViewModel {
    /// Inclusive date range for the current view.
    pub date_range: (NaiveDate, NaiveDate),
    /// Per-day total screen time (bar chart data).
    pub bar_chart: Vec<Bar>,
    /// Per-app usage breakdown (pie chart data).
    pub pie_app: Vec<Slice>,
    /// Per-category usage breakdown (pie chart data).
    pub pie_category: Vec<Slice>,
    /// Top N apps sorted by total usage.
    pub top_apps: Vec<AppListEntry>,
    /// Currently-blocked apps (info-only cards).
    pub block_cards: Vec<BlockCardInfo>,
}

// ---------------------------------------------------------------------------
// ViewModel builder (pure function, no gpui, no async)
// ---------------------------------------------------------------------------

/// Transform D-Bus cache data into a `DashboardViewModel`.
pub fn build_dashboard_viewmodel(
    usage: &[DailyUsageEntry],
    categories: &[Category],
    app_categories: &[AppCategoryRow],
) -> DashboardViewModel {
    let (start, end) = date_range_from_usage(usage);
    let bar_chart = build_bars(usage);
    let pie_app = build_app_slices(usage);
    let pie_category = build_category_slices(usage, categories, app_categories);
    let top_apps = build_top_apps(usage, app_categories);

    DashboardViewModel {
        date_range: (start, end),
        bar_chart,
        pie_app,
        pie_category,
        top_apps,
        block_cards: Vec::new(),
    }
}

/// Determine the inclusive date range from usage entries (defaults to today).
fn date_range_from_usage(usage: &[DailyUsageEntry]) -> (NaiveDate, NaiveDate) {
    let today = Utc::now().date_naive();
    if usage.is_empty() {
        return (today, today);
    }

    let mut min = NaiveDate::MAX;
    let mut max = NaiveDate::MIN;
    for entry in usage {
        if let Ok(d) = NaiveDate::parse_from_str(&entry.date, "%Y-%m-%d") {
            if d < min {
                min = d;
            }
            if d > max {
                max = d;
            }
        }
    }

    if min > max {
        (today, today)
    } else {
        (min, max)
    }
}

/// Build per-day bar chart data from usage entries.
fn build_bars(usage: &[DailyUsageEntry]) -> Vec<Bar> {
    let mut by_date: std::collections::BTreeMap<String, f64> = std::collections::BTreeMap::new();
    for entry in usage {
        let total_minutes = entry.total_seconds as f64 / 60.0;
        *by_date.entry(entry.date.clone()).or_insert(0.0) += total_minutes;
    }

    by_date
        .into_iter()
        .filter_map(|(date_str, total_minutes)| {
            NaiveDate::parse_from_str(&date_str, "%Y-%m-%d")
                .ok()
                .map(|date| Bar {
                    date,
                    total_minutes,
                })
        })
        .collect()
}

/// Build per-app pie slices sorted by usage descending.
fn build_app_slices(usage: &[DailyUsageEntry]) -> Vec<Slice> {
    let mut by_app: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
    let mut total: f64 = 0.0;
    for entry in usage {
        let minutes = entry.total_seconds as f64 / 60.0;
        *by_app.entry(entry.app_id.clone()).or_insert(0.0) += minutes;
        total += minutes;
    }

    if total <= 0.0 {
        return Vec::new();
    }

    let mut slices: Vec<Slice> = by_app
        .into_iter()
        .map(|(app_id, app_minutes)| Slice {
            percentage: (app_minutes / total) * 100.0,
            app_id,
            display_name: String::new(),
            color: String::new(),
        })
        .collect();

    slices.sort_by(|a, b| {
        b.percentage
            .partial_cmp(&a.percentage)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    slices
}

/// Build per-category pie slices by cross-referencing usage with categories.
fn build_category_slices(
    usage: &[DailyUsageEntry],
    categories: &[Category],
    app_categories: &[AppCategoryRow],
) -> Vec<Slice> {
    let app_to_cat: std::collections::HashMap<&str, i64> = app_categories
        .iter()
        .map(|ac| (ac.app_id.as_str(), ac.category_id))
        .collect();

    let cat_map: std::collections::HashMap<i64, &Category> =
        categories.iter().map(|c| (c.id.0, c)).collect();

    let mut by_cat: std::collections::HashMap<String, (f64, String)> =
        std::collections::HashMap::new();
    let mut total: f64 = 0.0;
    for entry in usage {
        let minutes = entry.total_seconds as f64 / 60.0;
        let cat_id = app_to_cat.get(entry.app_id.as_str()).copied().unwrap_or(0);
        let (cat_name, cat_color) = cat_map
            .get(&cat_id)
            .map(|c| (c.name.clone(), c.color.clone()))
            .unwrap_or_else(|| ("Uncategorized".into(), String::new()));
        let entry = by_cat.entry(cat_name).or_insert((0.0, cat_color));
        entry.0 += minutes;
        total += minutes;
    }

    if total <= 0.0 {
        return Vec::new();
    }

    let mut slices: Vec<Slice> = by_cat
        .into_iter()
        .map(|(name, (cat_minutes, color))| Slice {
            percentage: (cat_minutes / total) * 100.0,
            app_id: name.clone(),
            display_name: name,
            color,
        })
        .collect();

    slices.sort_by(|a, b| {
        b.percentage
            .partial_cmp(&a.percentage)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    slices
}

/// Build the top-apps list sorted by total usage descending.
fn build_top_apps(
    usage: &[DailyUsageEntry],
    app_categories: &[AppCategoryRow],
) -> Vec<AppListEntry> {
    let app_meta: std::collections::HashMap<&str, (&str, Option<String>)> = app_categories
        .iter()
        .map(|ac| {
            let color = if ac.category_id > 0 {
                Some(format!("cat_{}", ac.category_id))
            } else {
                None
            };
            (ac.app_id.as_str(), (ac.display_name.as_str(), color))
        })
        .collect();

    let mut by_app: std::collections::BTreeMap<String, i64> = std::collections::BTreeMap::new();
    let mut grand_total: f64 = 0.0;
    for entry in usage {
        *by_app.entry(entry.app_id.clone()).or_insert(0) += entry.total_seconds;
        grand_total += entry.total_seconds as f64;
    }

    if grand_total <= 0.0 {
        return Vec::new();
    }

    let mut entries: Vec<AppListEntry> = by_app
        .into_iter()
        .map(|(app_id, total_seconds)| {
            let display_name = app_meta
                .get(app_id.as_str())
                .map(|(name, _)| name.to_string())
                .unwrap_or_else(|| app_id.clone());
            let category_color = app_meta
                .get(app_id.as_str())
                .and_then(|(_, color)| color.clone());
            AppListEntry {
                rank: 0,
                total_minutes: total_seconds / 60,
                percentage: (total_seconds as f64 / grand_total) * 100.0,
                app_id,
                display_name,
                category_color,
                is_blocked: false,
            }
        })
        .collect();

    entries.sort_by_key(|a| std::cmp::Reverse(a.total_minutes));
    for (i, entry) in entries.iter_mut().enumerate() {
        entry.rank = i + 1;
    }

    entries.truncate(10);
    entries
}

// ---------------------------------------------------------------------------
// Color resolution (delegates to the shared design system in `crate::theme`)
// ---------------------------------------------------------------------------

fn parse_hex(hex: &str) -> Option<Hsla> {
    theme::parse_hex(hex)
}

// ---------------------------------------------------------------------------
// Charts & panels (real `gpui_component::chart` + shared `Card` primitives)
// ---------------------------------------------------------------------------

/// Format minutes into a human-readable duration string.
fn format_duration(total_minutes: i64) -> String {
    if total_minutes < 60 {
        format!("{}m", total_minutes)
    } else {
        let hours = total_minutes / 60;
        let mins = total_minutes % 60;
        if mins == 0 {
            format!("{}h", hours)
        } else {
            format!("{}h {}m", hours, mins)
        }
    }
}

/// Compute the KPI summary for the stat row.
struct Kpis {
    total_minutes: i64,
    top_app: String,
    top_app_minutes: i64,
    active_blocks: usize,
}

fn compute_kpis(vm: &DashboardViewModel) -> Kpis {
    let total_seconds: i64 = vm
        .bar_chart
        .iter()
        .map(|b| (b.total_minutes * 60.0) as i64)
        .sum();
    let top = vm.top_apps.first();
    Kpis {
        total_minutes: total_seconds / 60,
        top_app: top
            .map(|t| t.display_name.clone())
            .unwrap_or_else(|| "—".into()),
        top_app_minutes: top.map(|t| t.total_minutes).unwrap_or(0),
        active_blocks: vm.block_cards.len(),
    }
}

/// Render the complete dashboard view from a ViewModel.
pub fn render_dashboard_view(cx: &App, vm: &DashboardViewModel) -> impl IntoElement {
    let date_range_text = format!(
        "{} — {}",
        vm.date_range.0.format("%b %d"),
        vm.date_range.1.format("%b %d, %Y"),
    );

    let kpis = compute_kpis(vm);

    v_flex()
        .gap_4()
        .child(
            // Sub-header: date range
            h_flex().gap_3().items_center().child(
                div()
                    .text_xs()
                    .px(sp::XS)
                    .py(px(2.0))
                    .rounded(rad::sm())
                    .bg(theme::accent(cx))
                    .text_color(cx.theme().accent_foreground)
                    .child(date_range_text),
            ),
        )
        // KPI stat row
        .child(
            h_flex()
                .gap_4()
                .child(stat_card(
                    cx,
                    &format_duration(kpis.total_minutes),
                    "Total Screen Time",
                    Some(theme::accent(cx)),
                ))
                .child(stat_card(
                    cx,
                    &kpis.top_app,
                    &format!("Top App · {}", format_duration(kpis.top_app_minutes)),
                    Some(theme::info(cx)),
                ))
                .child(stat_card(
                    cx,
                    &kpis.active_blocks.to_string(),
                    "Active Blocks",
                    Some(theme::danger(cx)),
                )),
        )
        // Daily bar chart
        .child(card(
            cx,
            Some("Daily Screen Time"),
            vec![daily_bar_chart(cx, &vm.bar_chart).into_any_element()],
        ))
        // Two-up: pie by app / pie by category
        .child(
            h_flex()
                .gap_4()
                .child(div().flex_1().child(card(
                    cx,
                    Some("By App"),
                    vec![pie_chart_panel(cx, &vm.pie_app, false).into_any_element()],
                )))
                .child(div().flex_1().child(card(
                    cx,
                    Some("By Category"),
                    vec![pie_chart_panel(cx, &vm.pie_category, true).into_any_element()],
                ))),
        )
        // Top apps
        .child(card(
            cx,
            Some("Top Apps"),
            vec![app_list_panel(cx, &vm.top_apps).into_any_element()],
        ))
        // Blocked cards
        .when(!vm.block_cards.is_empty(), |el| {
            el.child(card(
                cx,
                Some("Currently Blocked"),
                vm.block_cards
                    .iter()
                    .map(|c| block_card(cx, c).into_any_element())
                    .collect::<Vec<_>>(),
            ))
        })
}

// ── Daily bar chart ──────────────────────────────────────────────────────────

fn daily_bar_chart(cx: &App, bars: &[Bar]) -> AnyElement {
    if bars.is_empty() {
        return empty_state(cx, "No usage data for the selected range.").into_any_element();
    }

    let accent = theme::accent(cx);
    div()
        .h(px(180.0))
        .child(
            BarChart::new(bars.to_vec())
                .band(|b: &Bar| b.date.format("%m/%d").to_string())
                .value(|b: &Bar| b.total_minutes)
                .label(|b: &Bar| SharedString::from(format!("{:.0}m", b.total_minutes)))
                .fill(move |_b, _bar, _chart, _align| accent)
                .label_axis(true),
        )
        .into_any_element()
}

// ── Pie chart panel (donut + legend) ─────────────────────────────────────────

fn pie_chart_panel(cx: &App, slices: &[Slice], by_category: bool) -> AnyElement {
    if slices.is_empty() {
        return empty_state(cx, "No data.").into_any_element();
    }

    let accent = theme::accent(cx);
    let chart = div()
        .h(px(200.0))
        .child(
            PieChart::new(slices.to_vec())
                .value(|s: &Slice| s.percentage as f32)
                .color(|s: &Slice| resolve_color(&s.color, &s.display_name))
                .inner_radius(0.55)
                .outer_radius(0.95)
                .label(|s: &Slice| {
                    let display = if s.display_name.is_empty() {
                        &s.app_id
                    } else {
                        &s.display_name
                    };
                    SharedString::from(format!("{} {:.0}%", display, s.percentage))
                }),
        )
        .into_any_element();

    // Legend
    let legend = v_flex()
        .gap_1()
        .children(slices.iter().map(|s| {
            let color = resolve_color(&s.color, &s.display_name);
            let display = if s.display_name.is_empty() {
                &s.app_id
            } else {
                &s.display_name
            };
            h_flex()
                .gap_2()
                .items_center()
                .child(div().size(px(10.0)).rounded(rad::full()).bg(color))
                .child(
                    div()
                        .text_xs()
                        .text_color(theme::text_secondary(cx))
                        .child(format!("{}  {:.0}%", display, s.percentage)),
                )
                .into_any_element()
        }))
        .into_any_element();

    let _ = by_category;
    let _ = accent;
    v_flex()
        .gap_2()
        .child(chart)
        .child(legend)
        .into_any_element()
}

// ── Top apps list ─────────────────────────────────────────────────────────────

fn app_list_panel(cx: &App, entries: &[AppListEntry]) -> AnyElement {
    if entries.is_empty() {
        return empty_state(cx, "No usage data yet.").into_any_element();
    }

    let rows: Vec<AnyElement> = entries
        .iter()
        .map(|entry| {
            let blocked_color = if entry.is_blocked {
                theme::danger(cx)
            } else {
                theme::success(cx)
            };
            let cat_color = entry
                .category_color
                .as_deref()
                .and_then(|c| parse_hex(c.trim_start_matches("cat_")))
                .unwrap_or_else(|| theme::text_muted(cx));

            h_flex()
                .px(sp::MD)
                .py(sp::SM)
                .rounded(rad::md())
                .hover(|s| s.bg(theme::border(cx)))
                .gap_4()
                .items_center()
                .child(
                    div()
                        .text_xs()
                        .text_color(theme::text_muted(cx))
                        .w(px(28.0))
                        .child(format!("#{}", entry.rank)),
                )
                .child(div().size(px(10.0)).rounded(rad::full()).bg(cat_color))
                .child(
                    div()
                        .text_sm()
                        .flex_1()
                        .text_color(theme::text_primary(cx))
                        .child(entry.display_name.clone()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(blocked_color)
                        .child(if entry.is_blocked { "BLOCKED" } else { "" }.to_string()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme::text_secondary(cx))
                        .child(format!("{:.1}%", entry.percentage)),
                )
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::BOLD)
                        .text_color(theme::text_primary(cx))
                        .child(format_duration(entry.total_minutes)),
                )
                .into_any_element()
        })
        .collect();

    v_flex().gap_1().children(rows).into_any_element()
}

// ── Blocked app card ──────────────────────────────────────────────────────────

fn block_card(cx: &App, info: &BlockCardInfo) -> AnyElement {
    let now = Utc::now();
    let duration = now.signed_duration_since(info.blocked_since);
    let ago = if duration.num_minutes() < 1 {
        "just now".to_string()
    } else if duration.num_minutes() < 60 {
        format!("{} minutes ago", duration.num_minutes())
    } else if duration.num_hours() < 24 {
        format!("{} hours ago", duration.num_hours())
    } else {
        format!("{} days ago", duration.num_days())
    };

    let display = if info.display_name.is_empty() {
        &info.app_id
    } else {
        &info.display_name
    };

    h_flex()
        .gap_3()
        .items_center()
        .child(div().size(px(10.0)).rounded(rad::full()).bg(theme::danger(cx)))
        .child(
            v_flex()
                .gap_1()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::BOLD)
                        .text_color(theme::text_primary(cx))
                        .child(format!("{} — Blocked {}", display, ago)),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme::text_secondary(cx))
                        .child("Daily limit reached. Switch to the window and use the overlay controls to continue."),
                ),
        )
        .into_any_element()
}

// ── Shared empty state ────────────────────────────────────────────────────────

fn empty_state(cx: &App, message: &str) -> AnyElement {
    div()
        .py(sp::LG)
        .w_full()
        .items_center()
        .justify_center()
        .child(
            div()
                .text_sm()
                .text_color(theme::text_muted(cx))
                .child(message.to_string()),
        )
        .into_any_element()
}
