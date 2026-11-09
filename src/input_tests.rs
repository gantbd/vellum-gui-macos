//! Actual pinned widget tree/layout/IME events, not synthetic Edit::Paste alone.
//! No window, OS input method, candidate geometry or screen-reader claim follows.
use super::*;
use iced::{
    Event, Point, Rectangle, Renderer, Size,
    advanced::{InputMethod, Layout, Shell, clipboard, input_method, layout, mouse, widget::Tree},
};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

#[test]
fn select_matches_returns_scoped_focus_and_actual_typing_edits_all_native_targets() {
    let fixture = Fixture::new();
    fs::write(fixture.0.join("a"), "cat\r\ncat").unwrap();
    let mut gui = fixture.gui();
    let token = gui.bridge.as_ref().unwrap().token();
    let _ = gui.update(Message::QueryChanged(token.clone(), "cat".into()));
    let _ = gui.update(Message::Find(token));
    let task = gui.update(Message::SelectMatches(gui.bridge.as_ref().unwrap().token()));
    assert!(gui.notice.is_empty(), "{}", gui.notice);
    assert_eq!(
        task.units(),
        1,
        "successful selection must return source focus work"
    );
    let mut framework = Framework::new();
    let id = gui_source_id(&gui);
    framework.focus_id(&mut gui, id);
    assert!(framework.key(&mut gui, "X", Modifiers::empty()) > 0);
    assert!(gui.notice.is_empty(), "{}", gui.notice);
    assert_eq!(gui.bridge.as_ref().unwrap().content().text(), "X\nX");
    let _ = gui.update(Message::Shortcut(
        gui.bridge.as_ref().unwrap().token(),
        Shortcut::Command(Command::Undo),
    ));
    assert_eq!(gui.bridge.as_ref().unwrap().content().text(), "cat\ncat");
    gui.bridge.as_mut().unwrap().close(true).unwrap();
}

fn gui_source_id(gui: &Gui) -> iced::advanced::widget::Id {
    gui.bridge.as_ref().unwrap().token().widget_id()
}

#[test]
fn late_source_focus_neither_targets_nor_blurs_a_replacement_attachment() {
    let fixture = Fixture::new();
    let mut gui = fixture.gui();
    let stale = gui_source_id(&gui);
    let _ = gui.update(Message::Shortcut(
        gui.bridge.as_ref().unwrap().token(),
        Shortcut::Command(Command::NextDocument),
    ));
    let current = gui_source_id(&gui);
    assert_ne!(stale, current);
    let mut framework = Framework::new();
    framework.focus_id(&mut gui, current);
    framework.focus_id(&mut gui, stale);
    assert!(
        framework.key(&mut gui, "X", Modifiers::empty()) > 0,
        "stale focus must not blur the current widget"
    );
    assert_eq!(gui.bridge.as_ref().unwrap().content().text(), "XB\n");
    gui.bridge.as_mut().unwrap().close(true).unwrap();
}

#[test]
fn refused_select_matches_does_not_issue_focus_work() {
    let fixture = Fixture::new();
    let mut gui = fixture.gui();
    let task = gui.update(Message::SelectMatches(gui.bridge.as_ref().unwrap().token()));
    assert!(!gui.notice.is_empty());
    assert_eq!(task.units(), 0);
    gui.bridge.as_mut().unwrap().close(true).unwrap();
}

