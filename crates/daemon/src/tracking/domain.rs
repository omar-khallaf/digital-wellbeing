//! Domain types for the tracking feature.

use chrono::{DateTime, Duration, Utc};
use wellbeing_core::AppId;

/// Reactive notifier that signals watchers when tracking state changes.
#[derive(Clone, Debug)]
pub struct ReactiveNotifier {
    tx: tokio::sync::watch::Sender<()>,
}

impl ReactiveNotifier {
    pub fn new() -> (Self, tokio::sync::watch::Receiver<()>) {
        let (tx, rx) = tokio::sync::watch::channel(());
        (Self { tx }, rx)
    }

    pub fn notify(&self) {
        let _ = self.tx.send(());
    }
}

impl Default for ReactiveNotifier {
    fn default() -> Self {
        let (tx, _) = tokio::sync::watch::channel(());
        Self { tx }
    }
}

/// In-memory focus interval state for a user's active window.
#[derive(Debug, Clone)]
pub struct FocusState {
    app_id: AppId,
    started_at: DateTime<Utc>,
    paused_at: Option<DateTime<Utc>>,
    paused_total: Duration,
}

impl FocusState {
    pub fn new(app_id: AppId, now: DateTime<Utc>) -> Self {
        Self {
            app_id,
            started_at: now,
            paused_at: None,
            paused_total: Duration::zero(),
        }
    }

    pub fn active_duration(&self, now: &DateTime<Utc>) -> i64 {
        let gross = *now - self.started_at;
        let idle = self.paused_total + self.paused_at.map(|p| *now - p).unwrap_or(Duration::zero());
        let active = (gross - idle).max(Duration::zero());
        active.num_minutes()
    }

    pub fn is_paused(&self) -> bool {
        self.paused_at.is_some()
    }

    pub fn pause(&mut self, now: DateTime<Utc>) {
        if self.paused_at.is_none() {
            self.paused_at = Some(now);
        }
    }

    pub fn resume(&mut self, now: DateTime<Utc>) {
        if let Some(p) = self.paused_at.take() {
            self.paused_total += now - p;
        }
    }

    pub fn app_id(&self) -> &AppId {
        &self.app_id
    }

    pub fn started_at(&self) -> &DateTime<Utc> {
        &self.started_at
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn app(s: &str) -> AppId {
        AppId::new(s).unwrap()
    }

    fn dt(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(secs, 0).unwrap()
    }

    #[test]
    fn test_focus_state_new() {
        let now = Utc::now();
        let fs = FocusState::new(app("firefox"), now);
        assert_eq!(fs.app_id().as_ref(), "firefox");
        assert_eq!(fs.started_at(), &now);
        assert!(!fs.is_paused());
        assert_eq!(fs.active_duration(&now), 0);
    }

    #[test]
    fn test_focus_state_active_duration_no_pause() {
        let start = dt(1_000_000);
        let fs = FocusState::new(app("code"), start);
        let now = start + Duration::seconds(300);
        assert_eq!(fs.active_duration(&now), 5);
        assert_eq!(fs.active_duration(&start), 0);
    }

    #[test]
    fn test_focus_state_pause_and_resume() {
        let start = dt(2_000_000);
        let mut fs = FocusState::new(app("terminal"), start);

        let idle_at = start + Duration::seconds(100);
        fs.pause(idle_at);
        assert!(fs.is_paused());
        assert_eq!(fs.active_duration(&idle_at), 1);

        let resume_at = idle_at + Duration::seconds(50);
        fs.resume(resume_at);
        assert!(!fs.is_paused());
        assert_eq!(fs.active_duration(&resume_at), 1);

        let later = resume_at + Duration::seconds(200);
        assert_eq!(fs.active_duration(&later), 5);
    }

    #[test]
    fn test_focus_state_multiple_idle_cycles() {
        let start = dt(3_000_000);
        let mut fs = FocusState::new(app("firefox"), start);

        fs.pause(start + Duration::seconds(60));
        fs.resume(start + Duration::seconds(90));

        fs.pause(start + Duration::seconds(210));
        fs.resume(start + Duration::seconds(230));

        let now = start + Duration::seconds(280);
        assert_eq!(fs.active_duration(&now), 3);
    }

    #[test]
    fn test_focus_state_idle_no_double_pause() {
        let start = dt(4_000_000);
        let mut fs = FocusState::new(app("music"), start);

        fs.pause(start + Duration::seconds(10));
        let paused_at = fs.paused_at;
        assert!(paused_at.is_some());

        fs.pause(start + Duration::seconds(20));
        assert_eq!(fs.paused_at, paused_at);
    }

    #[test]
    fn test_focus_state_resume_without_pause_is_noop() {
        let start = dt(5_000_000);
        let mut fs = FocusState::new(app("notes"), start);

        fs.resume(start + Duration::seconds(10));
        assert!(!fs.is_paused());
        assert_eq!(fs.paused_total, Duration::zero());
    }

    #[test]
    fn test_focus_state_active_duration_negative_clock() {
        let start = dt(5_000_000);
        let fs = FocusState::new(app("test"), start);
        let earlier = start - Duration::seconds(100);
        assert_eq!(fs.active_duration(&earlier), 0);
    }

    #[test]
    fn test_focus_state_display() {
        let fs = FocusState::new(app("org.gnome.gedit"), Utc::now());
        assert_eq!(fs.app_id().as_ref(), "org.gnome.gedit");
    }
}
