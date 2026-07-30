//! Dashboard sub-panels — app list, title list, and block cards.

use chrono::Utc;
use gpui::prelude::*;
use gpui::*;
use gpui_component::{h_flex, v_flex};

use crate::theme::{self, rad};

use crate::dashboard::domain::BlockCardInfo;

/// Blocked-application notification card.
pub struct BlockCard {
    pub info: BlockCardInfo,
}

impl RenderOnce for BlockCard {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let now = Utc::now();
        let duration = now.signed_duration_since(self.info.blocked_since);
        let ago = if duration.num_minutes() < 1 {
            "just now".to_string()
        } else if duration.num_minutes() < 60 {
            format!("{} minutes ago", duration.num_minutes())
        } else if duration.num_hours() < 24 {
            format!("{} hours ago", duration.num_hours())
        } else {
            format!("{} days ago", duration.num_days())
        };

        let display = if self.info.display_name.is_empty() {
            &self.info.app_class
        } else {
            &self.info.display_name
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
                            .child(format!("{} \u{2014} Blocked {}", display, ago)),
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
}