#[test]
fn actual_source_draw_uses_secondary_renderer_geometry_without_editing() {
    let fixture = Fixture::new();
    fs::write(fixture.0.join("a"), "cat\r\ncat").unwrap();
    let mut gui = fixture.gui();
    let token = gui.bridge.as_ref().unwrap().token();
    let _ = gui.update(Message::QueryChanged(token.clone(), "cat".into()));
    let _ = gui.update(Message::Find(token));
    let token = gui.bridge.as_ref().unwrap().token();
    let _ = gui.update(Message::SelectMatches(token));
    assert!(gui.notice.is_empty(), "{}", gui.notice);
    let bridge = gui.bridge.as_ref().unwrap();
    let primary = bridge.content().cursor();
    let mut renderer = Renderer::new(iced::Font::MONOSPACE, iced::Pixels(16.0));
    {
        let mut view = document_editor(bridge);
        let mut tree = Tree::new(view.as_widget());
        let node = view.as_widget_mut().layout(
            &mut tree,
            &renderer,
            &layout::Limits::new(Size::ZERO, Size::new(500.0, 240.0)),
        );
        view.as_widget().draw(
            &tree,
            &mut renderer,
            &iced::Theme::Dark,
            &iced::advanced::renderer::Style {
                text_color: iced::Color::WHITE,
            },
            Layout::new(&node),
            mouse::Cursor::Unavailable,
            &node.bounds(),
        );
    }
    let usage = bridge.secondary_geometry_usage();
    assert_eq!(usage.completed, 1);
    assert_eq!(usage.emitted_regions, 1);
    assert_eq!(usage.refused, 0);
    assert_eq!(bridge.content().cursor(), primary);
    assert_eq!(bridge.content().text(), "cat\ncat");
    assert!(!bridge.dirty());
    gui.bridge.as_mut().unwrap().close(true).unwrap();
}

#[test]
fn actual_source_widget_named_keys_route_unicode18_motion_and_deletion_to_engine() {
    let fixture = Fixture::new();
    fs::write(fixture.0.join("a"), "\u{94d}कX").unwrap();
    let mut gui = fixture.gui();
    assert!(gui.bridge.as_ref().unwrap().native_unicode_enabled());
    let mut framework = Framework::new();
    framework.focus(&mut gui);
    assert!(framework.named(&mut gui, keyboard::key::Named::Home) > 0);
    assert!(framework.named(&mut gui, keyboard::key::Named::ArrowRight) > 0);
    assert_eq!(
        gui.bridge
            .as_ref()
            .unwrap()
            .content()
            .cursor()
            .position
            .column,
        6
    );
    assert!(framework.named(&mut gui, keyboard::key::Named::Backspace) > 0);
    assert!(gui.notice.is_empty(), "{}", gui.notice);
    assert_eq!(gui.bridge.as_ref().unwrap().content().text(), "X");
    gui.bridge.as_mut().unwrap().close(true).unwrap();
    assert_eq!(
        gui.bridge
            .as_ref()
            .unwrap()
            .native_boundary_usage()
            .retained_bytes,
        0
    );
}

