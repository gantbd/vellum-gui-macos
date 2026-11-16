use iced::widget::text_editor::{self, Content, Cursor, Edit, Motion, Position};
use std::{
    hash::{Hash, Hasher},
    ops::Range,
    path::PathBuf,
    sync::Arc,
};
use unicode_segmentation::UnicodeSegmentation;
use vellum_editor::{
    Action, DestinationPlan, Editor, EditorError, EditorSnapshot, ReferenceSessionConfig,
};
use vellum_engine::{coordinator::DocumentVersion, source::Replacement};
mod clipboard;
mod comments;
mod geometry;
pub use geometry::{GeometryRequest, GeometryUsage};

/// Attachment identity, not authority or a frozen engine command. Ordered input
/// may continue after this view's own edits; a tab/attachment change spends it.
/// Native proposals still bind the exact current original snapshot separately.
#[derive(Clone)]
pub struct ViewToken(Arc<ViewIdentity>);
struct ViewIdentity {
    widget_id: iced::advanced::widget::Id,
}
impl ViewToken {
    pub(crate) fn fresh() -> Self {
        Self(Arc::new(ViewIdentity {
            widget_id: iced::advanced::widget::Id::unique(),
        }))
    }
    pub(crate) fn widget_id(&self) -> iced::advanced::widget::Id {
        self.0.widget_id.clone()
    }
}
#[cfg(test)]
impl ViewToken {
    pub(crate) fn test_token() -> Self {
        Self::fresh()
    }
}
impl PartialEq for ViewToken {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for ViewToken {}
impl Hash for ViewToken {
    fn hash<H: Hasher>(&self, state: &mut H) {
        Arc::as_ptr(&self.0).hash(state);
    }
}
impl std::fmt::Debug for ViewToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ViewToken(<opaque>)")
    }
}
#[derive(Clone, Copy, Debug)]
pub enum Command {
    Undo,
    Redo,
    Save,
    NextDocument,
    ToggleEditingLock,
    NextMatch,
    PreviousMatch,
}
#[derive(Debug)]
pub enum BridgeError {
    Editor(EditorError),
    Clipboard(vellum_clipboard_macos::IntegrationError),
    Stale,
    ProjectionMismatch,
    Position,
    Capacity,
    NativeLineIntent,
    UnsupportedSelectionProfile,
    GeometryUnavailable,
}
impl From<EditorError> for BridgeError {
    fn from(e: EditorError) -> Self {
        Self::Editor(e)
    }
}
impl From<vellum_clipboard_macos::IntegrationError> for BridgeError {
    fn from(error: vellum_clipboard_macos::IntegrationError) -> Self {
        match error {
            vellum_clipboard_macos::IntegrationError::Editor(error) => Self::Editor(error),
            other => Self::Clipboard(other),
        }
    }
}
impl std::fmt::Display for BridgeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "iced bridge: {self:?}")
    }
}
impl std::error::Error for BridgeError {}

