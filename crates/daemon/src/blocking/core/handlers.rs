//! Event handlers for [`EnforcerActor`].
//!
//! Each handler translates a [`PlatformEvent`] into buffered events and
//! state transitions.

use crate::signal::DaemonSignal;
use tracing::error;
use wellbeing_core::*;

use crate::platform::PlatformEvent;

use super::super::buffer::BufferedEvent;
use super::EnforcerActor;
use wellbeing_core::event_types::{EVENT_IDLE, EVENT_RESUMED};

impl<P: crate::platform::Platform, C: wellbeing_core::Clock> EnforcerActor<P, C> {
    pub async fn handle_event(&mut self, event: PlatformEvent) {
        match event {
            PlatformEvent::WindowFocused {
                app_id, uid, title, ..
            } => {
                self.handle_window_focused(app_id, uid, title).await;
            }
            PlatformEvent::WindowBlocked { app_id, uid, title } => {
                self.handle_window_blocked(app_id, uid, title).await
            }
            PlatformEvent::Unfocused => self.handle_unfocused().await,
            PlatformEvent::IdleActivity => self.handle_idle_activity().await,
            PlatformEvent::ResumedActivity => self.handle_resumed_activity().await,
            PlatformEvent::ResumedSystem => {
                // System resumed from sleep — main.rs checks the lock state
                // and calls handle_resync_after_termination_event directly.
            }
            PlatformEvent::Slept | PlatformEvent::Locked | PlatformEvent::LoggedOut => {
                self.handle_session_event(event).await;
            }
            PlatformEvent::ShutDown => self.handle_shut_down().await,
        }

        // Buffer threshold flush
        if self.event_buffer.len() >= 100
            && let Err(e) = self.flush_buffer().await
        {
            error!(error = %e, "Count-triggered flush failed");
        }
    }

    /// Handle a window focus switch: dedup, buffer the new focus.
    ///
    /// The pairing logic in `deltas.rs` implicitly closes the previous
    /// focus interval for this `uid` when a new `WindowFocused` arrives,
    /// so no synthetic `Unfocused` event is pushed here.
    ///
    /// Dedup is based on BOTH `app_id` AND `title` — identical consecutive
    /// titles within the same app are coalesced, but a title switch within
    /// the same app creates a new interval.
    pub(crate) async fn handle_window_focused(
        &mut self,
        app_id: AppId,
        uid: Uid,
        title: WindowTitle,
    ) {
        if matches!(self.current_focus.get(&uid), Some(prev) if *prev == app_id)
            && self.last_titles.get(&uid) == Some(&title)
        {
            return;
        }

        // Eagerly remove active block for the previously focused app, if any.
        // Guard: skip if same app_id (title-only switch) — dedup already
        // handled the identical case, but a different title within the same
        // app must not clear the block.
        if let Some(prev_app) = self.current_focus.get(&uid)
            && prev_app != &app_id
            && let Some(user_blocks) = self.blocked_apps.write().await.get_mut(&uid)
                && user_blocks.remove(prev_app).is_some() {
                    let _ = self.signal_tx.send(DaemonSignal::BlockedAppsChanged {
                        uid: uid.0,
                        app_id: prev_app.clone(),
                        blocked: false,
                        reason: 0,
                    });
                }

        self.current_focus.insert(uid, app_id.clone());
        self.last_titles.insert(uid, title.clone());
        self.event_buffer.push(BufferedEvent {
            uid,
            app_id,
            event_type: 0, // EVENT_WINDOW_FOCUSED
            timestamp: self.clock.now(),
            title: Some(title),
        });
    }

    /// Push a [`BufferedEvent`] for each entry in `current_focus` with the given
    /// `event_type`. If `drain` is true, the focus map is cleared after buffering.
    fn buffer_event_for_all_focused(&mut self, event_type: i32, drain: bool) {
        if drain {
            for (uid, app_id) in self.current_focus.drain() {
                self.event_buffer.push(BufferedEvent {
                    uid,
                    app_id,
                    event_type,
                    timestamp: self.clock.now(),
                    title: None,
                });
            }
        } else {
            for (uid, app_id) in &self.current_focus {
                self.event_buffer.push(BufferedEvent {
                    uid: *uid,
                    app_id: app_id.clone(),
                    event_type,
                    timestamp: self.clock.now(),
                    title: None,
                });
            }
        }
    }

    /// Handle a window-blocked event from the plugin (event_type = 8).
    ///
    /// The plugin sends `FocusChanged` with `FOCUS_TAG_BLOCKED` when the
    /// compositor blocks a window. This event is buffered like any other
    /// close event — the pairing logic in `deltas.rs` will close the
    /// previously open focus interval for this uid.
    pub(crate) async fn handle_window_blocked(
        &mut self,
        app_id: AppId,
        uid: Uid,
        title: WindowTitle,
    ) {
        self.current_focus.remove(&uid);
        self.last_titles.insert(uid, title.clone());
        self.event_buffer.push(BufferedEvent {
            uid,
            app_id,
            event_type: 8, // EVENT_WINDOW_BLOCKED
            timestamp: self.clock.now(),
            title: Some(title),
        });
    }

    /// Handle global unfocused: drain all current focus entries.
    async fn handle_unfocused(&mut self) {
        self.buffer_event_for_all_focused(1, true);
    }

    /// Handle idle transition: buffer idle for all focused apps.
    async fn handle_idle_activity(&mut self) {
        self.buffer_event_for_all_focused(EVENT_IDLE, false);
    }

