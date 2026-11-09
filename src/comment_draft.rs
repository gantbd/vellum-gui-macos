//! Bounded presentation-only form. Raw UTF-8 is the value; widget text is NEVER
//! serialized back as the body. Native annotation history begins on Apply.
use crate::{
    BridgeError, ViewToken,
    bridge::{Projection, projected_content},
};
use iced::widget::text_editor::{Action, Content, Cursor, Edit};
use unicode_segmentation::UnicodeSegmentation;
const HISTORY: usize = 16; // Reference form policy, not an engine/product limit.
struct Snapshot {
    raw: String,
    cursor: Cursor,
}

pub(crate) struct Draft {
    pub(crate) token: ViewToken,
    raw: String,
    projection: Projection,
    content: Content,
    limit: usize,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
}
impl Draft {
    pub(crate) fn new(raw: &str, limit: usize) -> Result<Self, BridgeError> {
        if raw.len() > limit {
            return Err(BridgeError::Capacity);
        }
        let mut text = String::new();
        text.try_reserve_exact(raw.len())
            .map_err(|_| BridgeError::Capacity)?;
        text.push_str(raw);
        let projection = Projection::new(raw);
        let content = projected_content(&projection.text);
        Ok(Self {
            token: ViewToken::fresh(),
            raw: text,
            projection,
            content,
            limit,
            undo: Vec::new(),
            redo: Vec::new(),
        })
    }
    pub(crate) fn content(&self) -> &Content {
        &self.content
    }
    pub(crate) fn raw(&self) -> &str {
        &self.raw
    }
    pub(crate) fn action(&mut self, action: Action) -> Result<(), BridgeError> {
        let Action::Edit(edit) = action else {
            self.content.perform(action);
            return Ok(());
        };
        let cursor = self.content.cursor();
        let head = self.projection.offset(&self.raw, cursor.position)?;
        let anchor = self
            .projection
            .offset(&self.raw, cursor.selection.unwrap_or(cursor.position))?;
        let mut start = head.min(anchor);
        let mut end = head.max(anchor);
        let inserted = match edit {
            Edit::Insert(ch) => ch.to_string(),
            Edit::Paste(text) => {
                if text.len() > self.limit {
                    return Err(BridgeError::Capacity);
                }
                text.as_str().to_owned()
            }
            Edit::Enter => "\n".into(),
            Edit::Indent => "\t".into(),
            Edit::Unindent => return Err(BridgeError::NativeLineIntent),
            Edit::Backspace => {
                if start == end {
                    start = self.raw[..start]
                        .grapheme_indices(true)
                        .next_back()
                        .map_or(0, |(i, _)| i);
                }
                String::new()
            }
            Edit::Delete => {
                if start == end {
                    end += self.raw[end..].graphemes(true).next().map_or(0, str::len);
                }
                String::new()
            }
        };
        let length = self
            .raw
            .len()
            .checked_sub(end - start)
            .and_then(|n| n.checked_add(inserted.len()))
            .ok_or(BridgeError::Capacity)?;
        if length > self.limit {
            return Err(BridgeError::Capacity);
        }
        if start == end && inserted.is_empty() {
            return Ok(());
        }
        let mut next = String::new();
        next.try_reserve_exact(length)
            .map_err(|_| BridgeError::Capacity)?;
        next.push_str(&self.raw[..start]);
        next.push_str(&inserted);
        next.push_str(&self.raw[end..]);
        let projection = Projection::new(&next);
        let mut content = projected_content(&projection.text);
        // Joining a canonical CRLF can hide its internal byte in the display.
        // Project the visual caret forward without changing any raw body bytes.
        let caret = start + inserted.len();
        let position = (caret..=next.len())
            .find_map(|byte| projection.position(&next, byte).ok())
            .ok_or(BridgeError::Position)?;
        content.move_to(Cursor {
            position,
            selection: None,
        });
        self.undo
            .try_reserve(1)
            .map_err(|_| BridgeError::Capacity)?;
        if self.undo.len() == HISTORY {
            self.undo.remove(0);
        }
        let raw = std::mem::replace(&mut self.raw, next);
        self.undo.push(Snapshot { raw, cursor });
        self.redo.clear();
        self.projection = projection;
        self.content = content;
        Ok(())
    }
    pub(crate) fn history(&mut self, redo: bool) -> Result<(), BridgeError> {
        let (from, to) = if redo {
            (&mut self.redo, &mut self.undo)
        } else {
            (&mut self.undo, &mut self.redo)
        };
        if from.is_empty() {
            return Ok(());
        }
        to.try_reserve(1).map_err(|_| BridgeError::Capacity)?;
        let old_cursor = self.content.cursor();
        let next = from.pop().expect("checked draft history");
        let projection = Projection::new(&next.raw);
        let mut content = projected_content(&projection.text);
        content.move_to(next.cursor);
        let raw = std::mem::replace(&mut self.raw, next.raw);
        if to.len() == HISTORY {
            to.remove(0);
        }
        to.push(Snapshot {
            raw,
            cursor: old_cursor,
        });
        self.projection = projection;
        self.content = content;
        self.token = ViewToken::fresh();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::widget::text_editor::Motion;
    use std::sync::Arc;
    #[test]
    fn raw_body_is_lossless_through_projection_edits_and_bounded_history() {
        for ending in ["\n", "\r", "\r\n", "\n\r", "\u{85}", "\u{2028}", "\u{2029}"] {
            let raw = format!("\u{feff}a{ending}e\u{301}\x1b$(inert)");
            let mut draft = Draft::new(&raw, 512).unwrap();
            draft.action(Action::Move(Motion::DocumentEnd)).unwrap();
            draft
                .action(Action::Edit(Edit::Paste(Arc::new("\r\nß".into()))))
                .unwrap();
            assert_eq!(draft.raw(), format!("{raw}\r\nß"));
            let old = draft.token.clone();
            draft.history(false).unwrap();
            assert_ne!(draft.token, old);
            assert_eq!(draft.raw(), raw);
            draft.history(true).unwrap();
            assert_eq!(draft.raw(), format!("{raw}\r\nß"));
            draft.action(Action::Edit(Edit::Backspace)).unwrap();
            assert_eq!(draft.raw(), format!("{raw}\r\n"));
        }
        let mut draft = Draft::new("e\u{301}", 3).unwrap();
        draft.action(Action::Move(Motion::DocumentEnd)).unwrap();
        assert!(draft.action(Action::Edit(Edit::Insert('x'))).is_err());
        assert_eq!(draft.raw(), "e\u{301}");
        assert!(draft.undo.is_empty());
        draft.action(Action::Edit(Edit::Backspace)).unwrap();
        assert_eq!(draft.raw(), "");
        for _ in 0..40 {
            draft.action(Action::Edit(Edit::Insert('a'))).unwrap();
            draft.action(Action::Edit(Edit::Backspace)).unwrap();
        }
        assert_eq!(draft.undo.len(), HISTORY);
        for _ in 0..40 {
            draft.history(false).unwrap();
        }
        assert_eq!(draft.redo.len(), HISTORY);
    }
}