/// Read-only literal/range proposal inspection, never admission or authority.
/// Line intents use `line_edit`, not a duplicate UI lowering or fake empty plan.
pub struct Proposal {
    pub version: DocumentVersion,
    pub edits: Vec<Replacement>,
    pub caret: usize,
    pub selection: Option<usize>,
}
pub struct Bridge {
    editor: Editor,
    native_clipboard: Option<vellum_clipboard_macos::ReferenceClipboard>,
    original: EditorSnapshot,
    content: Content,
    projection: Projection,
    token: ViewToken,
    dirty: bool,
    status: String,
    max_source: usize,
    max_edits: usize,
    max_payload: usize,
    full_refreshes: Option<u64>,
    geometry_usage: GeometryUsage,
    secondary_targets: Option<geometry::SecondaryTargets>,
    has_secondary_selections: bool,
    secondary_usage: std::cell::Cell<GeometryUsage>,
    #[cfg(feature = "desktop-qualification")]
    pub(crate) source_viewport: std::cell::Cell<Option<iced::Rectangle>>,
}
impl Bridge {
    pub fn open_reference(paths: &[PathBuf]) -> Result<Self, BridgeError> {
        let mut bridge = Self::open(paths, crate::reference_session_config())?;
        bridge.enable_native_unicode(
            &bridge.token(),
            vellum_editor::NativeUnicodeProfile::reference(),
        )?;
        bridge.native_clipboard = Some(vellum_clipboard_macos::ReferenceClipboard::new(
            &mut bridge.editor,
        )?);
        Ok(bridge)
    }
    /// Explicit host opt-in; never infer a provider from UI gestures or text.
    pub fn enable_native_unicode(
        &mut self,
        token: &ViewToken,
        profile: vellum_editor::NativeUnicodeProfile,
    ) -> Result<(), BridgeError> {
        self.enable_native_unicode_with(
            token,
            vellum_adapter_unicode_icu::IcuFactory::reference(),
            profile,
        )
    }
    pub fn enable_native_unicode_with(
        &mut self,
        token: &ViewToken,
        factory: vellum_adapter_unicode_icu::IcuFactory,
        profile: vellum_editor::NativeUnicodeProfile,
    ) -> Result<(), BridgeError> {
        self.check(token)?;
        self.editor.enable_native_unicode(factory, profile)?;
        self.status =
            "Ready — native Unicode 18 source commands; shaping/IME qualification separate".into();
        Ok(())
    }
    pub fn native_unicode_enabled(&self) -> bool {
        self.editor.native_unicode_enabled()
    }
    pub fn native_boundary_usage(&self) -> vellum_engine::coordinator::BoundaryUsage {
        self.editor.native_boundary_usage()
    }
    /// Includes candidate preview and exact-version evidence generation; not a
    /// count of user commands, edits or versions. No document identity/content.
    pub fn native_unicode_usage(&self) -> Option<vellum_editor::NativeUnicodeUsage> {
        self.editor.unicode_provider_usage()
    }
    pub fn open(paths: &[PathBuf], config: ReferenceSessionConfig) -> Result<Self, BridgeError> {
        let limits = config.engine.limits.source;
        let mut editor = Editor::open(paths, config)?;
        if editor.comments_enabled() {
            editor.enable_comment_files(vellum_editor::CommentFileLimits::reference())?;
        }
        let original = editor.snapshot()?;
        let projection = Projection::new(&original.text);
        let content = projected_content(&projection.text);
        if content.text() != projection.text {
            return Err(BridgeError::ProjectionMismatch);
        }
        let dirty = editor.is_dirty()?;
        Ok(Self {
            editor,
            native_clipboard: None,
            original,
            content,
            projection,
            token: ViewToken::fresh(),
            dirty,
            status: "Ready — engine owns source/history; generated newline LF".into(),
            max_source: limits.max_source_bytes,
            max_edits: limits.max_edits,
            max_payload: limits.max_edit_payload_bytes,
            full_refreshes: Some(1),
            geometry_usage: GeometryUsage::default(),
            secondary_targets: None,
            has_secondary_selections: false,
            secondary_usage: std::cell::Cell::new(GeometryUsage::default()),
            #[cfg(feature = "desktop-qualification")]
            source_viewport: std::cell::Cell::new(None),
        })
    }
    pub fn content(&self) -> &Content {
        &self.content
    }
    pub fn token(&self) -> ViewToken {
        self.token.clone()
    }
    pub fn dirty(&self) -> bool {
        self.dirty
    }
    pub fn editing_locked(&self) -> bool {
        self.editor.editing_locked()
    }
    pub fn status(&self) -> &str {
        &self.status
    }
    pub(crate) fn multiple_selections_active(&self) -> bool {
        self.has_secondary_selections
    }
    pub fn is_closed(&self) -> bool {
        self.editor.is_closed()
    }
    pub fn active_document(&self) -> usize {
        self.editor.active_document()
    }
    pub fn document_count(&self) -> usize {
        self.editor.document_count()
    }
    /// Original native selection disclosure for the current trusted attachment.
    pub fn native_selection(
        &mut self,
        token: &ViewToken,
    ) -> Result<Option<vellum_engine::coordinator::LogicalViewSnapshot>, BridgeError> {
        self.check(token)?;
        Ok(self.editor.native_selection()?)
    }
    pub fn native_view_usage(&self) -> vellum_engine::coordinator::LogicalViewUsage {
        self.editor.native_view_usage()
    }
    pub fn dropped_diagnostics(&self) -> u64 {
        self.editor.dropped_diagnostics()
    }
    pub fn pop_diagnostic(&mut self) -> Option<vellum_engine::instrumentation::Observation> {
        self.editor.pop_diagnostic()
    }
    pub fn diagnostic_window_overwrites(&self) -> Option<u64> {
        self.editor.diagnostic_window_overwrites()
    }
    /// UI-local bounded observation. None means counter overflow, not zero.
    pub fn full_projection_refreshes(&self) -> Option<u64> {
        self.full_refreshes
    }
    fn record_refresh(&mut self) {
        self.full_refreshes = self.full_refreshes.and_then(|n| n.checked_add(1));
    }
    fn check(&mut self, token: &ViewToken) -> Result<(), BridgeError> {
        if *token != self.token || self.editor.is_closed() {
            return Err(BridgeError::Stale);
        }
        // Native versions bind immutable text; avoid cloning the entire source
        // just to check identity. Widget/projection coherence is still checked.
        if self.editor.version()? != self.original.version {
            return Err(BridgeError::Stale);
        }
        let cursor = self.content.cursor();
        let anchor = cursor.selection.unwrap_or(cursor.position);
        if !content_matches_lines(
            &self.content,
            &self.original.text,
            &self.projection,
            cursor.position.line.min(anchor.line),
            cursor.position.line.max(anchor.line),
        ) {
            return Err(BridgeError::ProjectionMismatch);
        }
        Ok(())
    }
    pub fn set_cursor(&mut self, token: &ViewToken, cursor: Cursor) -> Result<(), BridgeError> {
        self.check(token)?;
        let primary = self
            .projection
            .offset(&self.original.text, cursor.position)?;
        let anchor = cursor
            .selection
            .map(|anchor| self.projection.offset(&self.original.text, anchor))
            .transpose()?
            .unwrap_or(primary);
        self.editor
            .set_selection(&self.original.version, anchor, primary)?;
        self.has_secondary_selections = false;
        self.secondary_targets = None;
        self.content = projected_content(&self.projection.text);
        self.record_refresh();
        self.content.move_to(cursor);
        Ok(())
    }
    fn bounds(&self) -> Result<Range<usize>, BridgeError> {
        let cursor = self.content.cursor();
        let caret = self
            .projection
            .offset(&self.original.text, cursor.position)?;
        let anchor = cursor
            .selection
            .map(|p| self.projection.offset(&self.original.text, p))
            .transpose()?
            .unwrap_or(caret);
        Ok(caret.min(anchor)..caret.max(anchor))
    }
    pub fn proposal(&mut self, token: &ViewToken, edit: &Edit) -> Result<Proposal, BridgeError> {
        self.check(token)?;
        let mut range = self.bounds()?;
        if self.native_unicode_enabled() {
            self.check_single_native_selection()?;
            if range.is_empty() && matches!(edit, Edit::Backspace | Edit::Delete) {
                range = self.editor.native_grapheme_deletion_target(
                    &self.original.version,
                    range.start,
                    matches!(edit, Edit::Backspace),
                )?;
            }
        }
        let text = &self.original.text;
        let mut edits = Vec::new();
        let (caret, selection) = match edit {
            Edit::Indent | Edit::Unindent => {
                return Err(BridgeError::NativeLineIntent);
            }
            _ => {
                let inserted = match edit {
                    Edit::Insert(ch) => ch.to_string(),
                    Edit::Paste(text) => {
                        if text.len() > self.max_source || text.len() > self.max_payload {
                            return Err(BridgeError::Capacity);
                        }
                        text.as_str().to_owned()
                    }
                    Edit::Enter => "\n".into(),
                    Edit::Backspace => {
                        if range.is_empty() && !self.editor.native_unicode_enabled() {
                            range.start = text[..range.start]
                                .grapheme_indices(true)
                                .next_back()
                                .map_or(0, |(i, _)| i);
                        }
                        String::new()
                    }
                    Edit::Delete => {
                        if range.is_empty() && !self.editor.native_unicode_enabled() {
                            range.end +=
                                text[range.end..].graphemes(true).next().map_or(0, str::len);
                        }
                        String::new()
                    }
                    Edit::Indent | Edit::Unindent => unreachable!(),
                };
                let caret = range
                    .start
                    .checked_add(inserted.len())
                    .ok_or(BridgeError::Capacity)?;
                if !range.is_empty() || !inserted.is_empty() {
                    edits.push(Replacement {
                        start: range.start,
                        end: range.end,
                        expected: text[range.clone()].into(),
                        inserted,
                    });
                }
                (caret, None)
            }
        };
        let mut length = text.len();
        let mut payload = 0usize;
        for edit in &edits {
            length = length
                .checked_sub(edit.end - edit.start)
                .and_then(|n| n.checked_add(edit.inserted.len()))
                .ok_or(BridgeError::Capacity)?;
            payload = payload
                .checked_add(edit.expected.len())
                .and_then(|n| n.checked_add(edit.inserted.len()))
                .ok_or(BridgeError::Capacity)?;
        }
        if length > self.max_source || payload > self.max_payload || edits.len() > self.max_edits {
            return Err(BridgeError::Capacity);
        }
        Ok(Proposal {
            version: self.original.version.clone(),
            edits,
            caret,
            selection,
        })
    }
    pub fn widget(
        &mut self,
        token: &ViewToken,
        action: text_editor::Action,
    ) -> Result<(), BridgeError> {
        let result = self.widget_inner(token, action);
        self.editor.collect_diagnostics();
        if let Err(error) = &result {
            self.status = error.to_string();
        }
        result
    }
    fn widget_inner(
        &mut self,
        token: &ViewToken,
        action: text_editor::Action,
    ) -> Result<(), BridgeError> {
        // Routing follows the native owner, never a fallible paint cache. An
        // unavailable projection must not turn a multi-edit into a primary edit.
        if self.has_secondary_selections {
            self.check(token)?;
            let native = match &action {
                text_editor::Action::Edit(Edit::Insert(c)) => Action::Insert(c.to_string()),
                text_editor::Action::Edit(Edit::Paste(text)) => {
                    if text.len() > self.max_payload {
                        return Err(BridgeError::Capacity);
                    }
                    Action::Insert((**text).clone())
                }
                text_editor::Action::Edit(Edit::Enter) => Action::Insert("\n".into()),
                text_editor::Action::Edit(Edit::Backspace) => Action::Backspace,
                text_editor::Action::Edit(Edit::Delete) => Action::Delete,
                text_editor::Action::Move(Motion::Left) => Action::Left,
                text_editor::Action::Move(Motion::Right) => Action::Right,
                text_editor::Action::Select(Motion::Left) => {
                    Action::ExtendSelection(vellum_editor::Movement::Left)
                }
                text_editor::Action::Select(Motion::Right) => {
                    Action::ExtendSelection(vellum_editor::Movement::Right)
                }
                text_editor::Action::SelectAll => Action::SelectAll,
                text_editor::Action::Scroll { .. } => {
                    self.content.perform(action);
                    return Ok(());
                }
                _ => return Err(BridgeError::UnsupportedSelectionProfile),
            };
            let result = self.editor.apply(native);
            // Native publication may succeed before a later presentation error.
            // Always retire old geometry and capture the actual resulting state.
            self.refresh_projection()?;
            return result.map_err(Into::into);
        }
        if self.native_unicode_enabled() {
            let (motion, extend) = match &action {
                text_editor::Action::Move(m) => (Some(*m), false),
                text_editor::Action::Select(m) => (Some(*m), true),
                _ => (None, false),
            };
            if let Some(motion @ (Motion::Left | Motion::Right)) = motion {
                self.check(token)?;
                self.check_single_native_selection()?;
                self.editor.apply(if extend {
                    Action::ExtendSelection(if motion == Motion::Left {
                        vellum_editor::Movement::Left
                    } else {
                        vellum_editor::Movement::Right
                    })
                } else if motion == Motion::Left {
                    Action::Left
                } else {
                    Action::Right
                })?;
                let cursor = self.logical_cursor(&self.projection, &self.original.text)?;
                self.move_widget_cursor(cursor);
                self.status = self.editor.status().into();
                return Ok(());
            }
        }
        if let text_editor::Action::Edit(Edit::Indent | Edit::Unindent) = &action {
            let request = if matches!(action, text_editor::Action::Edit(Edit::Indent)) {
                vellum_engine::coordinator::LineEditV1::Indent {
                    unit: "    ".into(),
                    tab_width: 4,
                }
            } else {
                vellum_engine::coordinator::LineEditV1::Outdent {
                    unit: "    ".into(),
                    tab_width: 4,
                }
            };
            return self.line_edit(token, &request);
        }
        if let text_editor::Action::Edit(edit) = action {
            let proposal = self.proposal(token, &edit)?;
            if proposal.edits.is_empty() {
                if self.native_unicode_enabled() && matches!(edit, Edit::Backspace | Edit::Delete) {
                    self.editor.apply(if matches!(edit, Edit::Backspace) {
                        Action::Backspace
                    } else {
                        Action::Delete
                    })?;
                    self.status = self.editor.status().into();
                }
                return Ok(());
            }
            // Prepare only a display candidate; never submit Content::text() as
            // authoritative replacement. Reject unprojectable positions before
            // engine publication rather than normalize bytes or repair targets.
            let length = proposal
                .edits
                .iter()
                .fold(self.original.text.len(), |n, e| {
                    n - (e.end - e.start) + e.inserted.len()
                });
            let mut text = String::new();
            text.try_reserve_exact(length)
                .map_err(|_| BridgeError::Capacity)?;
            let mut consumed = 0;
            for edit in &proposal.edits {
                text.push_str(&self.original.text[consumed..edit.start]);
                text.push_str(&edit.inserted);
                consumed = edit.end;
            }
            text.push_str(&self.original.text[consumed..]);
            let projection = Projection::new(&text);
            let caret = if self.native_unicode_enabled() {
                self.editor
                    .project_candidate_caret_after(&text, proposal.caret)?
            } else {
                text.grapheme_indices(true)
                    .map(|(i, _)| i)
                    .find(|&i| i >= proposal.caret)
                    .unwrap_or(text.len())
            };
            let cursor = Cursor {
                position: projection.position(&text, caret)?,
                selection: proposal
                    .selection
                    .map(|byte| projection.position(&text, byte))
                    .transpose()?,
            };
            // Original-coordinate visual patches, prepared before publication.
            // Ordinary typing does not rebuild or reshape the entire buffer.
            let mut patches = Vec::new();
            patches
                .try_reserve_exact(proposal.edits.len())
                .map_err(|_| BridgeError::Capacity)?;
            for edit in &proposal.edits {
                patches.push((
                    self.projection.position(&self.original.text, edit.start)?,
                    self.projection.position(&self.original.text, edit.end)?,
                    Projection::new(&edit.inserted).text,
                ));
            }
            let published = if self.native_unicode_enabled()
                && matches!(edit, Edit::Backspace | Edit::Delete)
            {
                self.editor.apply(if matches!(edit, Edit::Backspace) {
                    Action::Backspace
                } else {
                    Action::Delete
                })
            } else {
                self.editor
                    .edit_snapshot(&proposal.version, proposal.edits, caret)
            };
            if let Err(error) = published {
                if self.editor.version()? != self.original.version {
                    self.token = ViewToken::fresh();
                    self.refresh_projection()?;
                    self.status =
                        format!("Native state changed; presentation update failed: {error}");
                }
                return Err(error.into());
            }
            let original = self.editor.snapshot()?;
            if original.text != text {
                self.token = ViewToken::fresh();
                self.refresh_projection()?;
                self.status =
                    "Native command completed; display candidate disagreed (old input invalidated)"
                        .into();
                return Err(BridgeError::ProjectionMismatch);
            }
            let cursor = if self.editor.native_views_enabled() {
                self.logical_cursor(&projection, &original.text)?
            } else {
                cursor
            };
            let first_changed = patches
                .iter()
                .map(|(start, _, _)| start.line)
                .min()
                .ok_or(BridgeError::Position)?;
            let last_changed = patches
                .iter()
                .map(|(_, end, _)| end.line)
                .max()
                .ok_or(BridgeError::Position)?;
            let line_delta = self.projection.lines.len().abs_diff(projection.lines.len());
            let first_check = first_changed
                .saturating_sub(1)
                .min(projection.lines.len() - 1);
            let last_check = last_changed
                .saturating_add(line_delta)
                .saturating_add(1)
                .min(projection.lines.len() - 1);
            for (start, end, inserted) in patches.into_iter().rev() {
                if start == end && self.content.cursor().selection.is_some() {
                    self.content
                        .perform(text_editor::Action::Move(Motion::Left));
                }
                self.content.move_to(Cursor {
                    position: end,
                    selection: (start != end).then_some(start),
                });
                self.content
                    .perform(text_editor::Action::Edit(Edit::Paste(Arc::new(inserted))));
            }
            if !content_matches_lines(
                &self.content,
                &original.text,
                &projection,
                first_check,
                last_check,
            ) {
                // EOL joins or framework-specific transformations may need a
                // fresh visual projection; they never change engine source.
                self.content = projected_content(&projection.text);
                self.record_refresh();
                if self.content.text() != projection.text {
                    return Err(BridgeError::ProjectionMismatch);
                }
            }
            if cursor.selection.is_none() && self.content.cursor().selection.is_some() {
                self.content
                    .perform(text_editor::Action::Move(Motion::Left));
            }
            self.content.move_to(cursor);
            self.projection = projection;
            self.original = original;
            self.dirty = self.editor.is_dirty()?;
            self.status = self.editor.status().into();
        } else {
            self.check(token)?;
            if self.native_unicode_enabled() {
                self.check_single_native_selection()?;
            }
            let explicit_point_gesture = matches!(
                action,
                text_editor::Action::Click(_) | text_editor::Action::Drag(_)
            );
            let previous_cursor = self.content.cursor();
            if matches!(
                action,
                text_editor::Action::SelectWord | text_editor::Action::SelectLine
            ) {
                // Content::clone recreates text but loses cursor/selection. Use
                // the actual widget to expand the native selection, then retain
                // its exact bounds as a normal selection (including raw EOL map).
                let primary = self.content.cursor().position;
                self.content.perform(action.clone());
                self.content
                    .perform(text_editor::Action::Move(Motion::Left));
                let start = self.content.cursor().position;
                self.content.move_to(Cursor {
                    position: primary,
                    selection: None,
                });
                self.content.perform(action);
                self.content
                    .perform(text_editor::Action::Move(Motion::Right));
                let end = self.content.cursor().position;
                self.content.move_to(Cursor {
                    position: end,
                    selection: Some(start),
                });
            } else {
                self.content.perform(action);
            }
            // Non-authoring navigation/selection never publishes source/history.
            if self.content.text() != self.projection.text {
                return Err(BridgeError::ProjectionMismatch);
            }
            let cursor = self.content.cursor();
            let mut head = self
                .projection
                .offset(&self.original.text, cursor.position)?;
            let mut anchor = cursor
                .selection
                .map(|p| self.projection.offset(&self.original.text, p))
                .transpose()?
                .unwrap_or(head);
            if self.native_unicode_enabled()
                && (explicit_point_gesture
                    || cursor.position != previous_cursor.position
                    || cursor.selection != previous_cursor.selection)
            {
                // Resolve this new visual gesture before making a logical
                // target. Exact SDK positions and literal matches bypass this.
                let projected = (|| -> Result<_, BridgeError> {
                    Ok((
                        self.editor
                            .project_source_caret_after(&self.original.version, anchor)?,
                        self.editor
                            .project_source_caret_after(&self.original.version, head)?,
                    ))
                })();
                let (a, h) = match projected {
                    Ok(points) => points,
                    Err(error) => {
                        self.move_widget_cursor(previous_cursor);
                        return Err(error);
                    }
                };
                anchor = a;
                head = h;
            }
            if let Err(error) = self
                .editor
                .set_selection(&self.original.version, anchor, head)
            {
                self.content.move_to(previous_cursor);
                return Err(error.into());
            }
            if self.native_unicode_enabled() {
                let cursor = self.logical_cursor(&self.projection, &self.original.text)?;
                self.move_widget_cursor(cursor);
            }
        }
        Ok(())
    }
    fn move_widget_cursor(&mut self, cursor: Cursor) {
        if cursor.selection.is_none() && self.content.cursor().selection.is_some() {
            self.content
                .perform(text_editor::Action::Move(Motion::Left));
        }
        self.content.move_to(cursor);
    }
    fn check_single_native_selection(&mut self) -> Result<(), BridgeError> {
        let snapshot = self
            .editor
            .native_selection()?
            .ok_or(BridgeError::UnsupportedSelectionProfile)?;
        if snapshot.selections.selections().len() != 1 {
            return Err(BridgeError::UnsupportedSelectionProfile);
        }
        let selected = snapshot.selections.selections()[0];
        let cursor = self.content.cursor();
        let head = self
            .projection
            .offset(&self.original.text, cursor.position)?;
        let anchor = cursor
            .selection
            .map(|p| self.projection.offset(&self.original.text, p))
            .transpose()?
            .unwrap_or(head);
        if selected.head.0 != head || selected.anchor.0 != anchor {
            return Err(BridgeError::ProjectionMismatch);
        }
        Ok(())
    }
    /// Execute original native line semantics; the widget receives only the
    /// resulting projection. This explicit block action may rebuild its display;
    /// ordinary literal typing retains incremental patches.
    pub fn line_edit(
        &mut self,
        token: &ViewToken,
        request: &vellum_engine::coordinator::LineEditV1,
    ) -> Result<(), BridgeError> {
        self.check(token)?;
        let result = self.editor.edit_lines(&self.original.version, request);
        self.refresh_projection()?;
        result.map_err(Into::into)
    }
    pub fn command(&mut self, token: &ViewToken, command: Command) -> Result<(), BridgeError> {
        self.check(token)?;
        let result = match command {
            Command::NextMatch => self
                .editor
                .navigate_match(vellum_engine::coordinator::SearchDirection::Next, true),
            Command::PreviousMatch => self
                .editor
                .navigate_match(vellum_engine::coordinator::SearchDirection::Previous, true),
            _ => self.editor.apply(match command {
                Command::Undo => Action::Undo,
                Command::Redo => Action::Redo,
                Command::Save => Action::Save,
                Command::NextDocument => Action::NextDocument,
                Command::ToggleEditingLock => Action::ToggleEditingLock,
                Command::NextMatch | Command::PreviousMatch => unreachable!("handled above"),
            }),
        };
        if matches!(command, Command::Save) {
            if self.editor.version()? != self.original.version {
                return Err(BridgeError::ProjectionMismatch);
            }
            self.dirty = self.editor.is_dirty()?;
            self.status = self.editor.status().into();
            return result.map_err(Into::into);
        }
        if matches!(
            command,
            Command::Undo
                | Command::Redo
                | Command::NextDocument
                | Command::ToggleEditingLock
                | Command::NextMatch
                | Command::PreviousMatch
        ) && result.is_ok()
        {
            self.token = ViewToken::fresh();
        }
        self.refresh_projection()?;
        result.map_err(Into::into)
    }
    pub fn plan_destination(
        &mut self,
        token: &ViewToken,
        path: PathBuf,
        save_as: bool,
    ) -> Result<DestinationPlan, BridgeError> {
        self.check(token)?;
        Ok(self.editor.plan_destination(path, save_as)?)
    }
    pub fn confirm_destination(
        &mut self,
        token: &ViewToken,
        plan: &DestinationPlan,
    ) -> Result<(), BridgeError> {
        self.check(token)?;
        let result = self.editor.confirm_destination(plan);
        self.token = ViewToken::fresh();
        self.refresh_projection()?;
        result.map_err(Into::into)
    }
    pub fn find(&mut self, token: &ViewToken, query: &str) -> Result<(), BridgeError> {
        self.check(token)?;
        self.editor.find(query)?;
        self.token = ViewToken::fresh();
        self.refresh_projection()
    }
    /// Select the existing complete native search result, not a UI-side scan.
    pub fn select_matches(&mut self, token: &ViewToken) -> Result<(), BridgeError> {
        self.check(token)?;
        self.editor.select_matches()?;
        // Conservative until refresh captures the exact resulting selection.
        self.has_secondary_selections = true;
        self.token = ViewToken::fresh();
        self.refresh_projection()
    }
    pub fn replace_match(
        &mut self,
        token: &ViewToken,
        replacement: &str,
        all: bool,
    ) -> Result<(), BridgeError> {
        self.check(token)?;
        self.editor.replace_match(replacement, all)?;
        self.token = ViewToken::fresh();
        self.refresh_projection()
    }
    /// The new attachment spends old UI input even when joining the same owner.
    /// File resolution and imported ownership remain at the existing host.
    pub fn open_document(
        &mut self,
        token: &ViewToken,
        path: &std::path::Path,
    ) -> Result<bool, BridgeError> {
        self.check(token)?;
        let added = self.editor.open_additional(path)?;
        self.token = ViewToken::fresh();
        self.refresh_projection()?;
        Ok(added)
    }
    fn logical_cursor(&self, projection: &Projection, text: &str) -> Result<Cursor, BridgeError> {
        let head = self.editor.cursor();
        let anchor = self.editor.selection().map(|range| {
            if range.start == head {
                range.end
            } else {
                range.start
            }
        });
        Ok(Cursor {
            position: projection.position(text, head)?,
            selection: anchor
                .map(|byte| projection.position(text, byte))
                .transpose()?,
        })
    }
    fn refresh_projection(&mut self) -> Result<(), BridgeError> {
        self.secondary_targets = None;
        let native_selection = self.editor.native_selection()?;
        self.has_secondary_selections = native_selection
            .as_ref()
            .is_some_and(|s| s.selections.selections().len() > 1);
        // Engine state remains authoritative even after a save failure. Capture
        // its actual state; error delivery cannot roll a Written outcome back.
        let original = self.editor.snapshot()?;
        let projection = Projection::new(&original.text);
        let mut content = projected_content(&projection.text);
        if content.text() != projection.text {
            return Err(BridgeError::ProjectionMismatch);
        }
        let reset = if let Ok(cursor) = self.logical_cursor(&projection, &original.text) {
            content.move_to(cursor);
            false
        } else {
            // This is a new visual attachment, never repair of an executable
            // target: spend queued old input and visibly reanchor the display.
            self.token = ViewToken::fresh();
            self.editor.set_caret(0)?;
            true
        };
        let dirty = self.editor.is_dirty()?;
        self.original = original;
        self.content = content;
        self.record_refresh();
        self.projection = projection;
        self.dirty = dirty;
        self.status = if reset {
            "Native command completed; visual caret reset to document start (old input invalidated)"
                .into()
        } else {
            self.editor.status().into()
        };
        if reset {
            self.has_secondary_selections = false;
        } else {
            self.capture_secondary_targets(native_selection)?;
        }
        Ok(())
    }
    pub fn close(&mut self, discard: bool) -> Result<(), BridgeError> {
        if let Some(clipboard) = self.native_clipboard.as_mut() {
            clipboard
                .close(&mut self.editor, discard)
                .map_err(Into::into)
        } else {
            self.editor.close(discard).map_err(Into::into)
        }
    }
}

