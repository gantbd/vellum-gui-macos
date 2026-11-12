use crate::{Bridge, BridgeError, Command, ViewToken, plan_workspace_roots};
use iced::{
    Element, Subscription, Task, event,
    keyboard::{self, Key, Modifiers},
    widget::{button, column, container, row, scrollable, text, text_editor, text_input},
    window,
};
use std::path::PathBuf;
use vellum_adapter_clipboard_macos::Operation;
use vellum_model::{DestinationPlan, EditorError};
#[path = "comments_ui.rs"]
mod comments_ui;
use comments_ui::{CommentMessage, CommentPane};
#[cfg(feature = "desktop-qualification")]
#[path = "desktop_qualification.rs"]
pub(crate) mod qualification;

pub struct Gui {
    bridge: Option<Bridge>,
    notice: String,
    confirming: bool,
    path: String,
    query: String,
    replacement: String,
    destination: Option<(ViewToken, DestinationPlan)>,
    comments: CommentPane,
    exit_requested: bool,
}
#[derive(Clone, Copy, Debug)]
pub enum Shortcut {
    Command(Command),
    Quit,
}
#[derive(Clone)]
pub enum Message {
    Clipboard(ViewToken, Option<Operation>),
    Widget(ViewToken, text_editor::Action),
    Shortcut(ViewToken, Shortcut),
    NativeClose(window::Id),
    Discard(ViewToken),
    Cancel,
    Exit,
    PathChanged(String),
    OpenPath,
    QueryChanged(ViewToken, String),
    ReplacementChanged(ViewToken, String),
    Find(ViewToken),
    SelectMatches(ViewToken),
    Replace(ViewToken, bool),
    RequestDestination(ViewToken, bool),
    ConfirmDestination(DestinationPlan),
    CancelDestination,
    Comment(CommentMessage),
}
// Framework diagnostics must never serialize authored text, paste payloads,
// selected fragments or native identities through a derived Message::Debug.
impl std::fmt::Debug for Message {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Clipboard(..) => "Clipboard(<redacted>)",
            Self::Widget(..) => "Widget(<redacted>)",
            Self::Shortcut(..) => "Shortcut(<redacted>)",
            Self::NativeClose(..) => "NativeClose",
            Self::Discard(..) => "Discard",
            Self::Cancel => "Cancel",
            Self::Exit => "Exit",
            Self::PathChanged(..) => "PathChanged(<redacted>)",
            Self::OpenPath => "OpenPath",
            Self::QueryChanged(..) => "QueryChanged(<redacted>)",
            Self::ReplacementChanged(..) => "ReplacementChanged(<redacted>)",
            Self::Find(..) => "Find",
            Self::SelectMatches(..) => "SelectMatches",
            Self::Replace(..) => "Replace(<redacted>)",
            Self::RequestDestination(..) => "RequestDestination(<redacted>)",
            Self::ConfirmDestination(..) => "ConfirmDestination(<redacted>)",
            Self::CancelDestination => "CancelDestination",
            Self::Comment(..) => "Comment(<redacted>)",
        })
    }
}
impl Gui {
    pub fn open(paths: &[PathBuf]) -> Self {
        if paths.is_empty() {
            return Self {
                bridge: None,
                notice: "Open an existing local file (64 KiB reference profile)".into(),
                confirming: false,
                path: String::new(),
                query: String::new(),
                replacement: String::new(),
                destination: None,
                comments: CommentPane::default(),
                exit_requested: false,
            };
        }
        match Bridge::open_reference(paths) {
            Ok(bridge) => Self {
                bridge: Some(bridge),
                notice: String::new(),
                confirming: false,
                path: String::new(),
                query: String::new(),
                replacement: String::new(),
                destination: None,
                comments: CommentPane::default(),
                exit_requested: false,
            },
            Err(error) => Self {
                bridge: None,
                notice: error.to_string(),
                confirming: false,
                path: String::new(),
                query: String::new(),
                replacement: String::new(),
                destination: None,
                comments: CommentPane::default(),
                exit_requested: false,
            },
        }
    }
    pub fn title(&self) -> String {
        self.bridge.as_ref().map_or_else(
            || "Vellum — open file".into(),
            |b| {
                format!(
                    "Vellum — document {}/{}{}{}",
                    b.active_document() + 1,
                    b.document_count(),
                    if b.dirty() { " *" } else { "" },
                    if b.editing_locked() {
                        " [editing locked]"
                    } else {
                        ""
                    }
                )
            },
        )
    }
    pub fn update(&mut self, message: Message) -> Task<Message> {
        let task = self.update_component(message);
        if self.exit_requested {
            iced::exit()
        } else {
            task
        }
    }

