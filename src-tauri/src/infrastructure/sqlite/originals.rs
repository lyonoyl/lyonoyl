//! The message a draft or a queued message passes on in its own form, kept beside its row.
//!
//! Two pairs of tables of one shape: the original, and the pictures it draws, in order. Each pair
//! goes with the row it belongs to, by the cascades the schema declares, so a draft thrown away or
//! a message let go of takes what it passed on with it.

use rusqlite::{Connection, OptionalExtension};

use lyonoyl_domain::outgoing::{Original, OriginalPicture};

/// Which row a message passed on is kept beside.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Beside {
    Draft,
    Outbox,
}

impl Beside {
    fn originals(self) -> &'static str {
        match self {
            Self::Draft => "draft_originals",
            Self::Outbox => "outbox_originals",
        }
    }

    fn pictures(self) -> &'static str {
        match self {
            Self::Draft => "draft_original_pictures",
            Self::Outbox => "outbox_original_pictures",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::Draft => "draft_id",
            Self::Outbox => "outbox_id",
        }
    }
}

/// Writes down the message the row `id` passes on: its markup, its text and its pictures, in the
/// order it carries them.
pub(super) fn written_beside(
    writing: &Connection,
    beside: Beside,
    id: i64,
    original: &Original,
) -> rusqlite::Result<()> {
    writing.execute(
        &format!(
            "INSERT INTO {} ({}, opening, drawing, text) VALUES (?1, ?2, ?3, ?4)",
            beside.originals(),
            beside.key()
        ),
        rusqlite::params![id, original.opening, original.drawing, original.text],
    )?;

    for (position, picture) in original.pictures.iter().enumerate() {
        writing.execute(
            &format!(
                "INSERT INTO {} ({}, named, content_type, bytes, position)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                beside.pictures(),
                beside.key()
            ),
            rusqlite::params![
                id,
                picture.named,
                picture.content_type,
                picture.bytes,
                position as i64
            ],
        )?;
    }

    Ok(())
}

/// The message the row `id` passes on, and nothing for a row that passes none on.
pub(super) fn read_beside(
    reading: &Connection,
    beside: Beside,
    id: i64,
) -> rusqlite::Result<Option<Original>> {
    let Some((opening, drawing, text)) = reading
        .query_row(
            &format!(
                "SELECT opening, drawing, text FROM {} WHERE {} = ?1",
                beside.originals(),
                beside.key()
            ),
            [id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()?
    else {
        return Ok(None);
    };

    let pictures = reading
        .prepare(&format!(
            "SELECT named, content_type, bytes FROM {} WHERE {} = ?1 ORDER BY position, id",
            beside.pictures(),
            beside.key()
        ))?
        .query_map([id], |row| {
            Ok(OriginalPicture {
                named: row.get(0)?,
                content_type: row.get(1)?,
                bytes: row.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    Ok(Some(Original {
        opening,
        drawing,
        text,
        pictures,
    }))
}

#[cfg(test)]
#[path = "originals_tests.rs"]
mod tests;