#[test]
fn full_window_layout_keeps_source_and_comment_draft_in_separate_usable_viewports() {
    let fixture = Fixture::new();
    let mut gui = fixture.gui();
    let token = gui.bridge.as_ref().unwrap().token();
    let _ = gui.update(Message::Comment(CommentMessage::Begin(
        token,
        vellum_model::CommentPointProfile::Content,
    )));
    assert!(gui.notice.is_empty(), "{}", gui.notice);
    let renderer = Renderer::new(iced::Font::MONOSPACE, iced::Pixels(16.0));
    {
        let mut view = gui.view();
        let mut tree = Tree::new(view.as_widget());
        let node = view.as_widget_mut().layout(
            &mut tree,
            &renderer,
            &layout::Limits::new(Size::ZERO, Size::new(960.0, 720.0)),
        );
        let body = node.children()[4].bounds();
        assert!(body.height >= 160.0, "source viewport collapsed: {body:?}");
        assert!(
            node.children().len() == 9,
            "comments must share the content viewport, not be appended below it"
        );
        let panels = node.children()[4].children();
        assert_eq!(panels.len(), 2);
        assert!(panels[0].bounds().width >= 320.0);
        assert!(panels[1].bounds().width >= 240.0);
        assert!(panels.iter().all(|p| p.bounds().height >= 160.0));
        assert!(node.bounds().height <= 720.0);
    }
    assert!(gui.comments.has_draft());
    assert!(!gui.bridge.as_ref().unwrap().dirty());
    gui.bridge.as_mut().unwrap().close(true).unwrap();
}

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        loop {
            let path = std::env::temp_dir().join(format!(
                "vellum-ime-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&path) {
                Ok(()) => {
                    fs::write(path.join("a"), b"A\r\n").unwrap();
                    fs::write(path.join("b"), b"B\r\n").unwrap();
                    return Self(path);
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("fixture: {e}"),
            }
        }
    }
    fn gui(&self) -> Gui {
        Gui::open(&[self.0.join("a"), self.0.join("b")])
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_file(self.0.join("a")).unwrap();
        fs::remove_file(self.0.join("b")).unwrap();
        fs::remove_dir(&self.0).unwrap();
    }
}
struct Framework {
    renderer: Renderer,
    tree: Option<Tree>,
    cursor: mouse::Cursor,
    comment: bool,
}
impl Framework {
    fn focus_id(&mut self, gui: &mut Gui, id: iced::advanced::widget::Id) {
        use iced::advanced::widget::{Operation, operation::Outcome};
        let mut widget = document_editor(gui.bridge.as_ref().unwrap());
        let tree = self
            .tree
            .get_or_insert_with(|| Tree::new(widget.as_widget()));
        tree.diff(widget.as_widget());
        let node = widget.as_widget_mut().layout(
            tree,
            &self.renderer,
            &layout::Limits::new(Size::ZERO, Size::new(640.0, 480.0)),
        );
        let mut operation: Box<dyn Operation> = Box::new(crate::attachment::focus_if_present(id));
        loop {
            widget.as_widget_mut().operate(
                tree,
                Layout::new(&node),
                &self.renderer,
                operation.as_mut(),
            );
            match operation.finish() {
                Outcome::Chain(next) => operation = next,
                _ => break,
            }
        }
    }
    fn new() -> Self {
        Self {
            renderer: Renderer::new(iced::Font::MONOSPACE, iced::Pixels(16.0)),
            tree: None,
            cursor: mouse::Cursor::Available(Point::new(13.0, 13.0)),
            comment: false,
        }
    }
    fn event(&mut self, gui: &mut Gui, event: Event) -> (usize, InputMethod) {
        let mut messages = Vec::new();
        let method;
        {
            let mut widget = if self.comment {
                gui.comments.form_editor()
            } else {
                document_editor(gui.bridge.as_ref().unwrap())
            };
            let tree = self
                .tree
                .get_or_insert_with(|| Tree::new(widget.as_widget()));
            tree.diff(widget.as_widget());
            let bounds = Rectangle::new(Point::ORIGIN, Size::new(640.0, 480.0));
            let node = widget.as_widget_mut().layout(
                tree,
                &self.renderer,
                &layout::Limits::new(Size::ZERO, bounds.size()),
            );
            let mut shell = Shell::new(&mut messages);
            widget.as_widget_mut().update(
                tree,
                &event,
                Layout::new(&node),
                self.cursor,
                &self.renderer,
                &mut clipboard::Null,
                &mut shell,
                &bounds,
            );
            method = shell.input_method().clone();
        }
        let count = messages.len();
        for message in messages {
            let _ = gui.update(message);
        }
        (count, method)
    }
    fn focus(&mut self, gui: &mut Gui) {
        self.event(
            gui,
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
        );
        self.event(
            gui,
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
        );
    }
    fn key(&mut self, gui: &mut Gui, text: &str, modifiers: Modifiers) -> usize {
        self.event(
            gui,
            Event::Keyboard(keyboard::Event::KeyPressed {
                key: Key::Character(text.into()),
                modified_key: Key::Character(text.into()),
                physical_key: keyboard::key::Physical::Unidentified(
                    keyboard::key::NativeCode::Unidentified,
                ),
                location: keyboard::Location::Standard,
                modifiers,
                text: Some(text.into()),
                repeat: false,
            }),
        )
        .0
    }
    fn named(&mut self, gui: &mut Gui, name: keyboard::key::Named) -> usize {
        let key = Key::Named(name);
        self.event(
            gui,
            Event::Keyboard(keyboard::Event::KeyPressed {
                key: key.clone(),
                modified_key: key,
                physical_key: keyboard::key::Physical::Unidentified(
                    keyboard::key::NativeCode::Unidentified,
                ),
                location: keyboard::Location::Standard,
                modifiers: Modifiers::empty(),
                text: None,
                repeat: false,
            }),
        )
        .0
    }
    fn redraw(&mut self, gui: &mut Gui) -> InputMethod {
        self.event(
            gui,
            Event::Window(window::Event::RedrawRequested(iced::time::Instant::now())),
        )
        .1
    }
    fn preedit(&mut self, gui: &mut Gui) {
        assert_eq!(
            self.event(gui, Event::InputMethod(input_method::Event::Opened))
                .0,
            0
        );
        assert_eq!(
            self.event(
                gui,
                Event::InputMethod(input_method::Event::Preedit("かな".into(), Some(0..6)))
            )
            .0,
            0
        );
        assert!(
            matches!(self.redraw(gui), InputMethod::Enabled { preedit: Some(p), .. } if p.content == "かな")
        );
    }
}
fn command(gui: &mut Gui, command: Command) {
    let token = gui.bridge.as_ref().unwrap().token();
    let _ = gui.update(Message::Shortcut(token, Shortcut::Command(command)));
    assert!(gui.notice.is_empty(), "{}", gui.notice);
}