    /// Handles one pane message without acquiring authority to terminate the
    /// embedding application. Hosts observe [`Self::exit_requested`] and
    /// decide whether to remove the pane or close their own process.
    pub fn update_component(&mut self, message: Message) -> Task<Message> {
        let mut task = Task::none();
        if !matches!(
            &message,
            Message::ConfirmDestination(_) | Message::CancelDestination
        ) {
            self.destination = None;
        }
        match message {
            Message::Clipboard(token, operation) => {
                self.confirming = false;
                self.notice = if self.comments.has_draft() {
                    "Unapplied comment form: source clipboard gesture refused".into()
                } else {
                    match self
                        .bridge
                        .as_mut()
                        .map(|b| b.native_clipboard(&token, operation))
                    {
                        Some(Ok(notice)) => notice,
                        Some(Err(error)) => error.to_string(),
                        None => "No source document".into(),
                    }
                };
            }
            Message::Comment(message) => {
                self.confirming = false;
                self.notice = self
                    .comments
                    .update(self.bridge.as_mut(), message)
                    .err()
                    .map_or_else(String::new, |e| e.to_string());
            }
            Message::Widget(token, action) => {
                self.confirming = false;
                self.notice = match self.bridge.as_mut().map(|b| b.widget(&token, action)) {
                    Some(Err(error)) => error.to_string(),
                    _ => String::new(),
                };
            }
            Message::Shortcut(token, Shortcut::Command(command)) => {
                self.confirming = false;
                self.notice = match self.bridge.as_mut().map(|b| b.command(&token, command)) {
                    Some(Err(error)) => error.to_string(),
                    _ => String::new(),
                };
            }
            Message::Shortcut(token, Shortcut::Quit) => {
                if self.bridge.as_ref().is_some_and(|b| b.token() != token) {
                    self.notice = "Stale view; quit not applied".into();
                } else {
                    return self.request_close(false);
                }
            }
            Message::NativeClose(_) => return self.request_close(false),
            Message::Discard(token) => {
                if self.confirming && self.bridge.as_ref().is_some_and(|b| b.token() == token) {
                    return self.request_close(true);
                }
            }
            Message::Cancel => {
                self.confirming = false;
                self.notice.clear();
            }
            Message::Exit => return self.request_close(false),
            Message::PathChanged(path) => {
                if path.len() <= 4096 {
                    self.path = path;
                } else {
                    self.notice = "Path exceeds the 4096-byte frontend limit".into();
                }
            }
            Message::QueryChanged(token, query) => {
                if self.bridge.as_ref().is_some_and(|b| b.token() == token) && query.len() <= 1024 {
                    self.query = query;
                } else {
                    self.notice = "Search input stale or exceeds 1024-byte frontend limit".into();
                }
            }
            Message::ReplacementChanged(token, replacement) => {
                if self.bridge.as_ref().is_some_and(|b| b.token() == token)
                    && replacement.len() <= 65536
                {
                    self.replacement = replacement;
                } else {
                    self.notice = "Replacement input stale or exceeds 64-KiB frontend limit".into();
                }
            }
            Message::Find(token) => {
                self.confirming = false;
                self.notice = match self.bridge.as_mut().map(|b| b.find(&token, &self.query)) {
                    Some(Err(error)) => error.to_string(),
                    _ => String::new(),
                };
            }
            Message::SelectMatches(token) => {
                self.confirming = false;
                self.notice.clear();
                if let Some(bridge) = self.bridge.as_mut() {
                    match bridge.select_matches(&token) {
                        Ok(()) => {
                            task = iced::advanced::widget::operate(
                                crate::attachment::focus_if_present(bridge.token().widget_id()),
                            )
                            .discard();
                        }
                        Err(error) => self.notice = error.to_string(),
                    }
                }
            }
            Message::Replace(token, all) => {
                self.confirming = false;
                self.notice = match self
                    .bridge
                    .as_mut()
                    .map(|b| b.replace_match(&token, &self.replacement, all))
                {
                    Some(Err(error)) => error.to_string(),
                    _ => String::new(),
                };
            }
            Message::CancelDestination => {
                self.destination = None;
            }
            Message::RequestDestination(token, save_as) => {
                self.confirming = false;
                match self
                    .bridge
                    .as_mut()
                    .map(|b| b.plan_destination(&token, PathBuf::from(&self.path), save_as))
                {
                    Some(Ok(plan)) => {
                        self.destination = Some((token, plan));
                        self.notice.clear();
                    }
                    Some(Err(error)) => self.notice = error.to_string(),
                    None => {}
                }
            }
            Message::ConfirmDestination(plan) => {
                if self
                    .destination
                    .as_ref()
                    .is_some_and(|(_, current)| *current == plan)
                {
                    let (token, original) = self.destination.take().expect("matching confirmation");
                    self.notice = match self
                        .bridge
                        .as_mut()
                        .map(|b| b.confirm_destination(&token, &original))
                    {
                        Some(Err(error)) => error.to_string(),
                        _ => String::new(),
                    };
                } else {
                    self.notice = "Stale destination confirmation; no write requested".into();
                }
            }
            Message::OpenPath => {
                if !self.path.is_empty() {
                    self.confirming = false;
                    let root = PathBuf::from(&self.path);
                    let paths = match plan_workspace_roots([root]) {
                        Ok(paths) => paths,
                        Err(error) => {
                            self.notice = error.to_string();
                            return task;
                        }
                    };
                    if let Some(bridge) = self.bridge.as_mut() {
                        let mut attached = 0usize;
                        let mut failure = None;
                        for path in paths {
                            match bridge.open_document(&bridge.token(), &path) {
                                Ok(_) => attached += 1,
                                Err(error) => {
                                    failure = Some(error);
                                    break;
                                }
                            }
                        }
                        self.notice = failure.map_or_else(
                            || {
                                self.path.clear();
                                String::new()
                            },
                            |error| {
                                format!(
                                    "root admission stopped after {attached} attachment(s): {error}"
                                )
                            },
                        );
                    } else {
                        match Bridge::open_reference(&paths) {
                            Ok(bridge) => {
                                self.bridge = Some(bridge);
                                self.notice.clear();
                                self.path.clear();
                            }
                            Err(error) => {
                                self.notice = error.to_string();
                            }
                        }
                    }
                }
            }
        }
        self.comments.refresh(self.bridge.as_mut());
        task
    }
    fn request_close(&mut self, discard: bool) -> Task<Message> {
        if !discard && self.comments.has_draft() {
            self.confirming = true;
            self.notice =
                "Unapplied comment form. Discard and close requires explicit confirmation.".into();
            return Task::none();
        }
        let Some(bridge) = self.bridge.as_mut() else {
            self.exit_requested = true;
            return Task::none();
        };
        match bridge.close(discard) {
            Ok(()) => {
                self.comments = CommentPane::default();
                self.exit_requested = true;
                Task::none()
            }
            Err(BridgeError::Editor(EditorError::Unsaved)) => {
                self.confirming = true;
                self.notice =
                    "Unsaved documents. Discard and close requires explicit confirmation.".into();
                Task::none()
            }
            Err(error) => {
                self.notice = error.to_string();
                Task::none()
            }
        }
    }

