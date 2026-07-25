//! Pre-buffer focus resolution and close handling.
//!
//! When a close event arrives in the buffer but the corresponding open focus
//! happened before the buffer window (i.e. the open event is in a previous
//! flush cycle), we need to look up the last `WindowFocused` event from the
//! database to resolve the interval start, then close it normally.

use chrono::DateTime;
use diesel::{ExpressionMethods, QueryDsl};
use diesel_async::AsyncConnection;
use diesel_async::RunQueryDsl;
use wellbeing_core::{AppId, Uid, WindowTitle, event_types::EVENT_WINDOW_FOCUSED};

use crate::store::schema;

use super::BlockingRepo;
use crate::blocking::buffer::BufferedEvent;

impl BlockingRepo {
    /// Query the last WindowFocused event before the buffer for a uid.
    /// Returns `(timestamp_millis, app_id, title)`.
    pub(super) async fn resolve_pre_buffer_focus<Conn>(
        conn: &mut Conn,
        uid: Uid,
        today_start_millis: i64,
        now_millis: i64,
    ) -> Option<(i64, Option<String>, Option<String>)>
    where
        Conn: AsyncConnection<Backend = diesel::sqlite::Sqlite>,
    {
        schema::events::table
            .filter(schema::events::user_id.eq(uid.0 as i32))
            .filter(schema::events::event_type.eq(EVENT_WINDOW_FOCUSED))
            .filter(schema::events::timestamp.ge(today_start_millis))
            .filter(schema::events::timestamp.le(now_millis))
            .order(schema::events::timestamp.desc())
            .limit(1)
            .select((
                schema::events::timestamp,
                schema::events::app_id,
                schema::events::title,
            ))
            .first::<(i64, Option<String>, Option<String>)>(conn)
            .await
            .ok()
    }

    /// Handle a close event whose interval started before the buffer.
    pub(super) async fn apply_pre_buffer_close<Conn>(
        conn: &mut Conn,
        uid: Uid,
        event: &BufferedEvent,
        today_start_millis: i64,
        now_millis: i64,
    ) -> anyhow::Result<()>
    where
        Conn: AsyncConnection<Backend = diesel::sqlite::Sqlite>,
    {
        let Some((start_ts, db_app_id, db_title)) =
            Self::resolve_pre_buffer_focus(conn, uid, today_start_millis, now_millis).await
        else {
            return Ok(());
        };
        let Some(ref resolved) = db_app_id else {
            return Ok(());
        };
        let valid = AppId::new(resolved).unwrap_or_else(|_| AppId::new("unknown").unwrap());
        let title_str = db_title.as_deref().unwrap_or("(untitled)");
        let title = WindowTitle::new(title_str);
        let title_ref: Option<&WindowTitle> = Some(&title);

        let start_dt = DateTime::from_timestamp_millis(start_ts)
            .ok_or_else(|| anyhow::anyhow!("resolved focus timestamp must be valid: {start_ts}"))?;
        Self::upsert_interval_split_days(conn, uid, &valid, &start_dt, &event.timestamp, title_ref)
            .await?;

        Ok(())
    }
}
