//! Event repository — the `events` table.
//!
//! The events table is append-only. Uses raw SQL via turso.

use std::collections::HashMap;

use turso::params_from_iter;
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
    pub(crate) async fn flush_events(conn: &DbConn, events: &[TimedEvent]) -> anyhow::Result<()> {
        if events.is_empty() {
            return Ok(());
        }

        // Insert one row at a time. Each call is async-native (no spawn_blocking).
        for timed in events {
            let event = &timed.event;
            let sql = format!(
                "INSERT INTO {} ({}, {}, {}, {}, {}) VALUES (?1, ?2, ?3, ?4, ?5)",
                events::TABLE,
                events::EVENT_TYPE,
                events::USER_ID,
                events::TIMESTAMP,
                events::APP_CLASS,
                events::TITLE,
            );
            conn.execute(
                &sql,
                (
                    i32::from(event.event_type()),
                    event.uid().0 as i32,
                    timed.timestamp.timestamp_millis(),
                    event.app_class().map(|s| s.as_ref()),
                    event.title().map(|t| t.as_str()),
                ),
            )
            .await?;
        }

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

        // Build a query that gets the latest event per user, excluding ignored types.
        // Use a subquery to find the max timestamp per user, then join back.
        let placeholders: Vec<String> = raw_uids.iter().map(|_| "?".to_string()).collect();
        let uid_list = placeholders.join(", ");

        let ignored_types: Vec<i32> = EventType::IGNORED_BY_MEASUREMENT
            .iter()
            .map(|&et| et.into())
            .collect();
        let ignored_placeholders: Vec<String> =
            ignored_types.iter().map(|_| "?".to_string()).collect();
        let ignored_list = ignored_placeholders.join(", ");

        let sql = format!(
            "SELECT e1.{}, e1.{}, e1.{}, e1.{}, e1.{}, e1.{} FROM {} e1 \
             WHERE e1.{} IN ({}) \
             AND e1.{} NOT IN ({}) \
             AND NOT EXISTS ( \
                 SELECT 1 FROM {} e2 \
                 WHERE e2.{} = e1.{} \
                 AND e2.{} NOT IN ({}) \
                 AND (e2.{} > e1.{} OR (e2.{} = e1.{} AND e2.{} > e1.{})) \
             )",
            events::ID,
            events::EVENT_TYPE,
            events::USER_ID,
            events::TIMESTAMP,
            events::APP_CLASS,
            events::TITLE,
            events::TABLE,
            events::USER_ID,
            uid_list,
            events::EVENT_TYPE,
            ignored_list,
            events::TABLE,
            events::USER_ID,
            events::USER_ID,
            events::EVENT_TYPE,
            ignored_list,
            events::TIMESTAMP,
            events::TIMESTAMP,
            events::TIMESTAMP,
            events::TIMESTAMP,
            events::ID,
            events::ID,
        );

        let mut params: Vec<i32> = raw_uids.clone();
        params.extend(ignored_types.clone());

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
