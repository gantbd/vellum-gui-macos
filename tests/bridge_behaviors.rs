#![cfg(target_os = "macos")]
use iced::widget::text_editor::{Action, Cursor, Edit, Motion, Position};
use std::{
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
use vellum_editor::reference_profile;
use vellum_iced::{Bridge, Command};
static NEXT: AtomicU64 = AtomicU64::new(0);
#[path = "bridge_behaviors/clipboard.rs"]
mod clipboard;
#[path = "bridge_behaviors/unicode.rs"]
mod unicode;
struct Fixture(PathBuf);
impl Fixture {
    fn new(bytes: &[u8]) -> Self {
        loop {
            let path = std::env::temp_dir().join(format!(
                "vellum-iced-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&path) {
                Ok(()) => {
                    fs::write(path.join("source"), bytes).unwrap();
                    return Self(path);
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("fixture directory: {e}"),
            }
        }
    }
    fn path(&self) -> PathBuf {
        self.0.join("source")
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_file(self.path()).unwrap();
        fs::remove_dir(&self.0).unwrap();
    }
}
fn send(bridge: &mut Bridge, action: Action) {
    let token = bridge.token();
    bridge.widget(&token, action).unwrap();
}
fn command(bridge: &mut Bridge, cmd: Command) {
    let token = bridge.token();
    bridge.command(&token, cmd).unwrap();
}

#[test]
fn native_view_and_widget_selection_agree_through_indent_history_and_cleanup() {
    for capacity in [0, 1, 256] {
        let f = Fixture::new(b"A\r\nB");
        let mut profile = reference_profile();
        profile.capture_capacity = capacity;
        let mut bridge = Bridge::open(&[f.path()], profile).unwrap();
        bridge
            .set_cursor(
                &bridge.token(),
                Cursor {
                    position: Position { line: 1, column: 1 },
                    selection: Some(Position { line: 0, column: 0 }),
                },
            )
            .unwrap();
        send(&mut bridge, Action::Edit(Edit::Indent));
        let selected = bridge.native_selection(&bridge.token()).unwrap().unwrap();
        let primary = selected.selections.selections()[0];
        // Native range mapping includes inserted prefixes, instead of the old
        // widget-only hint that skipped the first inserted prefix.
        assert_eq!((primary.anchor.0, primary.head.0), (0, 12));
        assert_eq!(
            bridge.content().cursor().selection,
            Some(Position { line: 0, column: 0 })
        );
        command(&mut bridge, Command::Undo);
        assert_eq!(bridge.content().text(), "A\nB");
        assert_eq!(
            bridge.content().cursor().selection,
            Some(Position { line: 0, column: 0 })
        );
        let selected = bridge.native_selection(&bridge.token()).unwrap().unwrap();
        assert_eq!(selected.selections.selections()[0].head.0, 4);
        bridge.close(false).unwrap();
        assert_eq!(bridge.native_view_usage().live, 0);
    }
}

#[test]
fn joined_cluster_keeps_native_and_widget_byte_projection_coherent() {
    let f = Fixture::new("\u{301}X".as_bytes());
    let mut bridge = Bridge::open(&[f.path()], reference_profile()).unwrap();
    send(&mut bridge, Action::Edit(Edit::Insert('e')));
    assert_eq!(bridge.content().text(), "e\u{301}X");
    assert_eq!(bridge.content().cursor().position.column, 3);
    assert_eq!(
        bridge
            .native_selection(&bridge.token())
            .unwrap()
            .unwrap()
            .selections
            .selections()[0]
            .head
            .0,
        3
    );
    send(&mut bridge, Action::Edit(Edit::Backspace));
    assert_eq!(bridge.content().text(), "X");
    bridge.close(true).unwrap();
}

#[test]
fn line_intent_uses_native_prefix_columns_not_a_widget_replacement_guess() {
    let f = Fixture::new(b" \tprose\r\n\t\tcode\n");
    let mut bridge = Bridge::open(&[f.path()], reference_profile()).unwrap();
    send(&mut bridge, Action::SelectAll);
    assert!(matches!(
        bridge.proposal(&bridge.token(), &Edit::Unindent),
        Err(vellum_iced::BridgeError::NativeLineIntent)
    ));
    send(&mut bridge, Action::Edit(Edit::Unindent));
    assert_eq!(bridge.content().text(), "prose\n    code\n");
    command(&mut bridge, Command::Undo);
    assert_eq!(bridge.content().text(), " \tprose\n\t\tcode\n");
    command(&mut bridge, Command::ToggleEditingLock);
    let before = bridge.content().text();
    assert!(
        bridge
            .widget(&bridge.token(), Action::Edit(Edit::Indent))
            .is_err()
    );
    assert_eq!(bridge.content().text(), before);
    command(&mut bridge, Command::ToggleEditingLock);
    bridge.close(false).unwrap();
}

#[test]
fn widget_edits_preserve_bom_mixed_endings_and_engine_history_across_capture_modes() {
    for capacity in [0, 1, 256] {
        let f = Fixture::new("\u{feff}A\r\nDeutsch\n```mermaid\rgraph LR\r```".as_bytes());
        let mut config = reference_profile();
        config.capture_capacity = capacity;
        let mut bridge = Bridge::open(&[f.path()], config).unwrap();
        assert_eq!(
            bridge.content().text(),
            "A\nDeutsch\n```mermaid\ngraph LR\n```"
        );
        send(&mut bridge, Action::Move(Motion::Right));
        send(&mut bridge, Action::Edit(Edit::Insert('é')));
        assert!(bridge.dirty());
        command(&mut bridge, Command::Undo);
        assert!(!bridge.dirty());
        command(&mut bridge, Command::Redo);
        command(&mut bridge, Command::Save);
        assert!(!bridge.dirty());
        assert_eq!(
            fs::read(f.path()).unwrap(),
            "\u{feff}Aé\r\nDeutsch\n```mermaid\rgraph LR\r```".as_bytes()
        );
        bridge.close(false).unwrap();
    }
}

#[test]
fn repeated_text_uses_original_cursor_target_not_a_guessed_diff_location() {
    let f = Fixture::new(b"aaaa");
    let mut bridge = Bridge::open(&[f.path()], reference_profile()).unwrap();
    let token = bridge.token();
    let proposal = bridge.proposal(&token, &Edit::Insert('a')).unwrap();
    assert_eq!((proposal.edits[0].start, proposal.edits[0].end), (0, 0));
    send(&mut bridge, Action::Edit(Edit::Insert('a')));
    assert_eq!(bridge.content().cursor().position.column, 1);
    send(&mut bridge, Action::Edit(Edit::Delete));
    assert_eq!(bridge.content().text(), "aaaa");
    command(&mut bridge, Command::Undo);
    assert_eq!(bridge.content().text(), "aaaaa");
    bridge.close(true).unwrap();
}

#[test]
fn selected_word_and_multiline_paste_have_exact_original_ranges() {
    let f = Fixture::new("same same\r\nB".as_bytes());
    let mut bridge = Bridge::open(&[f.path()], reference_profile()).unwrap();
    for _ in 0..6 {
        send(&mut bridge, Action::Move(Motion::Right));
    }
    send(&mut bridge, Action::SelectWord);
    let proposal = bridge
        .proposal(&bridge.token(), &Edit::Insert('X'))
        .unwrap();
    assert_eq!((proposal.edits[0].start, proposal.edits[0].end), (5, 9));
    send(&mut bridge, Action::Edit(Edit::Insert('X')));
    assert_eq!(bridge.content().text(), "same X\nB");
    send(&mut bridge, Action::SelectAll);
    send(
        &mut bridge,
        Action::Edit(Edit::Paste(Arc::new("ü\r\n界\n".into()))),
    );
    assert_eq!(bridge.content().text(), "ü\n界\n");
    command(&mut bridge, Command::Save);
    assert_eq!(fs::read(f.path()).unwrap(), "ü\r\n界\n".as_bytes());
    bridge.close(true).unwrap();
}

#[test]
fn generated_newline_and_indent_preserve_unedited_delimiters() {
    let f = Fixture::new(b"A\r\n B\rC\n");
    let mut bridge = Bridge::open(&[f.path()], reference_profile()).unwrap();
    send(&mut bridge, Action::Move(Motion::Right));
    send(&mut bridge, Action::Edit(Edit::Enter));
    assert_eq!(bridge.content().text(), "A\n\n B\nC\n");
    send(&mut bridge, Action::SelectAll);
    send(&mut bridge, Action::Edit(Edit::Indent));
    assert_eq!(bridge.content().text(), "    A\n    \n     B\n    C\n");
    send(&mut bridge, Action::Edit(Edit::Unindent));
    assert_eq!(bridge.content().text(), "A\n\n B\nC\n");
    command(&mut bridge, Command::Save);
    assert_eq!(fs::read(f.path()).unwrap(), b"A\n\r\n B\rC\n");
    bridge.close(true).unwrap();
}

#[test]
fn late_widget_events_cannot_cross_tabs_even_after_an_aba_switch() {
    let a = Fixture::new(b"A");
    let b = Fixture::new(b"B");
    let mut bridge = Bridge::open(&[a.path(), b.path()], reference_profile()).unwrap();
    let old = bridge.token();
    command(&mut bridge, Command::NextDocument);
    assert!(
        bridge
            .widget(&old, Action::Edit(Edit::Insert('X')))
            .is_err()
    );
    command(&mut bridge, Command::NextDocument);
    assert!(
        bridge
            .widget(&old, Action::Edit(Edit::Insert('X')))
            .is_err()
    );
    assert_eq!(bridge.content().text(), "A");
    bridge.close(false).unwrap();
}

#[test]
fn foreign_and_invalid_positions_refuse_without_rounding_or_source_changes() {
    let f = Fixture::new("é".as_bytes());
    let g = Fixture::new(b"A");
    let mut one = Bridge::open(&[f.path()], reference_profile()).unwrap();
    let two = Bridge::open(&[g.path()], reference_profile()).unwrap();
    assert!(
        one.widget(&two.token(), Action::Edit(Edit::Insert('X')))
            .is_err()
    );
    // Even a malformed trusted-native cursor cannot become a rounded edit target.
    let malformed = Cursor {
        position: Position { line: 0, column: 1 },
        selection: None,
    };
    assert!(one.set_cursor(&one.token(), malformed).is_err());
    assert_eq!(one.content().text(), "é");
    one.close(false).unwrap();
}

#[test]
fn ordered_input_keeps_its_attachment_but_large_pastes_refuse_before_projection() {
    let f = Fixture::new(b"A");
    let mut bridge = Bridge::open(&[f.path()], reference_profile()).unwrap();
    let token = bridge.token();
    bridge
        .widget(&token, Action::Edit(Edit::Insert('B')))
        .unwrap();
    bridge
        .widget(&token, Action::Edit(Edit::Insert('C')))
        .unwrap();
    assert_eq!(bridge.content().text(), "BCA");
    assert!(
        bridge
            .widget(
                &token,
                Action::Edit(Edit::Paste(Arc::new("x".repeat(65537))))
            )
            .is_err()
    );
    assert_eq!(bridge.content().text(), "BCA");
    assert_eq!(fs::read(f.path()).unwrap(), b"A");
    bridge.close(true).unwrap();
}

#[test]
fn all_canonical_delimiter_pairs_survive_visual_projection_and_generated_enter() {
    for first in ["\n", "\r\n", "\r", "\n\r"] {
        for second in ["\n", "\r\n", "\r", "\n\r"] {
            let original = format!("\u{feff}é{first}界{second}Z");
            let f = Fixture::new(original.as_bytes());
            let mut bridge = Bridge::open(&[f.path()], reference_profile()).unwrap();
            let token = bridge.token();
            bridge
                .set_cursor(
                    &token,
                    Cursor {
                        position: Position { line: 0, column: 2 },
                        selection: None,
                    },
                )
                .unwrap();
            bridge.widget(&token, Action::Edit(Edit::Enter)).unwrap();
            command(&mut bridge, Command::Save);
            assert_eq!(
                fs::read(f.path()).unwrap(),
                format!("\u{feff}é\n{first}界{second}Z").as_bytes()
            );
            bridge.close(false).unwrap();
        }
    }
}

#[test]
fn opening_another_file_spends_old_input_without_replacing_unsaved_source() {
    let a = Fixture::new(b"A");
    let b = Fixture::new(b"B");
    let mut bridge = Bridge::open(&[a.path()], reference_profile()).unwrap();
    let old = bridge.token();
    bridge
        .widget(&old, Action::Edit(Edit::Insert('x')))
        .unwrap();
    assert!(bridge.open_document(&old, &b.path()).unwrap());
    assert_eq!(bridge.document_count(), 2);
    assert_eq!(bridge.content().text(), "B");
    assert!(
        bridge
            .widget(&old, Action::Edit(Edit::Insert('!')))
            .is_err()
    );
    let current = bridge.token();
    assert!(!bridge.open_document(&current, &a.path()).unwrap());
    assert_eq!(bridge.content().text(), "xA");
    assert!(
        bridge
            .widget(&old, Action::Edit(Edit::Insert('!')))
            .is_err()
    );
    assert!(bridge.close(false).is_err());
    bridge.close(true).unwrap();
    assert_eq!(fs::read(a.path()).unwrap(), b"A");
}

#[test]
fn editing_lock_blocks_mutation_undo_and_save_but_not_navigation() {
    let f = Fixture::new(b"A");
    let mut bridge = Bridge::open(&[f.path()], reference_profile()).unwrap();
    let old = bridge.token();
    bridge
        .widget(&old, Action::Edit(Edit::Insert('x')))
        .unwrap();
    command(&mut bridge, Command::ToggleEditingLock);
    assert!(bridge.editing_locked());
    assert!(
        bridge
            .widget(&old, Action::Edit(Edit::Insert('!')))
            .is_err()
    );
    let current = bridge.token();
    assert!(
        bridge
            .widget(&current, Action::Edit(Edit::Insert('!')))
            .is_err()
    );
    assert!(bridge.command(&current, Command::Undo).is_err());
    bridge
        .widget(&current, Action::Move(Motion::Right))
        .unwrap();
    assert!(bridge.command(&current, Command::Save).is_err());
    assert_eq!(fs::read(f.path()).unwrap(), b"A");
    command(&mut bridge, Command::ToggleEditingLock);
    assert!(!bridge.editing_locked());
    command(&mut bridge, Command::Save);
    assert_eq!(fs::read(f.path()).unwrap(), b"xA");
    command(&mut bridge, Command::Undo);
    assert_eq!(bridge.content().text(), "A");
    bridge.close(true).unwrap();
}

#[test]
fn search_controls_revalidate_native_results_and_spend_prior_visual_input() {
    let f = Fixture::new(b"name name\r\n");
    let mut bridge = Bridge::open(&[f.path()], reference_profile()).unwrap();
    let old = bridge.token();
    bridge.find(&old, "name").unwrap();
    assert!(
        bridge
            .widget(&old, Action::Edit(Edit::Insert('!')))
            .is_err()
    );
    let found = bridge.token();
    bridge.command(&found, Command::NextMatch).unwrap();
    assert!(bridge.replace_match(&found, "wrong", false).is_err());
    bridge.replace_match(&bridge.token(), "X", false).unwrap();
    assert_eq!(bridge.content().text(), "name X\n");
    assert!(
        bridge
            .replace_match(&bridge.token(), "wrong", true)
            .is_err()
    );
    command(&mut bridge, Command::Undo);
    bridge.find(&bridge.token(), "name").unwrap();
    bridge.replace_match(&bridge.token(), "", true).unwrap();
    assert_eq!(bridge.content().text(), " \n");
    command(&mut bridge, Command::Undo);
    command(&mut bridge, Command::Save);
    bridge.close(false).unwrap();
    assert_eq!(fs::read(f.path()).unwrap(), b"name name\r\n");
}

#[test]
fn destination_confirmation_is_version_bound_and_spends_old_visual_input() {
    let a = Fixture::new(b"A");
    let b = Fixture::new(b"OLD");
    let mut bridge = Bridge::open(&[a.path()], reference_profile()).unwrap();
    let old = bridge.token();
    let stale = bridge.plan_destination(&old, b.path(), true).unwrap();
    bridge
        .widget(&old, Action::Edit(Edit::Insert('x')))
        .unwrap();
    assert!(bridge.confirm_destination(&old, &stale).is_err());
    assert_eq!(fs::read(b.path()).unwrap(), b"OLD");
    let token = bridge.token();
    let plan = bridge.plan_destination(&token, b.path(), true).unwrap();
    bridge.confirm_destination(&token, &plan).unwrap();
    assert!(bridge.confirm_destination(&token, &plan).is_err());
    bridge
        .widget(&bridge.token(), Action::Edit(Edit::Insert('y')))
        .unwrap();
    command(&mut bridge, Command::Save);
    bridge.close(false).unwrap();
    assert_eq!(fs::read(a.path()).unwrap(), b"A");
    assert_eq!(fs::read(b.path()).unwrap(), b"xyA");
}