#[test]
fn comment_widget_preedit_commit_history_and_apply_remain_outside_source_editing() {
    let fixture = Fixture::new();
    let mut gui = fixture.gui();
    let source = gui.bridge.as_ref().unwrap().token();
    let _ = gui.update(Message::Comment(CommentMessage::Begin(
        source,
        vellum_model::CommentPointProfile::Content,
    )));
    assert!(gui.notice.is_empty(), "{}", gui.notice);
    let mut framework = Framework::new();
    framework.comment = true;
    framework.focus(&mut gui);
    framework.preedit(&mut gui);
    assert_eq!(gui.comments.form_raw(), "");
    assert!(!gui.bridge.as_ref().unwrap().dirty());
    assert_eq!(
        framework
            .event(
                &mut gui,
                Event::InputMethod(input_method::Event::Commit("かな\r\n".into()))
            )
            .0,
        1
    );
    assert_eq!(gui.comments.form_raw(), "かな\r\n");
    assert!(gui.bridge.as_ref().unwrap().cached_comments().is_empty());
    assert!(!gui.bridge.as_ref().unwrap().dirty());
    framework.preedit(&mut gui);
    let old = gui.comments.form_token();
    assert_eq!(framework.key(&mut gui, "z", Modifiers::COMMAND), 1);
    assert_ne!(gui.comments.form_token(), old);
    assert!(!matches!(
        framework.redraw(&mut gui),
        InputMethod::Enabled {
            preedit: Some(_),
            ..
        }
    ));
    assert_eq!(
        framework
            .event(
                &mut gui,
                Event::InputMethod(input_method::Event::Commit("late".into()))
            )
            .0,
        0
    );
    assert_eq!(gui.comments.form_raw(), "");
    assert_eq!(
        framework.key(&mut gui, "z", Modifiers::COMMAND | Modifiers::SHIFT),
        1
    );
    let raw = gui.comments.form_raw().to_owned();
    for key in ["c", "x", "v"] {
        assert_eq!(framework.key(&mut gui, key, Modifiers::COMMAND), 0);
        assert_eq!(gui.comments.form_raw(), raw);
    }
    assert_eq!(framework.key(&mut gui, "s", Modifiers::COMMAND), 1);
    assert!(gui.notice.is_empty(), "{}", gui.notice);
    assert_eq!(
        gui.bridge.as_ref().unwrap().cached_comments()[0].body,
        "かな\r\n"
    );
    assert_eq!(gui.bridge.as_ref().unwrap().content().text(), "A\n");
    assert_eq!(fs::read(fixture.0.join("a")).unwrap(), b"A\r\n");
    gui.bridge.as_mut().unwrap().close(true).unwrap();
}

