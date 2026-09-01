//! Quick capture (⌘⇧N) and the clipboard scratchpad (⌘⇧V).

use crate::db;
use crate::index::Inventory;
use crate::model::{now_unix, Clip, Note};
use crate::search::{SearchQuery, SearchResponse};
use crate::Result;

const SETTING_LAST_EXPORT: &str = "scratchpad_last_export_at";
/// How many clips, or how long, before offering to export and clear.
const PROMPT_AFTER_CLIPS: i64 = 250;
const PROMPT_AFTER_DAYS: i64 = 14;

/// What a capture produced: the saved note, and what the user had already
/// worked out about it. "It surfaces the conversation from last week where you
/// solved it."
#[derive(Debug, Clone)]
pub struct CaptureResult {
    pub note: Note,
    pub related: SearchResponse,
}

/// A nudge to export and clear the scratchpad. The reviewed product is
/// explicit about being "honestly labelled about what it stores when on", so
/// this exists to keep an opt-in buffer from quietly growing forever.
#[derive(Debug, Clone)]
pub struct ScratchpadPrompt {
    pub clips: i64,
    pub oldest_at: Option<i64>,
    pub reason: &'static str,
}

impl Inventory {
    // --- quick capture -----------------------------------------------------

    /// Save a thought and immediately match it against everything indexed.
    pub fn capture(&self, text: &str) -> Result<CaptureResult> {
        let text = text.trim();
        if text.is_empty() {
            return Err(crate::Error::other("nothing to capture"));
        }
        let now = now_unix();
        self.connection().execute(
            "INSERT INTO notes(text, created_at) VALUES (?1, ?2)",
            rusqlite::params![text, now],
        )?;
        let note = Note {
            id: self.connection().last_insert_rowid(),
            text: text.to_string(),
            created_at: now,
        };

        let mut query = SearchQuery::new(text);
        query.limit = 5;
        let related = self.search(&query)?;

        Ok(CaptureResult { note, related })
    }

    pub fn notes(&self, limit: usize) -> Result<Vec<Note>> {
        let mut stmt = self
            .connection()
            .prepare("SELECT id, text, created_at FROM notes ORDER BY created_at DESC LIMIT ?1")?;
        let rows = stmt.query_map([limit as i64], |r| {
            Ok(Note {
                id: r.get(0)?,
                text: r.get(1)?,
                created_at: r.get(2)?,
            })
        })?;
        Ok(rows.flatten().collect())
    }

    // --- clipboard scratchpad ---------------------------------------------

    /// Record a clipboard entry, tagged with the app it came from.
    ///
    /// Returns `Ok(false)` without storing anything when the scratchpad is
    /// off, which is the default. Nothing is captured until the user asks for
    /// it to be.
    pub fn remember_clip(&self, text: &str, app: Option<&str>) -> Result<bool> {
        if !self.scratchpad_enabled()? {
            return Ok(false);
        }
        let text = text.trim();
        if text.is_empty() {
            return Ok(false);
        }
        // Copying the same thing twice in a row is one entry, not two.
        let last: Option<String> = self
            .connection()
            .query_row("SELECT text FROM clips ORDER BY id DESC LIMIT 1", [], |r| {
                r.get(0)
            })
            .ok();
        if last.as_deref() == Some(text) {
            return Ok(false);
        }

        self.connection().execute(
            "INSERT INTO clips(text, app, created_at) VALUES (?1,?2,?3)",
            rusqlite::params![text, app, now_unix()],
        )?;
        Ok(true)
    }

    pub fn clips(&self, limit: usize) -> Result<Vec<Clip>> {
        let mut stmt = self.connection().prepare(
            "SELECT id, text, app, created_at FROM clips ORDER BY created_at DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit as i64], |r| {
            Ok(Clip {
                id: r.get(0)?,
                text: r.get(1)?,
                app: r.get(2)?,
                created_at: r.get(3)?,
            })
        })?;
        Ok(rows.flatten().collect())
    }

    pub fn clear_clips(&self) -> Result<usize> {
        let n = self.connection().execute("DELETE FROM clips", [])?;
        db::set_setting(
            self.connection(),
            SETTING_LAST_EXPORT,
            &now_unix().to_string(),
        )?;
        Ok(n)
    }

    /// Everything in the scratchpad as plain text, newest last.
    pub fn export_clips(&self) -> Result<String> {
        let mut stmt = self
            .connection()
            .prepare("SELECT text, app, created_at FROM clips ORDER BY created_at ASC")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })?;

        let mut out = String::new();
        for (text, app, at) in rows.flatten() {
            out.push_str(&format!(
                "--- {} · {}\n{}\n\n",
                crate::format::timestamp(at),
                app.unwrap_or_else(|| "unknown app".into()),
                text
            ));
        }
        Ok(out)
    }

    /// Is it time to offer an export-and-clear?
    pub fn scratchpad_prompt(&self) -> Result<Option<ScratchpadPrompt>> {
        if !self.scratchpad_enabled()? {
            return Ok(None);
        }
        let (count, oldest): (i64, Option<i64>) = self.connection().query_row(
            "SELECT count(*), min(created_at) FROM clips",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if count == 0 {
            return Ok(None);
        }

        let last_export: Option<i64> =
            db::get_setting(self.connection(), SETTING_LAST_EXPORT)?.and_then(|v| v.parse().ok());
        let reference = last_export.or(oldest).unwrap_or_else(now_unix);
        let age_days = (now_unix() - reference) / 86_400;

        let reason = if count >= PROMPT_AFTER_CLIPS {
            "the scratchpad has grown large"
        } else if age_days >= PROMPT_AFTER_DAYS {
            "it has been a while since the scratchpad was cleared"
        } else {
            return Ok(None);
        };

        Ok(Some(ScratchpadPrompt {
            clips: count,
            oldest_at: oldest,
            reason,
        }))
    }
}
