use gpui::prelude::*;
use gpui::*;
use gpui_component::list::ListState;

use crate::app::App as GuiApp;
use crate::components::{self as cmp, PolListDelegate, card};

use crate::policies::domain::PoliciesViewModel;

#[cfg(feature = "gui-gpui")]
impl GuiApp {
    pub fn render_policy_list(
        &self,
        cx: &mut Context<Self>,
        _vm: &PoliciesViewModel,
        pol_list: &Entity<ListState<PolListDelegate>>,
    ) -> AnyElement {
        card(
            &*cx,
            Some("Existing Policies"),
            vec![
                div()
                    .h(px(360.0))
                    .child(cmp::List::new(pol_list))
                    .into_any_element(),
            ],
        )
    }
}