#[test]
fn framework_preedit_is_inert_and_only_committed_text_reaches_native_history_and_save() {
    let fixture = Fixture::new();
    let mut gui = fixture.gui();
    let mut framework = Framework::new();
    framework.focus(&mut gui);
    let token = gui.bridge.as_ref().unwrap().token();
    let before = gui
        .bridge
        .as_mut()
        .unwrap()
        .proposal(&token, &text_editor::Edit::Insert('!'))
        .unwrap()
        .version;
    framework.preedit(&mut gui);
    let bridge = gui.bridge.as_mut().unwrap();
    assert!(!bridge.dirty());
    assert_eq!(bridge.content().text(), "A\n");
    assert_eq!(
        bridge
            .proposal(&token, &text_editor::Edit::Insert('!'))
            .unwrap()
            .version,
        before
    );
    assert_eq!(
        framework
            .event(
                &mut gui,
                Event::InputMethod(input_method::Event::Commit("漢字".into()))
            )
            .0,
        1
    );
    assert!(gui.notice.is_empty(), "{}", gui.notice);
    assert_eq!(gui.bridge.as_ref().unwrap().content().text(), "漢字A\n");
    command(&mut gui, Command::Save);
    assert_eq!(
        fs::read(fixture.0.join("a")).unwrap(),
        "漢字A\r\n".as_bytes()
    );
    command(&mut gui, Command::Undo);
    assert_eq!(gui.bridge.as_ref().unwrap().content().text(), "A\n");
    gui.bridge.as_mut().unwrap().close(true).unwrap();
}

#[test]
fn attachment_switch_and_aba_drop_framework_focus_and_preedit_before_late_commit() {
    let fixture = Fixture::new();
    let mut gui = fixture.gui();
    let mut framework = Framework::new();
    framework.focus(&mut gui);
    framework.preedit(&mut gui);
    for expected in ["B\n", "A\n"] {
        command(&mut gui, Command::NextDocument);
        assert_eq!(framework.redraw(&mut gui), InputMethod::Disabled);
        assert_eq!(
            framework
                .event(
                    &mut gui,
                    Event::InputMethod(input_method::Event::Commit("LATE".into()))
                )
                .0,
            0
        );
        assert_eq!(gui.bridge.as_ref().unwrap().content().text(), expected);
        assert!(!gui.bridge.as_ref().unwrap().dirty());
    }
    framework.focus(&mut gui);
    framework.preedit(&mut gui);
    assert_eq!(
        framework
            .event(&mut gui, Event::InputMethod(input_method::Event::Closed))
            .0,
        0
    );
    assert!(matches!(
        framework.redraw(&mut gui),
        InputMethod::Enabled { preedit: None, .. }
    ));
    assert_eq!(fs::read(fixture.0.join("a")).unwrap(), b"A\r\n");
    assert_eq!(fs::read(fixture.0.join("b")).unwrap(), b"B\r\n");
    gui.bridge.as_mut().unwrap().close(false).unwrap();
}