    /// Lifecycle output only; component mode never terminates the process.
    pub const fn exit_requested(&self) -> bool {
        self.exit_requested
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let close = window::close_requests().map(Message::NativeClose);
        Subscription::batch([close, self.subscription_component()])
    }

    /// Pane-local subscriptions only. Window-close ownership remains with the
    /// embedding host.
    pub fn subscription_component(&self) -> Subscription<Message> {
        let Some(bridge) = &self.bridge else {
            return Subscription::none();
        };
        event::listen_with(|event, status, _| {
            if status != event::Status::Ignored {
                return None;
            }
            let iced::Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. }) = event
            else {
                return None;
            };
            shortcut(&key, modifiers)
        })
        .with(bridge.token())
        .map(|(token, shortcut)| Message::Shortcut(token, shortcut))
    }
    pub fn view(&self) -> Element<'_, Message> {
        let Some(bridge) = &self.bridge else {
            return column![
                text(&self.notice),
                text_input("Existing local file path", &self.path)
                    .on_input(Message::PathChanged)
                    .on_submit(Message::OpenPath),
                button("Open").on_press(Message::OpenPath),
                button("Close").on_press(Message::Exit)
            ]
            .spacing(12)
            .padding(16)
            .into();
        };
        if bridge.is_closed() {
            return text("Closed — releasing window").into();
        }
        let token = bridge.token();
        let editor = document_editor(bridge);
        let toolbar = row![
            button("Save").on_press(Message::Shortcut(
                token.clone(),
                Shortcut::Command(Command::Save)
            )),
            button("Undo").on_press(Message::Shortcut(
                token.clone(),
                Shortcut::Command(Command::Undo)
            )),
            button("Redo").on_press(Message::Shortcut(
                token.clone(),
                Shortcut::Command(Command::Redo)
            )),
            button(if bridge.editing_locked() {
                "Unlock editing"
            } else {
                "Lock editing"
            })
            .on_press(Message::Shortcut(
                token.clone(),
                Shortcut::Command(Command::ToggleEditingLock)
            )),
            button("Next file").on_press(Message::Shortcut(
                token.clone(),
                Shortcut::Command(Command::NextDocument)
            )),
            button("Comments").on_press(Message::Comment(CommentMessage::Toggle(token.clone()))),
            button("Copy source")
                .on_press(Message::Clipboard(token.clone(), Some(Operation::Copy))),
            button("Cut source").on_press(Message::Clipboard(token.clone(), Some(Operation::Cut))),
            button("Paste source")
                .on_press(Message::Clipboard(token.clone(), Some(Operation::Paste))),
            button("Retry delivery").on_press_maybe(
                bridge
                    .native_clipboard_stage()
                    .filter(|s| *s != vellum_adapter_clipboard_macos::ClipboardStage::Idle)
                    .map(|_| Message::Clipboard(token.clone(), None))
            ),
            button("Quit").on_press(Message::Shortcut(token.clone(), Shortcut::Quit)),
        ]
        .spacing(8)
        .wrap();
        let views = bridge.native_view_usage();
        let clipboard_metrics = bridge.native_clipboard_usage().map_or_else(
            || "; clipboard unbound (no pasteboard access)".into(),
            |u| {
                format!(
                    "; clipboard I/O exports {} / reads {} / uncertain exports {}",
                    u.export_offers, u.read_offers, u.export_unknown
                )
            },
        );
        let unicode_metrics = bridge.native_unicode_usage().map_or_else(
            || "; Unicode legacy profile".into(),
            |u| {
                if u.overflowed {
                    "; Unicode service counters overflowed".into()
                } else {
                    format!(
                        "; Unicode service {}/{} completed/offered, {} refused",
                        u.completed, u.offered, u.refused
                    )
                }
            },
        );
        let loss = format!(
            "Native views: {}/{}; selections: {}; capture drops: {}; trace-window overwrites: {}{}{}",
            views.attached,
            views.live,
            views.selections,
            bridge.dropped_diagnostics(),
            bridge
                .diagnostic_window_overwrites()
                .map_or_else(|| "unknown".into(), |n| n.to_string()),
            clipboard_metrics,
            unicode_metrics,
        );
        let open = row![
            text_input("Existing file path: Open / Export / Save As", &self.path)
                .on_input(Message::PathChanged)
                .on_submit(Message::OpenPath),
            button("Open").on_press(Message::OpenPath),
            button("Export copy").on_press(Message::RequestDestination(token.clone(), false)),
            button("Save As").on_press(Message::RequestDestination(token.clone(), true))
        ]
        .spacing(8);
        let query_token = token.clone();
        let replacement_token = token.clone();
        let search = row![
            text_input("Literal query", &self.query)
                .on_input(move |s| Message::QueryChanged(query_token.clone(), s))
                .on_submit(Message::Find(token.clone())),
            button("Find").on_press(Message::Find(token.clone())),
            button("Select matches").on_press(Message::SelectMatches(token.clone())),
            button("Previous").on_press(Message::Shortcut(
                token.clone(),
                Shortcut::Command(Command::PreviousMatch)
            )),
            button("Next").on_press(Message::Shortcut(
                token.clone(),
                Shortcut::Command(Command::NextMatch)
            )),
        ]
        .spacing(8);
        let replace = row![
            text_input("Literal replacement", &self.replacement)
                .on_input(move |s| Message::ReplacementChanged(replacement_token.clone(), s)),
            button("Replace current").on_press(Message::Replace(token.clone(), false)),
            button("Replace all").on_press(Message::Replace(token.clone(), true)),
        ]
        .spacing(8);
        // Auxiliary forms must not consume the source's entire fill-height
        // allocation. Scroll their actual framework layout independently.
        let body: Element<'_, Message> = if self.comments.open || self.comments.has_draft() {
            row![
                container(editor).width(iced::Length::FillPortion(3)),
                scrollable(self.comments.view(bridge))
                    .width(iced::Length::FillPortion(2))
                    .height(iced::Length::Fill),
            ]
            .spacing(12)
            .height(iced::Length::Fill)
            .into()
        } else {
            editor
        };
        let source_status = if bridge.multiple_selections_active() {
            format!(
                "{} — Multi-selection: edits affect ALL targets, including clipped/unavailable overlays; Select All exits.",
                bridge.status()
            )
        } else {
            bridge.status().to_owned()
        };
        let mut layout=column![toolbar,open,search,replace,body,text(source_status),text(&self.notice),text(loss),text("Cmd-C/X/V source clipboard; Cmd-S/Z/Shift-Z; F3 next match; F4 lock; F6 next file. Private clipboard qualified; General/desktop qualification pending.")].spacing(8).padding(12);
        if let Some((_, plan)) = &self.destination {
            layout = layout
                .push(text(format!(
                    "{} will replace this existing file: {}",
                    if plan.is_save_as() {
                        "Save As"
                    } else {
                        "Copy export"
                    },
                    plan.destination()
                        .to_string_lossy()
                        .chars()
                        .flat_map(char::escape_default)
                        .collect::<String>()
                )))
                .push(
                    row![
                        button("Confirm overwrite")
                            .on_press(Message::ConfirmDestination(plan.clone())),
                        button("Cancel destination").on_press(Message::CancelDestination)
                    ]
                    .spacing(8),
                );
        }
        if self.confirming {
            layout = layout.push(
                row![
                    button("Discard and close").on_press(Message::Discard(token)),
                    button("Cancel").on_press(Message::Cancel),
                ]
                .spacing(8),
            );
        }
        layout.into()
    }
}
fn document_editor(bridge: &Bridge) -> Element<'_, Message> {
    let token = bridge.token();
    let input_token = token.clone();
    let binding_token = token.clone();
    crate::attachment::attached_source(
        bridge,
        text_editor(bridge.content())
            .id(token.widget_id())
            .font(iced::Font::MONOSPACE)
            .size(crate::attachment::SOURCE_SIZE)
            .height(iced::Length::Fill)
            .padding(crate::attachment::SOURCE_PADDING)
            .on_action(move |action| Message::Widget(input_token.clone(), action))
            .key_binding(move |press| key_binding(binding_token.clone(), press)),
    )
}
#[cfg(test)]
#[path = "input_tests.rs"]
mod input_tests;
fn shortcut(key: &Key, modifiers: Modifiers) -> Option<Shortcut> {
    match key.as_ref() {
        Key::Character("s" | "S") if modifiers.command() || modifiers.control() => {
            Some(Shortcut::Command(Command::Save))
        }
        Key::Character("z" | "Z") if modifiers.command() || modifiers.control() => {
            Some(Shortcut::Command(if modifiers.shift() {
                Command::Redo
            } else {
                Command::Undo
            }))
        }
        Key::Character("y" | "Y") if modifiers.command() || modifiers.control() => {
            Some(Shortcut::Command(Command::Redo))
        }
        Key::Character("q" | "Q") if modifiers.command() || modifiers.control() => {
            Some(Shortcut::Quit)
        }
        Key::Named(keyboard::key::Named::F3) => Some(Shortcut::Command(if modifiers.shift() {
            Command::PreviousMatch
        } else {
            Command::NextMatch
        })),
        Key::Named(keyboard::key::Named::F4) => Some(Shortcut::Command(Command::ToggleEditingLock)),
        Key::Named(keyboard::key::Named::F6) => Some(Shortcut::Command(Command::NextDocument)),
        _ => None,
    }
}
fn key_binding(
    token: ViewToken,
    press: text_editor::KeyPress,
) -> Option<text_editor::Binding<Message>> {
    if (press.modifiers.command() || press.modifiers.control())
        && !press.modifiers.shift()
        && !press.modifiers.alt()
        && matches!(press.status, text_editor::Status::Focused { .. })
    {
        let operation = match press.key.as_ref() {
            Key::Character("c" | "C") => Some(Operation::Copy),
            Key::Character("x" | "X") => Some(Operation::Cut),
            Key::Character("v" | "V") => Some(Operation::Paste),
            _ => None,
        };
        if let Some(operation) = operation {
            return Some(text_editor::Binding::Custom(Message::Clipboard(
                token,
                Some(operation),
            )));
        }
    }
    if let Some(shortcut) = shortcut(&press.key, press.modifiers) {
        return Some(text_editor::Binding::Custom(Message::Shortcut(
            token, shortcut,
        )));
    }
    match text_editor::Binding::from_key_press(press) {
        // Clipboard workflow/evidence belongs in integration. Do not silently
        // enable the widget's independent OS copy/cut/paste implementation.
        Some(
            text_editor::Binding::Copy | text_editor::Binding::Cut | text_editor::Binding::Paste,
        ) => None,
        binding => binding,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty_gui_admits_a_directory_without_a_synthetic_document() {
        use std::{
            fs,
            sync::atomic::{AtomicU64, Ordering},
        };
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let directory = loop {
            let path = std::env::temp_dir().join(format!(
                "vellum-gui-empty-open-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&path) {
                Ok(()) => break path,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("fixture directory: {error}"),
            }
        };
        let first = directory.join("a.txt");
        let second = directory.join("b.txt");
        fs::write(&first, "first").unwrap();
        fs::write(&second, "second").unwrap();

        let mut gui = Gui::open(&[]);
        assert!(gui.bridge.is_none());
        assert_eq!(gui.title(), "Vellum — open file");
        let _ = gui.update(Message::PathChanged(
            directory.to_str().expect("UTF-8 fixture").into(),
        ));
        let _ = gui.update(Message::OpenPath);
        assert_eq!(gui.bridge.as_ref().unwrap().document_count(), 2);
        assert!(gui.path.is_empty());
        assert!(gui.notice.is_empty());

        gui.bridge.as_mut().unwrap().close(true).unwrap();
        drop(gui);
        fs::remove_file(first).unwrap();
        fs::remove_file(second).unwrap();
        fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn message_diagnostics_redact_committed_and_pasted_source() {
        let token = ViewToken::test_token();
        let message = Message::Widget(
            token,
            text_editor::Action::Edit(text_editor::Edit::Paste(std::sync::Arc::new(
                "private document".into(),
            ))),
        );
        assert_eq!(format!("{message:?}"), "Widget(<redacted>)");
        assert_eq!(
            format!(
                "{:?}",
                Message::QueryChanged(ViewToken::test_token(), "private query".into())
            ),
            "QueryChanged(<redacted>)"
        );
        assert_eq!(
            format!(
                "{:?}",
                Message::ReplacementChanged(ViewToken::test_token(), "private replacement".into())
            ),
            "ReplacementChanged(<redacted>)"
        );
    }

    #[test]
    fn source_clipboard_uses_the_integration_route_never_the_widget_backend() {
        fn press(key: &str) -> text_editor::KeyPress {
            text_editor::KeyPress {
                key: Key::Character(key.into()),
                modified_key: Key::Character(key.into()),
                physical_key: keyboard::key::Physical::Unidentified(
                    keyboard::key::NativeCode::Unidentified,
                ),
                modifiers: Modifiers::COMMAND,
                text: None,
                status: text_editor::Status::Focused { is_hovered: false },
            }
        }
        for (key, expected) in [
            ("c", Operation::Copy),
            ("x", Operation::Cut),
            ("v", Operation::Paste),
        ] {
            assert!(matches!(key_binding(ViewToken::test_token(), press(key)),
                Some(text_editor::Binding::Custom(Message::Clipboard(_, Some(operation)))) if operation == expected));
            let mut unfocused = press(key);
            unfocused.status = text_editor::Status::Active;
            assert!(!matches!(
                key_binding(ViewToken::test_token(), unfocused),
                Some(text_editor::Binding::Custom(Message::Clipboard(..)))
            ));
        }
        assert!(matches!(
            key_binding(ViewToken::test_token(), press("z")),
            Some(text_editor::Binding::Custom(Message::Shortcut(
                _,
                Shortcut::Command(Command::Undo)
            )))
        ));
        assert!(matches!(
            key_binding(ViewToken::test_token(), press("s")),
            Some(text_editor::Binding::Custom(Message::Shortcut(
                _,
                Shortcut::Command(Command::Save)
            )))
        ));
        assert!(matches!(
            key_binding(ViewToken::test_token(), press("a")),
            Some(text_editor::Binding::SelectAll)
        ));
    }

    #[test]
    fn native_close_and_stale_discard_do_not_erase_new_unreviewed_edits() {
        use std::{
            fs,
            sync::atomic::{AtomicU64, Ordering},
        };
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let directory = loop {
            let path = std::env::temp_dir().join(format!(
                "vellum-gui-macos-close-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&path) {
                Ok(()) => break path,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("fixture directory: {e}"),
            }
        };
        let path = directory.join("source");
        fs::write(&path, b"A").unwrap();
        let mut gui = Gui::open(std::slice::from_ref(&path));
        let token = gui.bridge.as_ref().unwrap().token();
        let _ = gui.update(Message::Widget(
            token.clone(),
            text_editor::Action::Edit(text_editor::Edit::Insert('B')),
        ));
        let _ = gui.update(Message::NativeClose(window::Id::unique()));
        assert!(gui.confirming);
        assert!(!gui.bridge.as_ref().unwrap().is_closed());
        let _ = gui.update(Message::Widget(
            token.clone(),
            text_editor::Action::Edit(text_editor::Edit::Insert('C')),
        ));
        assert!(!gui.confirming);
        let _ = gui.update(Message::Discard(token.clone()));
        assert!(!gui.bridge.as_ref().unwrap().is_closed());
        let _ = gui.update(Message::NativeClose(window::Id::unique()));
        let _ = gui.update(Message::Cancel);
        assert!(!gui.confirming);
        let _ = gui.update(Message::NativeClose(window::Id::unique()));
        let _ = gui.update(Message::Discard(token));
        assert!(gui.bridge.as_ref().unwrap().is_closed());
        assert_eq!(fs::read(&path).unwrap(), b"A");
        drop(gui);
        fs::remove_file(path).unwrap();
        fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn component_close_emits_lifecycle_without_owning_process_exit() {
        let mut gui = Gui::open(&[]);
        let _ = gui.update_component(Message::Exit);
        assert!(gui.exit_requested());
    }

    #[test]
    fn stale_and_duplicate_destination_confirmations_cannot_overwrite_a_later_target() {
        use std::{
            fs,
            sync::atomic::{AtomicU64, Ordering},
        };
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let directory = loop {
            let p = std::env::temp_dir().join(format!(
                "vellum-gui-destination-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&p) {
                Ok(()) => break p,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("fixture: {e}"),
            }
        };
        let source = directory.join("source");
        let target = directory.join("target");
        fs::write(&source, b"A").unwrap();
        fs::write(&target, b"OLD").unwrap();
        let mut gui = Gui::open(std::slice::from_ref(&source));
        let token = gui.bridge.as_ref().unwrap().token();
        let _ = gui.update(Message::PathChanged(target.to_str().unwrap().into()));
        let _ = gui.update(Message::RequestDestination(token.clone(), true));
        let first = gui.destination.as_ref().unwrap().1.clone();
        let _ = gui.update(Message::RequestDestination(token, true));
        let second = gui.destination.as_ref().unwrap().1.clone();
        let _ = gui.update(Message::ConfirmDestination(first));
        assert_eq!(fs::read(&target).unwrap(), b"OLD");
        assert!(gui.destination.is_some());
        let _ = gui.update(Message::ConfirmDestination(second.clone()));
        assert_eq!(fs::read(&target).unwrap(), b"A");
        fs::write(&target, b"external author").unwrap();
        let _ = gui.update(Message::ConfirmDestination(second));
        assert_eq!(fs::read(&target).unwrap(), b"external author");
        assert!(gui.notice.contains("Stale"));
        gui.bridge.as_mut().unwrap().close(true).unwrap();
        drop(gui);
        fs::remove_file(source).unwrap();
        fs::remove_file(target).unwrap();
        fs::remove_dir(directory).unwrap();
    }
}
