//! Event handlers for [`EnforcerActor`].
//!
//! Every [`PlatformEvent`] is buffered as-is.  The plugin is the sole source
//! of truth for focus state; the daemon does not track `current_focus`.
//! Dedup (coalescing consecutive identical Focus events) is the plugin's
//! responsibility.
//!
//! On every `Focus` event, `evaluate_and_enforce` is called immediately in
//! addition to the periodic minute-ticker. This prevents evasion: a blocked
//! app is detected within milliseconds of receiving focus, not up to 60 s
//! later. The evaluation is lightweight — it reads the current focus from
//! the in-memory registry and queries policies/usage from the database.

use tracing::{error, warn};

use crate::platform::PlatformEvent;

use super::EnforcerActor;

impl<P: crate::platform::Platform, C: wellbeing_core::Clock> EnforcerActor<P, C> {
    pub async fn handle_event(&mut self, event: PlatformEvent) {
        let should_evaluate = matches!(
            event,
            PlatformEvent::Focus { .. } | PlatformEvent::Block { .. }
        );

        self.event_buffer.push(event, self.clock.now());

        // Per-focus/per-block evaluation: detect blocks immediately on focus
        // switch and self-correct stale overlays when a Block event arrives
        // for an app whose policy has since been relaxed.
        // This runs BEFORE the buffer-threshold flush so enforcement is not
        // delayed by the batching window.
        if should_evaluate && let Err(e) = self.evaluate_and_enforce(self.clock.now()).await {
            warn!(error = %e, "Per-event evaluation failed");
        }

        // Buffer threshold flush
        if self.event_buffer.len() >= 100
            && let Err(e) = self.flush_buffer().await
        {
            error!(error = %e, "Count-triggered flush failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::platform::PlatformEvent;
    use wellbeing_core::{AppClass, Uid};

    #[tokio::test]
    async fn test_handle_event_buffers_event() {
        let (_tmp, _pool, mut actor, _rx) = crate::blocking::test_support::setup().await;
        let uid = Uid(1000);
        let app_class = AppClass::new("firefox").unwrap();

        let event = PlatformEvent::Focus {
            app_class: app_class.clone(),
            title: wellbeing_core::WindowTitle::new("test"),
            pid: wellbeing_core::Pid(0),
            uid,
        };
        actor.handle_event(event).await;

        assert_eq!(actor.event_buffer.len(), 1);
    }
}
