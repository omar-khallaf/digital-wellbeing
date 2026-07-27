//! Event handlers for [`EnforcerActor`].
//!
//! Every [`PlatformEvent`] is buffered as-is.  The plugin is the sole source
//! of truth for focus state; the daemon does not track `current_focus`.
//! Dedup (coalescing consecutive identical Focus events) is the plugin's
//! responsibility.
//!
//! On every `Focus`/`Block` event, the event's own payload is used directly
//! for evaluation instead of re-querying the plugin's `CurrentFocus` D-Bus
//! property. This avoids a timing race where the compositor could switch
//! focus between signal receipt and property query. The periodic minute-ticker
//! (`evaluate_and_enforce`) still re-queries `CurrentFocus` as it has no
//! event payload to work from.

use tracing::{error, warn};

use crate::platform::PlatformEvent;

use super::EnforcerActor;

impl<P: crate::platform::Platform, C: wellbeing_core::Clock> EnforcerActor<P, C> {
    pub async fn handle_event(&mut self, event: PlatformEvent) {
        let should_evaluate = matches!(
            event,
            PlatformEvent::Focus { .. } | PlatformEvent::Block { .. }
        );

        // Extract uid + app_class BEFORE pushing event into the buffer
        // (which moves it). Per-focus/per-block evaluation uses the event's
        // own payload directly instead of re-querying CurrentFocus via D-Bus,
        // avoiding a timing race where compositor could switch focus between
        // signal receipt and property query. The periodic minute-ticker
        // (evaluate_and_enforce) still re-queries CurrentFocus as it has no
        // event payload.
        if should_evaluate {
            let uid = event.uid();
            if let Some(app_class) = event.app_class().cloned() {
                self.event_buffer.push(event, self.clock.now());
                if let Err(e) = self
                    .evaluate_and_apply(uid, &app_class, self.clock.now())
                    .await
                {
                    warn!(error = %e, "Per-event evaluation failed");
                }
            } else {
                self.event_buffer.push(event, self.clock.now());
            }
        } else {
            self.event_buffer.push(event, self.clock.now());
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