/// Seed through the public literal-insertion path, not with_text(full_document).
/// The pinned renderer's latter path eagerly shapes the unbounded document
/// before widget layout can supply a viewport. This is UI attachment only: no
/// source publication, widget undo authority or synthetic engine operation.
pub(crate) fn projected_content(text: &str) -> Content {
    let mut content = Content::new();
    // cosmic-text 0.15 inserts middle lines at a fixed Vec index. A single
    // document-sized paste is quadratic in line count. Append bounded batches
    // instead; each batch shifts at most 64 new lines, never the prior prefix.
    // The projection is LF-only, so every boundary is a complete delimiter.
    let mut start = 0;
    for (index, (offset, _)) in text.match_indices('\n').enumerate() {
        if (index + 1).is_multiple_of(64) {
            let end = offset + 1;
            content.perform(text_editor::Action::Edit(Edit::Paste(Arc::new(
                text[start..end].to_owned(),
            ))));
            start = end;
        }
    }
    if start < text.len() {
        content.perform(text_editor::Action::Edit(Edit::Paste(Arc::new(
            text[start..].to_owned(),
        ))));
    }
    content.move_to(Cursor {
        position: Position { line: 0, column: 0 },
        selection: None,
    });
    content
}

/// LF-only visual lines plus original body coordinates. This is a lossy display
/// projection with an explicit map, NEVER a serialization or source replacement.
/// In particular LF+CR and LF+CRLF retain the engine's two distinct delimiters;
/// the widget cannot greedily reinterpret them as its legacy LFCR delimiter.
pub(crate) struct Projection {
    pub(crate) text: String,
    lines: Vec<(usize, usize)>,
}
/// Initial attachments are fully checked. This private buffer is authored only
/// by the bridge's explicit patches; pinned widget layout/navigation do not edit
/// it. Preserve coherence inductively: check selected input lines, then every
/// changed output line plus adjacent join/split boundaries and total line count.
/// Full-buffer oracles remain in conformance/workloads, not each ordinary key.
fn content_matches_lines(
    content: &Content,
    source: &str,
    projection: &Projection,
    first: usize,
    last: usize,
) -> bool {
    if content.line_count() != projection.lines.len()
        || first > last
        || last >= projection.lines.len()
    {
        return false;
    }
    for index in first..=last {
        let Some(line) = content.line(index) else {
            return false;
        };
        let (start, length) = projection.lines[index];
        if line.text.as_ref() != &source[start..start + length] {
            return false;
        }
        if index + 1 < projection.lines.len() {
            let ending = if line.ending == text_editor::LineEnding::None {
                text_editor::LineEnding::default()
            } else {
                line.ending
            };
            if ending.as_str() != "\n" {
                return false;
            }
        }
    }
    true
}
impl Projection {
    pub(crate) fn new(source: &str) -> Self {
        let mut text = String::with_capacity(source.len());
        let mut lines = Vec::new();
        let mut start = 0;
        let mut index = 0;
        while index < source.len() {
            if matches!(source.as_bytes()[index], b'\r' | b'\n') {
                lines.push((start, index - start));
                text.push_str(&source[start..index]);
                text.push('\n');
                index += if source.as_bytes()[index] == b'\r'
                    && source.as_bytes().get(index + 1) == Some(&b'\n')
                {
                    2
                } else {
                    1
                };
                start = index;
            } else {
                index += 1;
            }
        }
        lines.push((start, source.len() - start));
        text.push_str(&source[start..]);
        Self { text, lines }
    }
    pub(crate) fn offset(&self, source: &str, position: Position) -> Result<usize, BridgeError> {
        let &(start, len) = self.lines.get(position.line).ok_or(BridgeError::Position)?;
        if position.column > len || !source.is_char_boundary(start + position.column) {
            return Err(BridgeError::Position);
        }
        Ok(start + position.column)
    }
    pub(crate) fn position(&self, source: &str, byte: usize) -> Result<Position, BridgeError> {
        for (line, &(start, len)) in self.lines.iter().enumerate() {
            if byte >= start && byte <= start + len {
                let position = Position {
                    line,
                    column: byte - start,
                };
                self.offset(source, position)?;
                return Ok(position);
            }
        }
        Err(BridgeError::Position)
    }
}

