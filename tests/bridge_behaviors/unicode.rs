use super::*;
use vellum_gui_macos::BridgeError;
use vellum_model::NativeUnicodeProfile;

fn native(f: &Fixture, capacity: usize) -> Bridge {
    let mut config = reference_profile();
    config.capture_capacity = capacity;
    let mut bridge = Bridge::open(&[f.path()], config).unwrap();
    let mut profile = NativeUnicodeProfile::reference();
    profile.capture_capacity = capacity;
    bridge
        .enable_native_unicode(&bridge.token(), profile)
        .unwrap();
    bridge
}
#[test]
fn source_keyboard_uses_native18_clusters_cached_evidence_history_and_cleanup() {
    for capacity in [0, 1, 256] {
        let raw = "\u{feff}\u{94d}\u{915}\r\ne\u{301}";
        let f = Fixture::new(raw.as_bytes());
        let mut bridge = native(&f, capacity);
        send(&mut bridge, Action::Move(Motion::Right));
        assert_eq!(
            bridge.content().cursor().position,
            Position { line: 0, column: 6 }
        );
        for _ in 0..8 {
            send(&mut bridge, Action::Move(Motion::Left));
            send(&mut bridge, Action::Move(Motion::Right));
        }
        assert_eq!(bridge.native_unicode_usage().unwrap().completed, 1);
        assert_eq!(bridge.native_boundary_usage().ready, 1);
        send(&mut bridge, Action::Select(Motion::Right));
        assert_eq!(
            bridge.content().cursor().position,
            Position { line: 1, column: 0 }
        );
        send(&mut bridge, Action::Edit(Edit::Backspace));
        assert_eq!(bridge.content().text(), "\u{94d}\u{915}e\u{301}");
        command(&mut bridge, Command::Undo);
        assert_eq!(bridge.content().text(), "\u{94d}\u{915}\ne\u{301}");
        bridge.close(false).unwrap();
        assert_eq!(bridge.native_boundary_usage().retained_bytes, 0);
        assert_eq!(fs::read(f.path()).unwrap(), raw.as_bytes());
    }
}

#[test]
fn native_candidate_join_and_deletion_stay_incremental_and_undo_once() {
    let f = Fixture::new("कX".as_bytes());
    let mut bridge = native(&f, 256);
    let refreshes = bridge.full_projection_refreshes();
    send(&mut bridge, Action::Edit(Edit::Insert('\u{94d}')));
    assert_eq!(bridge.content().text(), "\u{94d}कX");
    assert_eq!(bridge.content().cursor().position.column, 6);
    let proposed = bridge.proposal(&bridge.token(), &Edit::Backspace).unwrap();
    assert_eq!((proposed.edits[0].start, proposed.edits[0].end), (0, 6));
    send(&mut bridge, Action::Edit(Edit::Backspace));
    assert_eq!(bridge.content().text(), "X");
    assert_eq!(bridge.full_projection_refreshes(), refreshes);
    // Candidate service and exact-version evidence are two real populations of
    // provider work, not a fabricated one-call-per-version measurement.
    assert_eq!(bridge.native_unicode_usage().unwrap().completed, 4);
    command(&mut bridge, Command::Undo);
    assert_eq!(bridge.content().text(), "\u{94d}कX");
    command(&mut bridge, Command::Undo);
    assert_eq!(bridge.content().text(), "कX");
    bridge.close(false).unwrap();
}

