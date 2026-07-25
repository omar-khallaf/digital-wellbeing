//! Event handlers for [`EnforcerActor`].
//!
//! Every [`PlatformEvent`] is buffered as-is.  The plugin is the sole source
//! of truth for focus state; the daemon does not track `current_focus`.
//! Dedup (coalescing consecutive identical Focus events) is the plugin's
//! responsibility.

use tracing::error;

use crate::platform::PlatformEvent;

use super::EnforcerActor;

impl<P: crate::platform::Platform, C: wellbeing_core::Clock> EnforcerActor<P, C> {
    pub async fn handle_event(&mut self, event: PlatformEvent) {
        self.event_buffer.push(event, self.clock.now());

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
    use wellbeing_core::{AppId, Uid};

    #[tokio::test]
    async fn test_handle_event_buffers_event() {
        let (_tmp, _pool, mut actor, _rx) = crate::blocking::test_support::setup().await;
        let uid = Uid(1000);
        let app_id = AppId::new("firefox").unwrap();

        let event = PlatformEvent::Focus {
            app_id: app_id.clone(),
            title: wellbeing_core::WindowTitle::new("test"),
            pid: wellbeing_core::Pid(0),
            uid,
        };
        actor.handle_event(event).await;

        assert_eq!(actor.event_buffer.len(), 1);
    }
}
