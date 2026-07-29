//! Event repository — the `events` table.
//!
//! The events table is append-only. Uses raw SQL via turso.

use std::collections::HashMap;

use turso::{Value, params_from_iter};
use wellbeing_core::{AppClass, DayEventRow, EventType, Uid, WindowTitle};

use crate::blocking::domain::TimedEvent;
use crate::store::connection::DbConn;
use crate::store::schema_constants::events;

/// Event row type (plain struct, no diesel derives).
#[derive(Debug, Clone)]
pub struct EventRow {
    pub id: i32,
    pub event_type: i32,
    pub user_id: i32,
    pub timestamp: i64,
    pub app_class: Option<String>,
    pub title: Option<String>,
}

pub(crate) struct EventDao;

impl EventDao {
    /// Insert all events in a single multi-row INSERT statement.
    /// Replaces N individual INSERTs with one round-trip.
    pub(crate) async fn batch_insert_events(
        conn: &DbConn,
        events: &[TimedEvent],
    ) -> anyhow::Result<()> {
        if events.is_empty() {
            return Ok(());
        }

        let n = events.len();
        let cols = format!(
            "{}, {}, {}, {}, {}",
            events::EVENT_TYPE,
            events::USER_ID,
            events::TIMESTAMP,
            events::APP_CLASS,
            events::TITLE,
        );

        // Build VALUES (?, ?, ?, ?, ?), ..., (?, ?, ?, ?, ?) for N rows
        let row_placeholder = "(?, ?, ?, ?, ?)";
        let mut values = String::with_capacity(n * (row_placeholder.len() + 2));
        for i in 0..n {
            if i > 0 {
                values.push_str(", ");
            }
            values.push_str(row_placeholder);
        }

        let sql = format!("INSERT INTO {} ({}) VALUES {}", events::TABLE, cols, values,);

        let mut params: Vec<Value> = Vec::with_capacity(n * 5);
        for timed in events {
            let event = &timed.event;
            params.push(i32::from(event.event_type()).into());
            params.push((event.uid().0 as i32).into());
            params.push(timed.timestamp.timestamp_millis().into());
            // SQLite TEXT columns accept empty string for NULL app_class/title
            params.push(
                event
                    .app_class()
                    .map(|s| Value::Text(s.as_ref().to_string()))
                    .unwrap_or(Value::Null),
            );
            params.push(
                event
                    .title()
                    .map(|t| Value::Text(t.as_str().to_string()))
                    .unwrap_or(Value::Null),
            );
        }

        conn.execute(&sql, params_from_iter(params)).await?;
        Ok(())
    }

    pub(crate) async fn get_day_events(
        conn: &DbConn,
        uid: i32,
        start_millis: i64,
        end_millis: i64,
    ) -> anyhow::Result<Vec<DayEventRow>> {
        let sql = format!(
            "SELECT {}, {}, {}, {}, {}, {} FROM {} WHERE {} = ?1 AND {} >= ?2 AND {} < ?3 ORDER BY {} ASC",
            events::ID,
            events::EVENT_TYPE,
            events::USER_ID,
            events::TIMESTAMP,
            events::APP_CLASS,
            events::TITLE,
            events::TABLE,
            events::USER_ID,
            events::TIMESTAMP,
            events::TIMESTAMP,
            events::TIMESTAMP,
        );

        let mut result = conn.query(&sql, (uid, start_millis, end_millis)).await?;
        let mut rows = Vec::new();

        while let Some(row) = result.next().await? {
            rows.push(EventRow {
                id: row.get(0)?,
                event_type: row.get(1)?,
                user_id: row.get(2)?,
                timestamp: row.get(3)?,
                app_class: row.get(4)?,
                title: row.get(5)?,
            });
        }

        Ok(rows
            .into_iter()
            .map(|row| DayEventRow {
                id: row.id as u64,
                event_type: EventType::try_from(row.event_type).unwrap_or(EventType::Focus),
                timestamp: row.timestamp,
                app_class: row
                    .app_class
                    .filter(|s| !s.is_empty())
                    .and_then(|s| AppClass::new(&s).ok())
                    .unwrap_or_else(|| AppClass::new("unknown").unwrap()),
                title: WindowTitle::new(&row.title.unwrap_or_default()),
                user_id: row.user_id as u64,
            })
            .collect())
    }