#[test]
fn exact_partial_targets_are_not_repaired_by_delete_or_scroll() {
    let f = Fixture::new("e\u{301}X".as_bytes());
    let mut bridge = native(&f, 256);
    bridge
        .set_cursor(
            &bridge.token(),
            Cursor {
                position: Position { line: 0, column: 1 },
                selection: None,
            },
        )
        .unwrap();
    assert!(
        bridge
            .widget(&bridge.token(), Action::Edit(Edit::Backspace))
            .is_err()
    );
    assert_eq!(bridge.content().text(), "e\u{301}X");
    assert_eq!(bridge.content().cursor().position.column, 1);
    bridge
        .set_cursor(
            &bridge.token(),
            Cursor {
                position: Position { line: 0, column: 3 },
                selection: Some(Position { line: 0, column: 1 }),
            },
        )
        .unwrap();
    send(&mut bridge, Action::Scroll { lines: 1 });
    let selected = bridge
        .native_selection(&bridge.token())
        .unwrap()
        .unwrap()
        .selections
        .selections()[0];
    assert_eq!((selected.anchor.0, selected.head.0), (1, 3));
    send(&mut bridge, Action::Edit(Edit::Delete));
    assert_eq!(bridge.content().text(), "eX");
    bridge.close(true).unwrap();
}

#[test]
fn refused_candidate_is_not_published_and_edge_noop_still_checks_native_policy() {
    let f = Fixture::new(b"a");
    let mut bridge = Bridge::open(&[f.path()], reference_profile()).unwrap();
    let profile = NativeUnicodeProfile::reference();
    let mut factory = vellum_adapter_unicode_icu::IcuFactory::reference();
    factory.limits.max_source_bytes = 1;
    bridge
        .enable_native_unicode_with(&bridge.token(), factory, profile)
        .unwrap();
    assert!(
        bridge
            .widget(&bridge.token(), Action::Edit(Edit::Insert('X')))
            .is_err()
    );
    assert_eq!(bridge.content().text(), "a");
    assert!(!bridge.dirty());
    assert_eq!(bridge.native_boundary_usage().issued_lifetime, 0);
    assert_eq!(bridge.native_unicode_usage().unwrap().refused, 1);
    send(&mut bridge, Action::Move(Motion::Right));
    command(&mut bridge, Command::ToggleEditingLock);
    assert!(
        bridge
            .widget(&bridge.token(), Action::Edit(Edit::Delete))
            .is_err()
    );
    assert_eq!(bridge.content().text(), "a");
    command(&mut bridge, Command::ToggleEditingLock);
    bridge.close(false).unwrap();
}

#[test]
fn opt_in_and_native_work_require_current_attachment_even_after_same_path_aba() {
    let f = Fixture::new("\u{94d}कX".as_bytes());
    let other = Fixture::new(b"B");
    let mut bridge = Bridge::open(&[f.path()], reference_profile()).unwrap();
    let old = bridge.token();
    command(&mut bridge, Command::NextDocument);
    assert!(matches!(
        bridge.enable_native_unicode(&old, NativeUnicodeProfile::reference()),
        Err(BridgeError::Stale)
    ));
    assert!(!bridge.native_unicode_enabled());
    bridge
        .enable_native_unicode(&bridge.token(), NativeUnicodeProfile::reference())
        .unwrap();
    send(&mut bridge, Action::Move(Motion::Right));
    assert_eq!(bridge.content().cursor().position.column, 6);
    let original = bridge.token();
    bridge
        .open_document(&bridge.token(), &other.path())
        .unwrap();
    send(&mut bridge, Action::Move(Motion::Right));
    assert_eq!(bridge.content().cursor().position.column, 1);
    bridge.open_document(&bridge.token(), &f.path()).unwrap();
    let before = bridge.native_unicode_usage().unwrap();
    assert!(matches!(
        bridge.widget(&original, Action::Edit(Edit::Backspace)),
        Err(BridgeError::Stale)
    ));
    assert_eq!(bridge.native_unicode_usage().unwrap(), before);
    send(&mut bridge, Action::Move(Motion::Left));
    assert_eq!(bridge.native_boundary_usage().ready, 1);
    assert_eq!(bridge.native_unicode_usage().unwrap().completed, 3);
    bridge.close(false).unwrap();
    assert_eq!(bridge.native_boundary_usage().retained_bytes, 0);
}