    /// Handle resume from idle: buffer resumed for all focused apps.
    async fn handle_resumed_activity(&mut self) {
        self.buffer_event_for_all_focused(EVENT_RESUMED, false);
    }

    /// Handle session events (sleep, lock, logout): drain focus with
    /// the appropriate event type.
    async fn handle_session_event(&mut self, event: PlatformEvent) {
        let event_type = match event {
            PlatformEvent::Slept => 4,
            PlatformEvent::Locked => 6,
            PlatformEvent::LoggedOut => 7,
            _ => unreachable!(),
        };
        self.buffer_event_for_all_focused(event_type, true);
    }

    /// Handle shut down: drain focus with shut-down event type.
    async fn handle_shut_down(&mut self) {
        self.buffer_event_for_all_focused(5, true);
    }
}

#[cfg(test)]
mod tests {
    use wellbeing_core::{AppId, Uid, WindowTitle};

    

    /// Verify handle_window_blocked removes uid from current_focus and pushes a
    /// buffered event.
    #[tokio::test]
    async fn test_handle_window_blocked_removes_from_current_focus() {
        let (_tmp, _pool, mut actor, _rx) = crate::blocking::test_support::setup().await;
        let uid = Uid(1000);
        let app_id = AppId::new("firefox").unwrap();
        let title = WindowTitle::new("Mozilla Firefox");

        // Given: uid is in current_focus
        actor.current_focus.insert(uid, app_id.clone());

        // When: handle_window_blocked is called
        actor
            .handle_window_blocked(app_id.clone(), uid, title.clone())
            .await;

        // Then: uid is removed from current_focus
        assert!(
            !actor.current_focus.contains_key(&uid),
            "uid should be removed from current_focus"
        );

        // And: a blocked event is in the buffer
        assert_eq!(actor.event_buffer.len(), 1);
        let buffered = actor.event_buffer.drain();
        assert_eq!(buffered[0].event_type, 8); // EVENT_WINDOW_BLOCKED
        assert_eq!(buffered[0].uid, uid);
        assert_eq!(buffered[0].app_id, app_id);
        assert_eq!(buffered[0].title, Some(title));
    }

    /// Verify handle_window_blocked still works when uid is NOT in current_focus
    /// (e.g., from a stale plugin message).
    #[tokio::test]
    async fn test_handle_window_blocked_no_current_focus() {
        let (_tmp, _pool, mut actor, _rx) = crate::blocking::test_support::setup().await;
        let uid = Uid(1000);
        let app_id = AppId::new("firefox").unwrap();
        let title = WindowTitle::new("Mozilla Firefox");

        // When: handle_window_blocked called with NO current_focus entry
        actor
            .handle_window_blocked(app_id.clone(), uid, title.clone())
            .await;

        // Then: no panic, event is still buffered
        assert_eq!(actor.event_buffer.len(), 1);
        let buffered = actor.event_buffer.drain();
        assert_eq!(buffered[0].event_type, 8);
    }

    /// Verify that after WindowBlocked, a new WindowFocused can be set.
    #[tokio::test]
    async fn test_handle_window_blocked_then_new_focus() {
        let (_tmp, _pool, mut actor, _rx) = crate::blocking::test_support::setup().await;
        let uid = Uid(1000);
        let app_a = AppId::new("firefox").unwrap();
        let app_b = AppId::new("code").unwrap();
        let title = WindowTitle::new("window");

        // Given: focus on firefox
        actor.current_focus.insert(uid, app_a.clone());

        // When: block firefox, then focus code
        actor.handle_window_blocked(app_a, uid, title.clone()).await;
        actor.handle_window_focused(app_b, uid, title).await;

        // Then: current_focus points to code
        assert_eq!(
            actor.current_focus.get(&uid).map(|a| a.as_ref()),
            Some("code")
        );
    }

    /// Verify that refocusing the same app with the SAME title is a no-op.
    #[tokio::test]
    async fn test_handle_window_focused_same_app_same_title_is_noop() {
        let (_tmp, _pool, mut actor, _rx) = crate::blocking::test_support::setup().await;
        let uid = Uid(1000);
        let app_id = AppId::new("firefox").unwrap();
        let title = WindowTitle::new("Mozilla Firefox");

        actor.current_focus.insert(uid, app_id.clone());
        actor.last_titles.insert(uid, title.clone());

        actor
            .handle_window_focused(app_id.clone(), uid, title.clone())
            .await;

        assert_eq!(actor.event_buffer.len(), 0, "no event should be buffered");
        assert_eq!(
            actor.current_focus.get(&uid).map(|a| a.as_ref()),
            Some("firefox")
        );
    }

    /// Verify that refocusing the same app with a DIFFERENT title buffers
    /// a new WindowFocused event (creates a new interval).
    #[tokio::test]
    async fn test_handle_window_focused_same_app_different_title_buffers_event() {
        let (_tmp, _pool, mut actor, _rx) = crate::blocking::test_support::setup().await;
        let uid = Uid(1000);
        let app_id = AppId::new("firefox").unwrap();
        let title1 = WindowTitle::new("Tab 1");
        let title2 = WindowTitle::new("Tab 2");

        actor.current_focus.insert(uid, app_id.clone());
        actor.last_titles.insert(uid, title1.clone());

        actor
            .handle_window_focused(app_id.clone(), uid, title2.clone())
            .await;

        assert_eq!(actor.event_buffer.len(), 1, "new event should be buffered");
        let buffered = actor.event_buffer.drain();
        assert_eq!(buffered[0].event_type, 0); // EVENT_WINDOW_FOCUSED
        assert_eq!(buffered[0].title, Some(title2));
    }
}
