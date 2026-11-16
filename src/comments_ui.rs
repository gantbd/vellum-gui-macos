//! Presentation-only forms over the existing editor annotation owner.
use super::*;
use crate::comment_draft::Draft;
use iced::widget::{column, scrollable};
use vellum_editor::{
    CommentDraft, CommentHandle, CommentHistoryDirection, CommentLocationEvidence,
    CommentPlacement, CommentPointProfile, CommentTarget,
};

#[derive(Default)]
pub(super) struct CommentPane {
    pub(super) open: bool,
    error: Option<String>,
    form: Option<Form>,
    path: String,
    pending: Option<(ViewToken, vellum_editor::CommentFilePlan)>,
    dirty: bool,
}
struct Form {
    source: ViewToken,
    target: Target,
    body: Draft,
}
enum Target {
    Create(CommentDraft),
    Edit(CommentHandle),
}
// Tokens and native handles are exact identities, not visible row indices.
#[derive(Clone)]
pub enum CommentMessage {
    FilePath(ViewToken, String),
    FileRequest(ViewToken, bool),
    FileConfirm(ViewToken, vellum_editor::CommentFilePlan),
    FileCancel,
    FileReleaseFailed(ViewToken),
    Toggle(ViewToken),
    Begin(ViewToken, CommentPointProfile),
    Edit(ViewToken, CommentHandle),
    Delete(ViewToken, CommentHandle),
    Attach(ViewToken, CommentHandle, CommentPointProfile),
    Lock(ViewToken, bool),
    Reverse(ViewToken, CommentHistoryDirection),
    Action(ViewToken, text_editor::Action),
    History(ViewToken, bool),
    Apply(ViewToken),
    Cancel(ViewToken),
}
impl std::fmt::Debug for CommentMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CommentMessage(<redacted>)")
    }
}
impl CommentPane {
    pub(super) fn has_draft(&self) -> bool {
        self.form.is_some()
    }
    #[cfg(test)]
    pub(super) fn form_token(&self) -> ViewToken {
        self.form.as_ref().expect("open form").body.token.clone()
    }
    #[cfg(test)]
    pub(super) fn form_raw(&self) -> &str {
        self.form.as_ref().expect("open form").body.raw()
    }
    #[cfg(test)]
    pub(super) fn form_editor(&self) -> Element<'_, Message> {
        comment_editor(self.form.as_ref().expect("open form"))
    }
    pub(super) fn refresh(&mut self, bridge: Option<&mut Bridge>) {
        if !self.open && self.form.is_none() {
            return;
        }
        self.error = bridge
            .map(|b| {
                b.refresh_comments(&b.token())?;
                self.dirty = b.comments_dirty()?;
                Ok::<(), BridgeError>(())
            })
            .transpose()
            .err()
            .map(|e| e.to_string());
    }
    pub(super) fn update(
        &mut self,
        bridge: Option<&mut Bridge>,
        message: CommentMessage,
    ) -> Result<(), BridgeError> {
        // Form messages carry a separate attachment. Native Apply still uses
        // the original source attachment and native draft/record versions.
        let form_token = match &message {
            CommentMessage::Action(t, _)
            | CommentMessage::History(t, _)
            | CommentMessage::Apply(t)
            | CommentMessage::Cancel(t) => Some(t),
            _ => None,
        };
        if let Some(token) = form_token
            && !self.form.as_ref().is_some_and(|f| f.body.token == *token)
        {
            return Err(BridgeError::Position);
        }
        let Some(bridge) = bridge else {
            return Err(BridgeError::Position);
        };
        if !matches!(
            &message,
            CommentMessage::FileConfirm(..) | CommentMessage::FileCancel
        ) && self.pending.take().is_some()
        {
            bridge.cancel_comment_file_plan();
        }
        match message {
            CommentMessage::FilePath(token, path) => {
                if bridge.token() != token || path.len() > 4096 {
                    return Err(BridgeError::Position);
                }
                self.path = path;
            }
            CommentMessage::FileRequest(token, load) => {
                if self.has_draft() {
                    return Err(BridgeError::Position);
                }
                if bridge.comment_file_bound() {
                    bridge.comment_file_action(&token, load)?;
                } else {
                    let plan = bridge.plan_comment_file(&token, PathBuf::from(&self.path), load)?;
                    self.pending = Some((token, plan));
                }
            }
            CommentMessage::FileConfirm(token, plan) => {
                if self.has_draft()
                    || !self
                        .pending
                        .as_ref()
                        .is_some_and(|(t, p)| *t == token && *p == plan)
                {
                    return Err(BridgeError::Position);
                }
                self.pending = None;
                bridge.confirm_comment_file(&token, &plan)?;
            }
            CommentMessage::FileCancel => {
                self.pending = None;
                bridge.cancel_comment_file_plan();
            }
            CommentMessage::FileReleaseFailed(token) => {
                bridge.release_failed_comment_read(&token)?;
            }
            CommentMessage::Action(_, action) => self
                .form
                .as_mut()
                .expect("checked form")
                .body
                .action(action)?,
            CommentMessage::History(_, redo) => self
                .form
                .as_mut()
                .expect("checked form")
                .body
                .history(redo)?,
            CommentMessage::Cancel(_) => self.form = None,
            CommentMessage::Apply(_) => {
                let form = self.form.as_ref().expect("checked form");
                match &form.target {
                    Target::Create(draft) => {
                        bridge.create_comment(&form.source, draft, form.body.raw())?
                    }
                    Target::Edit(handle) => {
                        bridge.update_comment(&form.source, handle, form.body.raw())?
                    }
                }
                self.form = None;
            }
            CommentMessage::Toggle(token) => {
                bridge.refresh_comments(&token)?;
                self.open = !self.open;
            }
            CommentMessage::Begin(token, profile) => {
                if self.form.is_some() {
                    return Err(BridgeError::Position);
                }
                let target = bridge.begin_comment(&token, profile)?;
                let body = Draft::new(
                    "",
                    bridge.comment_body_limit().ok_or(BridgeError::Capacity)?,
                )?;
                self.form = Some(Form {
                    source: token,
                    target: Target::Create(target),
                    body,
                });
                self.open = true;
            }
            CommentMessage::Edit(token, handle) => {
                if self.form.is_some() {
                    return Err(BridgeError::Position);
                }
                bridge.refresh_comments(&token)?;
                let raw = &bridge
                    .cached_comments()
                    .iter()
                    .find(|r| r.handle == handle)
                    .ok_or(BridgeError::Position)?
                    .body;
                let body = Draft::new(
                    raw,
                    bridge.comment_body_limit().ok_or(BridgeError::Capacity)?,
                )?;
                self.form = Some(Form {
                    source: token,
                    target: Target::Edit(handle),
                    body,
                });
                self.open = true;
            }
            CommentMessage::Delete(token, handle) => bridge.delete_comment(&token, &handle)?,
            CommentMessage::Attach(token, handle, profile) => {
                bridge.reattach_comment(&token, &handle, profile)?
            }
            CommentMessage::Lock(token, locked) => bridge.lock_comments(&token, locked)?,
            CommentMessage::Reverse(token, direction) => {
                bridge.reverse_comment(&token, direction)?
            }
        }
        Ok(())
    }
    pub(super) fn view<'a>(&'a self, bridge: &'a Bridge) -> Element<'a, Message> {
        let token = bridge.token();
        let send = |m| Message::Comment(m);
        let controls = row![
            text(if self.dirty {
                "Comments — UNSAVED"
            } else {
                "Comments — no unsaved changes"
            }),
            button("Add content").on_press(send(CommentMessage::Begin(
                token.clone(),
                CommentPointProfile::Content
            ))),
            button("Add position").on_press(send(CommentMessage::Begin(
                token.clone(),
                CommentPointProfile::Position
            ))),
            button(if bridge.comments_locked() {
                "Unlock notes"
            } else {
                "Lock notes"
            })
            .on_press(send(CommentMessage::Lock(
                token.clone(),
                !bridge.comments_locked()
            ))),
            button("Undo note").on_press(send(CommentMessage::Reverse(
                token.clone(),
                CommentHistoryDirection::Undo
            ))),
            button("Redo note").on_press(send(CommentMessage::Reverse(
                token.clone(),
                CommentHistoryDirection::Redo
            ))),
        ]
        .spacing(6)
        .wrap();
        let mut pane = column![controls, text("Source save does not persist notes. Note history may reverse linked source history.")].spacing(6);
        let path_token = token.clone();
        let mut file_controls = row![
            button("Save JSON").on_press(send(CommentMessage::FileRequest(token.clone(), false))),
            button("Load JSON").on_press(send(CommentMessage::FileRequest(token.clone(), true))),
            button("Release failed read")
                .on_press(send(CommentMessage::FileReleaseFailed(token.clone()))),
        ]
        .spacing(6);
        if !bridge.comment_file_bound() {
            file_controls = file_controls.push(
                text_input("Existing annotation JSON destination", &self.path).on_input(move |s| {
                    Message::Comment(CommentMessage::FilePath(path_token.clone(), s))
                }),
            );
        } else {
            file_controls =
                file_controls.push(text("Explicitly associated file; no automatic sidecar"));
        }
        pane = pane.push(file_controls.wrap());
        if bridge.comment_file_needs_load() {
            pane = pane.push(text(
                "Load incomplete: ordinary Save is fenced; original JSON is preserved.",
            ));
        }
        if let Some((origin, plan)) = &self.pending {
            let path: String = plan
                .destination()
                .to_string_lossy()
                .chars()
                .flat_map(char::escape_default)
                .collect();
            pane = pane
                .push(text(format!(
                    "{} existing file: {path}",
                    if plan.is_load() {
                        "Confirm load from"
                    } else {
                        "Confirm overwrite of"
                    }
                )))
                .push(
                    row![
                        button("Confirm selected file").on_press(send(
                            CommentMessage::FileConfirm(origin.clone(), plan.clone())
                        )),
                        button("Cancel file").on_press(send(CommentMessage::FileCancel))
                    ]
                    .spacing(8)
                    .wrap(),
                );
        }
        if let Some(error) = &self.error {
            pane = pane.push(text(format!("Comments unavailable: {error}")));
        } else {
            let mut rows = column![].spacing(6);
            for note in bridge.cached_comments() {
                let preview: String = note
                    .body
                    .chars()
                    .flat_map(char::escape_default)
                    .take(160)
                    .collect();
                let location = match note.placement {
                    CommentPlacement::Located { target, evidence } => {
                        let target = match target {
                            CommentTarget::Document => "document".into(),
                            CommentTarget::Point {
                                offset, profile, ..
                            } => format!("point byte {offset} {profile:?}"),
                            CommentTarget::ContentRange { start, end } => {
                                format!("bytes {start}..{end}")
                            }
                        };
                        format!(
                            "{target}{}",
                            if evidence == CommentLocationEvidence::Affected {
                                " affected"
                            } else {
                                ""
                            }
                        )
                    }
                    CommentPlacement::Detached => "detached".into(),
                    CommentPlacement::Unavailable => "unavailable".into(),
                };
                rows = rows.push(column![
                    text(format!("[{location}] {preview}")),
                    row![
                        button("Edit").on_press(send(CommentMessage::Edit(
                            token.clone(),
                            note.handle.clone()
                        ))),
                        button("Delete").on_press(send(CommentMessage::Delete(
                            token.clone(),
                            note.handle.clone()
                        ))),
                        button("Attach content").on_press(send(CommentMessage::Attach(
                            token.clone(),
                            note.handle.clone(),
                            CommentPointProfile::Content
                        ))),
                        button("Attach position").on_press(send(CommentMessage::Attach(
                            token.clone(),
                            note.handle.clone(),
                            CommentPointProfile::Position
                        ))),
                    ]
                    .spacing(6)
                    .wrap(),
                ]);
            }
            pane = pane.push(scrollable(rows).height(120));
        }
        if let Some(form) = &self.form {
            pane = pane.push(text("Unapplied literal body — Cmd-S applies to session; Cmd-Z/Shift-Z only changes this form."))
                .push(comment_editor(form))
                .push(row![
                    button("Apply to session").on_press(send(CommentMessage::Apply(form.body.token.clone()))),
                    button("Cancel form").on_press(send(CommentMessage::Cancel(form.body.token.clone()))),
                ].spacing(8).wrap());
        }
        pane.into()
    }
}
fn comment_editor(form: &Form) -> Element<'_, Message> {
    let input = form.body.token.clone();
    let binding = input.clone();
    crate::attachment::attached(
        input.clone(),
        text_editor(form.body.content())
            .font(iced::Font::MONOSPACE)
            .height(120)
            .padding(12)
            .on_action(move |action| {
                Message::Comment(CommentMessage::Action(input.clone(), action))
            })
            .key_binding(move |press| {
                let message = match shortcut(&press.key, press.modifiers) {
                    Some(Shortcut::Command(Command::Save)) => {
                        Some(CommentMessage::Apply(binding.clone()))
                    }
                    Some(Shortcut::Command(Command::Undo)) => {
                        Some(CommentMessage::History(binding.clone(), false))
                    }
                    Some(Shortcut::Command(Command::Redo)) => {
                        Some(CommentMessage::History(binding.clone(), true))
                    }
                    Some(_) => return None,
                    None => None,
                };
                if let Some(message) = message {
                    return Some(text_editor::Binding::Custom(Message::Comment(message)));
                }
                match text_editor::Binding::from_key_press(press) {
                    Some(
                        text_editor::Binding::Copy
                        | text_editor::Binding::Cut
                        | text_editor::Binding::Paste,
                    ) => None,
                    other => other,
                }
            }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        sync::{
            Arc,
            atomic::{AtomicU64, Ordering},
        },
    };
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            loop {
                let directory = std::env::temp_dir().join(format!(
                    "vellum-gui-notes-{}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                ));
                match fs::create_dir(&directory) {
                    Ok(()) => {
                        fs::write(directory.join("a"), b"A\r\n").unwrap();
                        fs::write(directory.join("b"), b"B\r\n").unwrap();
                        fs::write(directory.join("notes"), b"old notes destination").unwrap();
                        return Self(directory);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(e) => panic!("fixture: {e}"),
                }
            }
        }
        fn gui(&self) -> Gui {
            Gui::open(
                crate::reference_workspace_host(),
                &[self.0.join("a"), self.0.join("b")],
            )
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_file(self.0.join("a")).unwrap();
            fs::remove_file(self.0.join("b")).unwrap();
            fs::remove_file(self.0.join("notes")).unwrap();
            fs::remove_dir(&self.0).unwrap();
        }
    }
    fn send(gui: &mut Gui, message: CommentMessage) {
        let _ = gui.update(Message::Comment(message));
    }
    fn source(gui: &Gui) -> ViewToken {
        gui.bridge.as_ref().unwrap().token()
    }
    fn form(gui: &Gui) -> ViewToken {
        gui.comments.form.as_ref().unwrap().body.token.clone()
    }
    fn begin(gui: &mut Gui, profile: CommentPointProfile, body: &str) -> ViewToken {
        send(gui, CommentMessage::Begin(source(gui), profile));
        assert!(gui.notice.is_empty(), "{}", gui.notice);
        let token = form(gui);
        send(
            gui,
            CommentMessage::Action(
                token.clone(),
                text_editor::Action::Edit(text_editor::Edit::Paste(Arc::new(body.into()))),
            ),
        );
        assert!(gui.notice.is_empty(), "{}", gui.notice);
        token
    }
    fn apply(gui: &mut Gui, token: ViewToken) {
        send(gui, CommentMessage::Apply(token));
        assert!(gui.notice.is_empty(), "{}", gui.notice);
    }
    #[test]
    fn native_comments_have_separate_lock_history_dirty_and_lossless_raw_forms() {
        let fixture = Fixture::new();
        let mut gui = fixture.gui();
        let raw = "\u{feff}note\r\nGrüße\n\r\x1b$(inert)";
        let token = begin(&mut gui, CommentPointProfile::Content, raw);
        assert!(!gui.bridge.as_ref().unwrap().dirty());
        apply(&mut gui, token);
        let bridge = gui.bridge.as_ref().unwrap();
        assert!(bridge.dirty());
        assert_eq!(bridge.cached_comments()[0].body, raw);
        assert!(matches!(
            bridge.cached_comments()[0].authored_target,
            CommentTarget::Point {
                profile: CommentPointProfile::Content,
                ..
            }
        ));
        let handle = bridge.cached_comments()[0].handle.clone();
        let _ = gui.update(Message::Shortcut(
            source(&gui),
            Shortcut::Command(Command::Save),
        ));
        assert!(gui.bridge.as_ref().unwrap().dirty());
        assert_eq!(fs::read(fixture.0.join("a")).unwrap(), b"A\r\n");
        assert_eq!(fs::read_dir(&fixture.0).unwrap().count(), 3);
        let token = source(&gui);
        send(&mut gui, CommentMessage::Lock(token.clone(), true));
        send(
            &mut gui,
            CommentMessage::Delete(token.clone(), handle.clone()),
        );
        assert!(!gui.notice.is_empty());
        assert_eq!(gui.bridge.as_ref().unwrap().cached_comments()[0].body, raw);
        send(&mut gui, CommentMessage::Lock(token.clone(), false));
        send(&mut gui, CommentMessage::Edit(token, handle.clone()));
        let token = form(&gui);
        assert_eq!(gui.comments.form.as_ref().unwrap().body.raw(), raw);
        send(
            &mut gui,
            CommentMessage::Action(
                token.clone(),
                text_editor::Action::Move(text_editor::Motion::DocumentEnd),
            ),
        );
        send(
            &mut gui,
            CommentMessage::Action(
                token.clone(),
                text_editor::Action::Edit(text_editor::Edit::Insert('!')),
            ),
        );
        apply(&mut gui, token);
        let handle = gui.bridge.as_ref().unwrap().cached_comments()[0]
            .handle
            .clone();
        let token = source(&gui);
        send(
            &mut gui,
            CommentMessage::Attach(token, handle, CommentPointProfile::Position),
        );
        assert!(gui.notice.is_empty(), "{}", gui.notice);
        let handle = gui.bridge.as_ref().unwrap().cached_comments()[0]
            .handle
            .clone();
        let token = source(&gui);
        send(&mut gui, CommentMessage::Delete(token.clone(), handle));
        assert!(gui.bridge.as_ref().unwrap().cached_comments().is_empty());
        send(
            &mut gui,
            CommentMessage::Reverse(token, CommentHistoryDirection::Undo),
        );
        assert_eq!(
            gui.bridge.as_ref().unwrap().cached_comments()[0].body,
            format!("{raw}!")
        );
        let token = source(&gui);
        send(
            &mut gui,
            CommentMessage::Reverse(token, CommentHistoryDirection::Redo),
        );
        assert!(gui.bridge.as_ref().unwrap().cached_comments().is_empty());
        let _ = gui.update(Message::Shortcut(
            source(&gui),
            Shortcut::Command(Command::Save),
        ));
        assert_eq!(fs::read(fixture.0.join("a")).unwrap(), b"A\r\n");
        assert_eq!(gui.bridge.as_ref().unwrap().content().text(), "A\n");
        gui.bridge.as_mut().unwrap().close(true).unwrap();
    }
    #[test]
    fn stale_form_target_and_attachment_aba_cannot_retarget_or_overwrite_another_form() {
        let fixture = Fixture::new();
        let mut gui = fixture.gui();
        let first = begin(&mut gui, CommentPointProfile::Position, "keep me");
        let _ = gui.update(Message::Shortcut(
            source(&gui),
            Shortcut::Command(Command::NextDocument),
        ));
        send(&mut gui, CommentMessage::Apply(first.clone()));
        assert!(!gui.notice.is_empty());
        assert_eq!(gui.comments.form.as_ref().unwrap().body.raw(), "keep me");
        assert!(gui.bridge.as_ref().unwrap().cached_comments().is_empty());
        let _ = gui.update(Message::Shortcut(
            source(&gui),
            Shortcut::Command(Command::NextDocument),
        ));
        send(&mut gui, CommentMessage::Apply(first.clone()));
        assert!(!gui.notice.is_empty());
        send(&mut gui, CommentMessage::Cancel(first.clone()));
        let second = begin(&mut gui, CommentPointProfile::Position, "new body");
        send(&mut gui, CommentMessage::Apply(first.clone()));
        assert!(!gui.notice.is_empty());
        send(&mut gui, CommentMessage::Cancel(first));
        assert!(gui.comments.has_draft());
        apply(&mut gui, second);
        assert_eq!(
            gui.bridge.as_ref().unwrap().cached_comments()[0].body,
            "new body"
        );
        assert!(matches!(
            gui.bridge.as_ref().unwrap().cached_comments()[0].authored_target,
            CommentTarget::Point {
                profile: CommentPointProfile::Position,
                ..
            }
        ));
        gui.bridge.as_mut().unwrap().close(true).unwrap();
    }
    #[test]
    fn comment_files_require_confirmation_and_round_trip_separately_from_source() {
        let fixture = Fixture::new();
        let mut gui = fixture.gui();
        let raw = "Grüße\r\nkept literally";
        let token = begin(&mut gui, CommentPointProfile::Position, raw);
        apply(&mut gui, token);
        let token = source(&gui);
        send(
            &mut gui,
            CommentMessage::FilePath(token.clone(), fixture.0.join("notes").display().to_string()),
        );
        send(&mut gui, CommentMessage::FileRequest(token, false));
        assert_eq!(
            fs::read(fixture.0.join("notes")).unwrap(),
            b"old notes destination"
        );
        let (token, plan) = gui.comments.pending.clone().unwrap();
        send(
            &mut gui,
            CommentMessage::FileConfirm(token.clone(), plan.clone()),
        );
        assert!(gui.notice.is_empty(), "{}", gui.notice);
        assert!(!gui.bridge.as_ref().unwrap().dirty());
        let written = fs::read(fixture.0.join("notes")).unwrap();
        send(&mut gui, CommentMessage::FileConfirm(token, plan));
        assert!(!gui.notice.is_empty());
        assert_eq!(fs::read(fixture.0.join("notes")).unwrap(), written);
        gui.bridge.as_mut().unwrap().close(false).unwrap();

        let mut reopened = fixture.gui();
        let token = source(&reopened);
        send(&mut reopened, CommentMessage::Toggle(token.clone()));
        send(
            &mut reopened,
            CommentMessage::FilePath(token.clone(), fixture.0.join("notes").display().to_string()),
        );
        send(&mut reopened, CommentMessage::FileRequest(token, true));
        let (token, plan) = reopened.comments.pending.clone().unwrap();
        send(&mut reopened, CommentMessage::FileConfirm(token, plan));
        assert!(reopened.notice.is_empty(), "{}", reopened.notice);
        assert_eq!(
            reopened.bridge.as_ref().unwrap().cached_comments()[0].body,
            raw
        );
        assert!(matches!(
            reopened.bridge.as_ref().unwrap().cached_comments()[0].authored_target,
            CommentTarget::Point {
                profile: CommentPointProfile::Position,
                ..
            }
        ));
        assert!(!reopened.bridge.as_ref().unwrap().dirty());
        assert_eq!(fs::read(fixture.0.join("a")).unwrap(), b"A\r\n");
        reopened.bridge.as_mut().unwrap().close(false).unwrap();
    }
    #[test]
    fn comment_file_confirmation_cannot_cross_source_attachment_changes() {
        let fixture = Fixture::new();
        let mut gui = fixture.gui();
        let token = begin(&mut gui, CommentPointProfile::Content, "note");
        apply(&mut gui, token);
        let token = source(&gui);
        send(
            &mut gui,
            CommentMessage::FilePath(token.clone(), fixture.0.join("notes").display().to_string()),
        );
        send(&mut gui, CommentMessage::FileRequest(token, false));
        let (token, plan) = gui.comments.pending.clone().unwrap();
        let _ = gui.update(Message::Shortcut(
            source(&gui),
            Shortcut::Command(Command::NextDocument),
        ));
        send(&mut gui, CommentMessage::FileConfirm(token, plan));
        assert!(!gui.notice.is_empty());
        assert_eq!(
            fs::read(fixture.0.join("notes")).unwrap(),
            b"old notes destination"
        );
        gui.bridge.as_mut().unwrap().close(true).unwrap();
    }
    #[test]
    fn scratch_history_spends_form_token_and_pending_form_requires_fresh_discard_consent() {
        let fixture = Fixture::new();
        let mut gui = fixture.gui();
        let original = begin(&mut gui, CommentPointProfile::Content, "unsent");
        send(&mut gui, CommentMessage::History(original.clone(), false));
        assert_ne!(form(&gui), original);
        send(&mut gui, CommentMessage::Apply(original));
        assert!(!gui.notice.is_empty());
        assert!(gui.bridge.as_ref().unwrap().cached_comments().is_empty());
        let _ = gui.update(Message::NativeClose(window::Id::unique()));
        assert!(gui.confirming);
        let token = form(&gui);
        send(&mut gui, CommentMessage::History(token, true));
        assert!(!gui.confirming);
        let _ = gui.update(Message::Discard(source(&gui)));
        assert!(!gui.bridge.as_ref().unwrap().is_closed());
        assert_eq!(gui.comments.form.as_ref().unwrap().body.raw(), "unsent");
        let _ = gui.update(Message::NativeClose(window::Id::unique()));
        let _ = gui.update(Message::Discard(source(&gui)));
        assert!(gui.bridge.as_ref().unwrap().is_closed());
        assert!(!gui.comments.has_draft());
        assert_eq!(fs::read(fixture.0.join("a")).unwrap(), b"A\r\n");
    }

    #[test]
    fn source_clipboard_refuses_stale_attachments_and_unapplied_note_forms_before_binding() {
        let fixture = Fixture::new();
        let mut gui = fixture.gui();
        let original = source(&gui);
        let _ = gui.update(Message::Shortcut(
            original.clone(),
            Shortcut::Command(Command::NextDocument),
        ));
        let _ = gui.update(Message::Clipboard(
            original.clone(),
            Some(ClipboardAction::Copy),
        ));
        assert!(gui.notice.contains("Stale"), "{}", gui.notice);
        let _ = gui.update(Message::Shortcut(
            source(&gui),
            Shortcut::Command(Command::NextDocument),
        ));
        let _ = gui.update(Message::Clipboard(original, Some(ClipboardAction::Paste)));
        assert!(gui.notice.contains("Stale"), "{}", gui.notice);
        let form = begin(&mut gui, CommentPointProfile::Content, "unapplied");
        let _ = gui.update(Message::Clipboard(source(&gui), Some(ClipboardAction::Cut)));
        assert!(gui.notice.contains("Unapplied"));
        assert_eq!(gui.comments.form.as_ref().unwrap().body.raw(), "unapplied");
        assert_eq!(
            gui.bridge.as_ref().unwrap().native_clipboard_stage(),
            Some(vellum_clipboard_macos::ClipboardStage::Idle)
        );
        send(&mut gui, CommentMessage::Cancel(form));
        gui.bridge.as_mut().unwrap().close(false).unwrap();
        assert_eq!(fs::read(fixture.0.join("a")).unwrap(), b"A\r\n");
    }
}
