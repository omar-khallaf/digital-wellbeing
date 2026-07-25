//! UTC-day-boundary interval splitting for closed focus intervals.
//!
//! When a focus interval spans a UTC day boundary, it must be split into
//! per-day segments so that daily usage totals are accurate. Each segment
//! is upserted into both `daily_usage` (per-app) and `daily_usage_by_title`
//! (per-title).

use chrono::{DateTime, Utc};
use diesel_async::AsyncConnection;
use wellbeing_core::{AppId, Uid, WindowTitle};

use super::BlockingRepo;

impl BlockingRepo {
    /// Split a focus interval across UTC-day boundaries and upsert each day
    /// segment into both `daily_usage` and `daily_usage_by_title`.
    pub(super) async fn upsert_interval_split_days<Conn>(
        conn: &mut Conn,
        uid: Uid,
        app_id: &AppId,
        focus_ts: &DateTime<Utc>,
        close_ts: &DateTime<Utc>,
        title: Option<&WindowTitle>,
    ) -> anyhow::Result<()>
    where
        Conn: AsyncConnection<Backend = diesel::sqlite::Sqlite>,
    {
        let mut seg_start = *focus_ts;

        loop {
            let next_boundary = (seg_start.date_naive() + chrono::TimeDelta::days(1))
                .and_hms_opt(0, 0, 0)
                .unwrap()
                .and_utc();

            if next_boundary >= *close_ts {
                let dur = Self::duration_millis(
                    seg_start.timestamp_millis(),
                    close_ts.timestamp_millis(),
                );
                let date = seg_start.format("%Y-%m-%d").to_string();
                Self::upsert_closed_delta(conn, &date, uid, app_id, dur).await?;
                if let Some(t) = title {
                    Self::upsert_closed_delta_by_title(conn, &date, uid, app_id, t, dur).await?;
                }
                break;
            }

            let dur = Self::duration_millis(
                seg_start.timestamp_millis(),
                next_boundary.timestamp_millis(),
            );
            let date = seg_start.format("%Y-%m-%d").to_string();
            Self::upsert_closed_delta(conn, &date, uid, app_id, dur).await?;
            if let Some(t) = title {
                Self::upsert_closed_delta_by_title(conn, &date, uid, app_id, t, dur).await?;
            }
            seg_start = next_boundary;
        }

        Ok(())
    }
}
