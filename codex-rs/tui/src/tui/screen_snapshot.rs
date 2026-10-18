//! Render the last owned-screen frame into the normal screen before handing control to the shell.

use std::io::Write;
use std::io::stdout;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;

use crossterm::cursor::MoveTo;
use crossterm::execute;
use crossterm::style::Attribute;
use crossterm::style::ResetColor;
use crossterm::style::SetAttribute;
use crossterm::terminal::Clear;
use crossterm::terminal::ClearType;
use ratatui::buffer::Buffer;
use unicode_width::UnicodeWidthStr;

#[derive(Clone, Default)]
pub(super) struct ScreenSnapshot(Arc<Mutex<Vec<String>>>);

impl ScreenSnapshot {
    pub(super) fn capture(&self, buffer: &Buffer) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = rows(buffer);
    }

    pub(super) fn print(&self) -> std::io::Result<()> {
        let rows = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        print(&rows)
    }
}

pub(super) fn rows(buffer: &Buffer) -> Vec<String> {
    (buffer.area.y..buffer.area.bottom())
        .map(|y| {
            let mut row = String::new();
            let mut last_non_space = 0;
            let mut skipped_columns = 0;
            for x in buffer.area.x..buffer.area.right() {
                let cell = &buffer[(x, y)];
                if skipped_columns > 0 {
                    skipped_columns -= 1;
                    continue;
                }
                if cell.symbol().is_empty() {
                    continue;
                }
                row.push_str(cell.symbol());
                skipped_columns = UnicodeWidthStr::width(cell.symbol()).saturating_sub(1);
                if cell.symbol() != " " {
                    last_non_space = row.len();
                }
            }
            row.truncate(last_non_space);
            row
        })
        .collect()
}

pub(super) fn print(rows: &[String]) -> std::io::Result<()> {
    let mut writer = stdout();
    print_to(&mut writer, rows)
}

fn print_to(writer: &mut impl Write, rows: &[String]) -> std::io::Result<()> {
    execute!(
        writer,
        SetAttribute(Attribute::Reset),
        ResetColor,
        Clear(ClearType::All),
        MoveTo(0, 0)
    )?;
    for (y, row) in rows.iter().enumerate() {
        execute!(writer, MoveTo(0, y as u16))?;
        writer.write_all(row.as_bytes())?;
    }
    if let Some(last_row) = rows.len().checked_sub(1) {
        execute!(writer, MoveTo(0, last_row as u16))?;
        writer.write_all(b"\r\n")?;
    }
    writer.flush()
}

#[cfg(test)]
#[path = "screen_snapshot_tests.rs"]
mod tests;