#[test]
fn history_and_policy_changes_spend_widget_composition_but_ordinary_typing_keeps_focus() {
    let fixture = Fixture::new();
    let mut gui = fixture.gui();
    let mut framework = Framework::new();
    framework.focus(&mut gui);
    for text in ["x", "y"] {
        assert_eq!(
            framework
                .event(
                    &mut gui,
                    Event::InputMethod(input_method::Event::Commit(text.into()))
                )
                .0,
            1
        );
        assert!(gui.notice.is_empty());
    }
    assert_eq!(gui.bridge.as_ref().unwrap().content().text(), "xyA\n");
    for action in [Command::Undo, Command::Redo, Command::ToggleEditingLock] {
        framework.preedit(&mut gui);
        command(&mut gui, action);
        assert_eq!(framework.redraw(&mut gui), InputMethod::Disabled);
        assert_eq!(
            framework
                .event(
                    &mut gui,
                    Event::InputMethod(input_method::Event::Commit("LATE".into()))
                )
                .0,
            0
        );
        framework.focus(&mut gui);
    }
    let original = gui.bridge.as_ref().unwrap().content().text();
    assert_eq!(
        framework
            .event(
                &mut gui,
                Event::InputMethod(input_method::Event::Commit("DENIED".into()))
            )
            .0,
        1
    );
    assert!(!gui.notice.is_empty());
    assert_eq!(gui.bridge.as_ref().unwrap().content().text(), original);
    gui.bridge.as_mut().unwrap().close(true).unwrap();
}

#[test]
fn keyboard_history_cancels_old_composition_then_typing_resumes_without_stealing_other_focus() {
    let fixture = Fixture::new();
    let mut gui = fixture.gui();
    let mut framework = Framework::new();
    framework.focus(&mut gui);
    assert_eq!(framework.key(&mut gui, "x", Modifiers::empty()), 1);
    framework.preedit(&mut gui);
    assert_eq!(framework.key(&mut gui, "z", Modifiers::COMMAND), 1);
    assert_eq!(gui.bridge.as_ref().unwrap().content().text(), "A\n");
    assert_eq!(
        framework
            .event(
                &mut gui,
                Event::InputMethod(input_method::Event::Commit("LATE".into()))
            )
            .0,
        0
    );
    assert_eq!(framework.key(&mut gui, "q", Modifiers::empty()), 1);
    assert!(gui.notice.is_empty());
    // Native logical views map the caret through Undo; the removed insertion
    // leaves it before A, rather than clamping the obsolete byte offset after A.
    assert_eq!(gui.bridge.as_ref().unwrap().content().text(), "qA\n");
    assert_eq!(framework.key(&mut gui, "z", Modifiers::COMMAND), 1);
    assert_eq!(gui.bridge.as_ref().unwrap().content().text(), "A\n");
    // A click elsewhere cancels deferred editor refocus. The query/path field
    // must not have its next keyboard event stolen by the source widget.
    framework.cursor = mouse::Cursor::Available(Point::new(700.0, 700.0));
    framework.focus(&mut gui);
    assert_eq!(framework.key(&mut gui, "q", Modifiers::empty()), 0);
    assert_eq!(gui.bridge.as_ref().unwrap().content().text(), "A\n");
    gui.bridge.as_mut().unwrap().close(false).unwrap();
}

#[test]
fn window_blur_cancels_preedit_and_refuses_off_window_commit_without_publishing_source() {
    let fixture = Fixture::new();
    let mut gui = fixture.gui();
    let mut framework = Framework::new();
    framework.focus(&mut gui);
    framework.preedit(&mut gui);
    framework.event(&mut gui, Event::Window(window::Event::Unfocused));
    for input in [
        input_method::Event::Opened,
        input_method::Event::Preedit("LATE".into(), None),
        input_method::Event::Commit("LATE".into()),
    ] {
        assert_eq!(framework.event(&mut gui, Event::InputMethod(input)).0, 0);
    }
    assert_eq!(gui.bridge.as_ref().unwrap().content().text(), "A\n");
    assert!(!gui.bridge.as_ref().unwrap().dirty());
    framework.event(&mut gui, Event::Window(window::Event::Focused));
    assert!(matches!(
        framework.redraw(&mut gui),
        InputMethod::Enabled { preedit: None, .. }
    ));
    assert_eq!(framework.key(&mut gui, "x", Modifiers::empty()), 1);
    assert_eq!(gui.bridge.as_ref().unwrap().content().text(), "xA\n");
    gui.bridge.as_mut().unwrap().close(true).unwrap();
}
