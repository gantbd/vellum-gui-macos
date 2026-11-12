#![cfg(target_os = "macos")]
use iced::{
    Element, Length, Renderer, Size,
    advanced::{layout, text::editor::Selection, widget::Tree},
    widget::{
        text_editor,
        text_editor::{Action, Content, Cursor, Edit, Position},
    },
};

fn layout_content(content: &Content, size: Size) {
    let renderer = Renderer::new(iced::Font::MONOSPACE, iced::Pixels(16.0));
    let mut widget: Element<'_, ()> = text_editor(content)
        .font(iced::Font::MONOSPACE)
        .width(size.width)
        .height(Length::Fixed(size.height))
        .padding(0)
        .into();
    let mut tree = Tree::new(widget.as_widget());
    widget
        .as_widget_mut()
        .layout(&mut tree, &renderer, &layout::Limits::new(Size::ZERO, size));
}
fn cursor(line: usize, column: usize) -> Cursor {
    Cursor {
        position: Position { line, column },
        selection: None,
    }
}

#[test]
fn selected_matches_paint_current_native_targets_and_edit_as_one_history_step() {
    use vellum_gui_macos::{Bridge, BridgeError, Command};
    let dir = std::env::temp_dir().join(format!(
        "vellum-secondary-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&dir).unwrap();
    let path = dir.join("source");
    std::fs::write(&path, "cat\r\ncat").unwrap();
    let mut bridge = Bridge::open(
        std::slice::from_ref(&path),
        vellum_gui_macos::reference_session_config(),
    )
    .unwrap();
    bridge.find(&bridge.token(), "cat").unwrap();
    let stale = bridge.token();
    bridge.select_matches(&stale).unwrap();
    assert!(matches!(
        bridge.select_matches(&stale),
        Err(BridgeError::Stale)
    ));
    let size = Size::new(320.0, 160.0);
    assert!(bridge.secondary_geometry(size, 8).is_err());
    layout_content(bridge.content(), size);
    let primary = bridge.content().cursor();
    let geometry = bridge.secondary_geometry(size, 8).unwrap();
    assert_eq!(geometry.len(), 1);
    assert!(matches!(&geometry[0], Selection::Range(r) if r.len() == 1 && r[0].y > 0.0));
    assert_eq!(bridge.content().cursor(), primary);
    assert!(bridge.secondary_geometry(size, 0).is_err());
    bridge
        .widget(&bridge.token(), Action::Move(text_editor::Motion::Up))
        .unwrap_err();
    assert_eq!(bridge.secondary_geometry(size, 8).unwrap().len(), 1);
    bridge
        .widget(&bridge.token(), Action::Edit(Edit::Insert('X')))
        .unwrap();
    assert_eq!(bridge.content().text(), "X\nX");
    assert!(bridge.secondary_geometry(size, 8).is_err());
    layout_content(bridge.content(), size);
    assert!(matches!(
        &bridge.secondary_geometry(size, 8).unwrap()[0],
        Selection::Caret(_)
    ));
    bridge.command(&bridge.token(), Command::Save).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"X\r\nX");
    bridge.command(&bridge.token(), Command::Undo).unwrap();
    assert_eq!(bridge.content().text(), "cat\ncat");
    let usage = bridge.secondary_geometry_usage();
    assert!(usage.completed >= 3 && usage.refused >= 3);
    bridge.set_cursor(&bridge.token(), cursor(0, 0)).unwrap();
    assert!(bridge.secondary_geometry(size, 8).unwrap().is_empty());
    bridge.close(true).unwrap();
    assert!(bridge.secondary_geometry(size, 8).is_err());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn geometry_requires_actual_current_layout_and_preserves_cursor_and_source() {
    let mut content = Content::with_text("é abc\nsecond\nthird");
    let size = Size::new(320.0, 160.0);
    assert!(content.selection_geometry(size, cursor(0, 2), 8).is_none());
    layout_content(&content, size);
    let original_cursor = content.cursor();
    let original_text = content.text();
    let Selection::Caret(point) = content.selection_geometry(size, cursor(1, 2), 8).unwrap() else {
        panic!("expected caret")
    };
    assert!(point.x > 0.0 && point.y > 0.0);
    assert_eq!(content.cursor(), original_cursor);
    assert_eq!(content.text(), original_text);
    assert!(content.selection_geometry(size, cursor(0, 1), 8).is_none()); // inside UTF-8 scalar: no repair
    assert!(content.selection_geometry(size, cursor(99, 0), 8).is_none());
    assert!(content.selection_geometry(size, cursor(0, 0), 0).is_none());
    content.perform(Action::Edit(Edit::Insert('Z')));
    assert!(content.selection_geometry(size, cursor(0, 0), 8).is_none());
    let resized = Size::new(180.0, 160.0);
    layout_content(&content, resized);
    assert!(content.selection_geometry(size, cursor(0, 0), 8).is_none());
    assert!(
        content
            .selection_geometry(resized, cursor(0, 0), 8)
            .is_some()
    );
}

#[test]
fn secondary_ranges_are_bounded_and_do_not_retarget_the_primary() {
    let content = Content::with_text("first\nsecond\nthird");
    let size = Size::new(320.0, 160.0);
    layout_content(&content, size);
    let query = Cursor {
        position: Position { line: 2, column: 3 },
        selection: Some(Position { line: 0, column: 1 }),
    };
    assert!(content.selection_geometry(size, query, 2).is_none());
    let Selection::Range(regions) = content.selection_geometry(size, query, 3).unwrap() else {
        panic!("expected range")
    };
    assert_eq!(regions.len(), 3);
    assert!(regions.iter().all(|r| r.width > 0.0 && r.height > 0.0));
    assert_eq!(content.cursor(), cursor(0, 0));
}

#[test]
fn geometry_tracks_actual_wrapping_without_reflow_on_read() {
    let content = Content::with_text("a long line with several words to wrap\nnext");
    let wide = Size::new(600.0, 200.0);
    layout_content(&content, wide);
    let Selection::Caret(before) = content.selection_geometry(wide, cursor(0, 30), 8).unwrap()
    else {
        panic!("caret")
    };
    let narrow = Size::new(90.0, 200.0);
    layout_content(&content, narrow);
    let Selection::Caret(after) = content
        .selection_geometry(narrow, cursor(0, 30), 8)
        .unwrap()
    else {
        panic!("caret")
    };
    assert!(after.y > before.y);
    assert!(content.selection_geometry(wide, cursor(0, 30), 8).is_none());
}

#[test]
fn renderer_query_matches_its_primary_paint_geometry_without_mutating_cursor() {
    use iced::advanced::text::{self, editor::Editor as _};
    type TextEditor = <Renderer as text::Renderer>::Editor;
    let mut renderer = TextEditor::with_text("first second third fourth\nnext line");
    let mut highlighter = text::highlighter::PlainText;
    let size = Size::new(90.0, 200.0);
    renderer.update(
        size,
        iced::Font::MONOSPACE,
        iced::Pixels(16.0),
        text::LineHeight::default(),
        text::Wrapping::default(),
        &mut highlighter,
    );
    let initial = renderer.cursor();
    for query in [
        cursor(0, 4),
        cursor(1, 3),
        Cursor {
            position: Position { line: 1, column: 4 },
            selection: Some(Position { line: 0, column: 2 }),
        },
    ] {
        let measured = renderer.selection_for(query, 32).unwrap();
        assert_eq!(renderer.cursor(), initial);
        renderer.move_to(query);
        let painted = renderer.selection();
        match (measured, painted) {
            (Selection::Caret(a), Selection::Caret(b)) => assert_eq!(a, b),
            (Selection::Range(a), Selection::Range(b)) => assert_eq!(a, b),
            _ => panic!("query changed renderer geometry kind"),
        }
        // Restore the primary explicitly only in the oracle, not during query.
        renderer.move_to(initial);
    }
}

#[test]
fn bridge_geometry_binds_exact_source_version_and_refuses_unrepresentable_targets() {
    use vellum_gui_macos::{Bridge, BridgeError, Command, GeometryRequest};
    let mut dir = None;
    for n in 0..256 {
        let path = std::env::temp_dir().join(format!("vellum-geometry-{}-{n}", std::process::id()));
        match std::fs::create_dir(&path) {
            Ok(()) => {
                dir = Some(path);
                break;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => panic!("fixture: {e}"),
        }
    }
    let dir = dir.unwrap();
    let source = "é A\r\nsecond";
    std::fs::write(dir.join("a"), source).unwrap();
    std::fs::write(dir.join("b"), "other").unwrap();
    let mut bridge = Bridge::open(
        &[dir.join("a"), dir.join("b")],
        vellum_gui_macos::reference_session_config(),
    )
    .unwrap();
    let token = bridge.token();
    let version = bridge
        .native_selection(&token)
        .unwrap()
        .unwrap()
        .source
        .unwrap();
    let size = Size::new(320.0, 160.0);
    layout_content(bridge.content(), size);
    let original = bridge.content().cursor();
    assert!(matches!(
        bridge.rendered_geometry(
            &token,
            GeometryRequest {
                version: &version,
                anchor: 6,
                head: 9,
                text_bounds: size,
                max_regions: 8
            }
        ),
        Ok(Selection::Range(_))
    ));
    assert_eq!(bridge.content().cursor(), original);
    for byte in [1, 5] {
        // interior UTF-8 scalar and interior CRLF: never repair
        assert!(matches!(
            bridge.rendered_geometry(
                &token,
                GeometryRequest {
                    version: &version,
                    anchor: byte,
                    head: byte,
                    text_bounds: size,
                    max_regions: 8
                }
            ),
            Err(BridgeError::Position)
        ));
    }
    bridge
        .widget(&token, Action::Edit(Edit::Insert('Z')))
        .unwrap();
    assert!(matches!(
        bridge.rendered_geometry(
            &token,
            GeometryRequest {
                version: &version,
                anchor: 0,
                head: 0,
                text_bounds: size,
                max_regions: 8
            }
        ),
        Err(BridgeError::Stale)
    ));
    bridge
        .command(&bridge.token(), Command::NextDocument)
        .unwrap();
    assert!(matches!(
        bridge.rendered_geometry(
            &token,
            GeometryRequest {
                version: &version,
                anchor: 0,
                head: 0,
                text_bounds: size,
                max_regions: 8
            }
        ),
        Err(BridgeError::Stale)
    ));
    bridge.close(true).unwrap();
    let usage = bridge.geometry_usage();
    assert_eq!(
        (
            usage.offered,
            usage.completed,
            usage.refused,
            usage.emitted_regions
        ),
        (5, 1, 4, 1)
    );
    assert_eq!(std::fs::read(dir.join("a")).unwrap(), source.as_bytes());
    std::fs::remove_file(dir.join("a")).unwrap();
    std::fs::remove_file(dir.join("b")).unwrap();
    std::fs::remove_dir(dir).unwrap();
}
