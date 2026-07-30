use gpui::prelude::*;
use gpui::px;
use gpui::*;
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::input::{Input, InputState};
use gpui_component::{h_flex, v_flex};

use wellbeing_core::TimeWindow;

use crate::app::App as GuiApp;
use crate::theme;

use crate::policies::domain::PolicyConfigForm;

#[cfg(feature = "gui-gpui")]
pub(crate) struct ScheduleSection {
    pub entity: Entity<GuiApp>,
    pub form: PolicyConfigForm,
    pub schedule_start_hour: Entity<InputState>,
    pub schedule_start_minute: Entity<InputState>,
    pub schedule_end_hour: Entity<InputState>,
    pub schedule_end_minute: Entity<InputState>,
}

#[cfg(feature = "gui-gpui")]
impl RenderOnce for ScheduleSection {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let entity = self.entity;
        let form = self.form;
        let schedule_start_hour = self.schedule_start_hour;
        let schedule_start_minute = self.schedule_start_minute;
        let schedule_end_hour = self.schedule_end_hour;
        let schedule_end_minute = self.schedule_end_minute;
        const DAY_NAMES: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

        let day_label = |mask: u8| -> String {
            if mask == 0x7F {
                "All".into()
            } else if mask == 0 {
                "—".into()
            } else {
                DAY_NAMES
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| mask & (1 << i) != 0)
                    .map(|(_, l)| *l)
                    .collect::<Vec<_>>()
                    .join(",")
            }
        };

        // Clone schedules so we can iterate without holding the borrow.
        let windows: Vec<TimeWindow> = form.schedules.clone();

        let window_rows: Vec<AnyElement> = windows
            .iter()
            .enumerate()
            .map(|(idx, w)| {
                let label = day_label(w.day_mask);
                let start = format!("{:02}:{:02}", w.start_minute / 60, w.start_minute % 60);
                let end = format!("{:02}:{:02}", w.end_minute / 60, w.end_minute % 60);
                let remove_idx = idx;
                let entity = entity.clone();
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::text_primary(cx))
                            .child(format!("{label}  {start} → {end}")),
                    )
                    .child({
                        let entity = entity.clone();
                        Button::new(format!("rm-sched-{idx}"))
                            .label("×")
                            .danger()
                            .on_click(move |_, _window, app| {
                                entity.update(app, |this, cx2| {
                                    if let Some((_, ref mut f)) = this.policy_edit
                                        && remove_idx < f.schedules.len()
                                    {
                                        f.schedules.remove(remove_idx);
                                    }
                                    cx2.notify();
                                });
                            })
                    })
                    .into_any_element()
            })
            .collect();

        let empty_hint: Vec<AnyElement> = if window_rows.is_empty() {
            vec![
                div()
                    .text_xs()
                    .text_color(theme::text_muted(cx))
                    .child("Always active")
                    .into_any_element(),
            ]
        } else {
            window_rows
        };

        let current_day_mask = form.schedule_new_day_mask;

        let day_buttons: Vec<AnyElement> = DAY_NAMES
            .iter()
            .enumerate()
            .map(|(i, label)| {
                let bit = 1u8 << i;
                let is_set = current_day_mask & bit != 0;
                let entity = entity.clone();
                Button::new(format!("day-toggle-{i}"))
                    .label(*label)
                    .when(is_set, |b| b.primary())
                    .on_click(move |_, _window, app| {
                        entity.update(app, |this, cx2| {
                            if let Some((_, ref mut f)) = this.policy_edit {
                                f.schedule_new_day_mask ^= bit;
                            }
                            cx2.notify();
                        });
                    })
                    .into_any_element()
            })
            .collect();

        v_flex()
            .gap_2()
            .child(
                div()
                    .text_xs()
                    .text_color(theme::text_primary(cx))
                    .child("Schedule (when this policy applies)"),
            )
            .children(empty_hint)
            .child(h_flex().gap_1().items_center().children(day_buttons))
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::text_primary(cx))
                            .child("Start:"),
                    )
                    .child(div().w(px(50.0)).child(Input::new(&schedule_start_hour)))
                    .child(div().text_xs().child(":"))
                    .child(div().w(px(50.0)).child(Input::new(&schedule_start_minute)))
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::text_primary(cx))
                            .child("End:"),
                    )
                    .child(div().w(px(50.0)).child(Input::new(&schedule_end_hour)))
                    .child(div().text_xs().child(":"))
                    .child(div().w(px(50.0)).child(Input::new(&schedule_end_minute))),
            )
            .child({
                let entity = entity.clone();
                Button::new("add-schedule-window")
                    .label("Add Window")
                    .on_click(move |_, _window, app| {
                        let (sh, sm, eh, em, dm) = {
                            let me = entity.read(app);
                            let sh = me
                                .schedule_start_hour
                                .as_ref()
                                .and_then(|e| e.read(app).value().parse::<u16>().ok())
                                .unwrap_or(0)
                                .min(23);
                            let sm = me
                                .schedule_start_minute
                                .as_ref()
                                .and_then(|e| e.read(app).value().parse::<u16>().ok())
                                .unwrap_or(0)
                                .min(59);
                            let eh = me
                                .schedule_end_hour
                                .as_ref()
                                .and_then(|e| e.read(app).value().parse::<u16>().ok())
                                .unwrap_or(0)
                                .min(23);
                            let em = me
                                .schedule_end_minute
                                .as_ref()
                                .and_then(|e| e.read(app).value().parse::<u16>().ok())
                                .unwrap_or(0)
                                .min(59);
                            let dm = me
                                .policy_edit
                                .as_ref()
                                .map(|(_, f)| f.schedule_new_day_mask)
                                .unwrap_or(0x7F);
                            (sh, sm, eh, em, dm)
                        };
                        entity.update(app, |this, cx2| {
                            let start_minute = sh * 60 + sm;
                            let end_minute = eh * 60 + em;
                            if start_minute == end_minute {
                                return;
                            }
                            if let Some((_, ref mut f)) = this.policy_edit {
                                f.schedules.push(TimeWindow {
                                    start_minute,
                                    end_minute,
                                    day_mask: dm,
                                });
                                f.schedule_new_day_mask = 0x7F;
                            }
                            cx2.notify();
                        });
                    })
            })
            .into_any_element()
    }
}