#[cfg(test)]
mod coherence_tests {
    use super::*;
    #[test]
    fn lazy_attachment_preserves_literal_data_delimiters_and_initial_caret() {
        let boundary = "Grüße 界 👩‍💻\r\n\n".repeat(130) + "tail";
        for source in [
            "",
            "\u{feff}a\t\0é\u{202e}\n\nZ\n",
            "A\r\nB\rC\n\rD",
            &boundary,
        ] {
            let projection = Projection::new(source);
            let content = projected_content(&projection.text);
            assert_eq!(content.text(), projection.text);
            assert_eq!(content.cursor().position, Position { line: 0, column: 0 });
            assert!(content.cursor().selection.is_none());
            assert!(content_matches_lines(
                &content,
                source,
                &projection,
                0,
                projection.lines.len() - 1
            ));
        }
    }
    #[test]
    fn incremental_checks_detect_changed_lines_and_line_structure_without_serializing_source() {
        let empty = Content::with_text("");
        assert!(content_matches_lines(
            &empty,
            "",
            &Projection::new(""),
            0,
            0
        ));
        let source = "é\r\nSecond\nThird";
        let projection = Projection::new(source);
        let mut content = Content::with_text(&projection.text);
        assert!(content_matches_lines(&content, source, &projection, 0, 2));
        content.move_to(Cursor {
            position: Position { line: 1, column: 0 },
            selection: None,
        });
        content.perform(text_editor::Action::Edit(Edit::Insert('X')));
        assert!(!content_matches_lines(&content, source, &projection, 1, 1));
        assert!(!content_matches_lines(&content, source, &projection, 0, 2));
        content.perform(text_editor::Action::Edit(Edit::Enter));
        assert!(!content_matches_lines(&content, source, &projection, 0, 0));
    }
}