    pub(crate) async fn get_last_events_for_uids(
        conn: &DbConn,
        uids: &[Uid],
    ) -> anyhow::Result<HashMap<Uid, Option<EventRow>>> {
        let mut results: HashMap<Uid, Option<EventRow>> =
            uids.iter().map(|&uid| (uid, None)).collect();

        if uids.is_empty() {
            return Ok(results);
        }

        let raw_uids: Vec<i32> = uids.iter().map(|uid| uid.0 as i32).collect();
        let uid_placeholders: Vec<String> = raw_uids.iter().map(|_| "?".to_string()).collect();
        let uid_list = uid_placeholders.join(", ");

        let ignored_types: Vec<i32> = EventType::IGNORED_BY_MEASUREMENT
            .iter()
            .map(|&et| et.into())
            .collect();
        let ignored_placeholders: Vec<String> =
            ignored_types.iter().map(|_| "?".to_string()).collect();
        let ignored_list = ignored_placeholders.join(", ");

        // Find the latest non-ignored event per user using MAX(timestamp).
        // The composite index idx_events_user_type_ts_id
        // (user_id, event_type, timestamp, id) allows SQLite to resolve
        // this as a group-wise index scan — one seek per user to the
        // last entry (by timestamp) that isn't Idle/Resume.
        let sql = format!(
            "SELECT e.{}, e.{}, e.{}, e.{}, e.{}, e.{} FROM {} e \
             WHERE (e.{} , e.{}) IN ( \
                 SELECT e2.{} , MAX(e2.{}) \
                 FROM {} e2 \
                 WHERE e2.{} IN ({}) \
                 AND e2.{} NOT IN ({}) \
                 GROUP BY e2.{} \
             )",
            events::ID,
            events::EVENT_TYPE,
            events::USER_ID,
            events::TIMESTAMP,
            events::APP_CLASS,
            events::TITLE,
            events::TABLE,
            events::USER_ID,
            events::TIMESTAMP,
            events::USER_ID,
            events::TIMESTAMP,
            events::TABLE,
            events::USER_ID,
            uid_list,
            events::EVENT_TYPE,
            ignored_list,
            events::USER_ID,
        );

        let mut params: Vec<i32> = raw_uids;
        params.extend(ignored_types);

        match conn.query(&sql, params_from_iter(params)).await {
            Ok(mut result) => {
                while let Some(row) = result.next().await? {
                    let user_id: i32 = row.get(2)?;
                    let uid = Uid(user_id as u32);
                    results.insert(
                        uid,
                        Some(EventRow {
                            id: row.get(0)?,
                            event_type: row.get(1)?,
                            user_id,
                            timestamp: row.get(3)?,
                            app_class: row.get(4)?,
                            title: row.get(5)?,
                        }),
                    );
                }
            }
            Err(e) => {
                return Err(anyhow::anyhow!("query failed: {}", e));
            }
        }

        Ok(results)
    }

    pub(crate) async fn get_event_range(
        conn: &DbConn,
        start: i64,
        end: i64,
    ) -> anyhow::Result<Vec<EventRow>> {
        let sql = format!(
            "SELECT {}, {}, {}, {}, {}, {} FROM {} WHERE {} >= ?1 AND {} <= ?2 ORDER BY {} ASC",
            events::ID,
            events::EVENT_TYPE,
            events::USER_ID,
            events::TIMESTAMP,
            events::APP_CLASS,
            events::TITLE,
            events::TABLE,
            events::TIMESTAMP,
            events::TIMESTAMP,
            events::TIMESTAMP,
        );

        let mut result = conn.query(&sql, (start, end)).await?;
        let mut rows = Vec::new();

        while let Some(row) = result.next().await? {
            rows.push(EventRow {
                id: row.get(0)?,
                event_type: row.get(1)?,
                user_id: row.get(2)?,
                timestamp: row.get(3)?,
                app_class: row.get(4)?,
                title: row.get(5)?,
            });
        }

        Ok(rows)
    }
}
