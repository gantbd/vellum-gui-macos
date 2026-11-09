use super::*;
use vellum_engine::coordinator::InvocationLimits;
use vellum_session::host::{SessionConfig, TextTransferCaptureLimitsV1, copy_transfer::*};
fn config() -> SessionConfig {
    let mut c = reference_profile();
    c.engine.invocation = Some(InvocationLimits {
        max_lifetime_issued: 32,
        max_retained: 8,
        max_intent_bytes: 262144,
    });
    c
}
fn setup(paths: &[PathBuf]) -> (Bridge, CopyProviderPortV1) {
    let mut b = Bridge::open(paths, config()).unwrap();
    b.configure_clipboard(
        CopyLimitsV1 {
            max_provider_registrations: 1,
            max_profile_bytes: 64,
            max_tasks: 4,
            max_held_bytes: 32768,
            capture: TextTransferCaptureLimitsV1 {
                max_fragments: 16,
                max_plain_bytes: 4096,
                max_logical_bytes: 8192,
            },
        },
        Some(PasteLimitsV1 {
            max_targets: 16,
            max_plain_bytes: 256,
            max_expanded_bytes: 4096,
        }),
    )
    .unwrap();
    let p = b
        .register_clipboard_provider(CopyProviderRegistrationV1 {
            destination: CopyDestinationV1::SessionClipboard,
            readable: true,
            writable: true,
            acknowledgment_profile: "test-iced/1".into(),
        })
        .unwrap();
    (b, p)
}
fn admit() -> PasteAdmissionFactsV1 {
    PasteAdmissionFactsV1 {
        target_read_allowed: Some(true),
        edit_allowed: Some(true),
        view_write_allowed: Some(true),
        retention_allowed: Some(true),
    }
}
fn launch() -> PasteDispatchFactsV1 {
    PasteDispatchFactsV1 {
        launch_allowed: Some(true),
        clipboard_read_allowed: Some(true),
    }
}
fn evidence() -> PasteEvidenceFactsV1 {
    PasteEvidenceFactsV1 {
        evidence: CopyEvidenceFactsV1 {
            authentic: Some(true),
            evidence_allowed: Some(true),
            ingress_ready: Some(true),
        },
        payload_retention_allowed: Some(true),
    }
}
fn publish() -> PastePublicationFactsV1 {
    PastePublicationFactsV1 {
        edit_allowed: Some(true),
        view_write_allowed: Some(true),
        payload_use_allowed: Some(true),
    }
}
fn retire() -> CopyRetirementFactsV1 {
    CopyRetirementFactsV1 {
        retention_released: Some(true),
        delivery_drained: Some(true),
        session_references_released: Some(true),
    }
}
fn select_a(b: &mut Bridge) {
    b.set_cursor(
        &b.token(),
        Cursor {
            position: Position { line: 0, column: 1 },
            selection: Some(Position { line: 0, column: 0 }),
        },
    )
    .unwrap();
}

#[test]
fn paste_refreshes_native_widget_projection_once_and_old_input_cannot_replay_afterward() {
    let f = Fixture::new("\u{feff}A\r\nB".as_bytes());
    let (mut b, p) = setup(&[f.path()]);
    select_a(&mut b);
    let old = b.token();
    let plan = b.begin_paste(&old, &p, admit()).unwrap();
    let d = b.dispatch_clipboard_paste(&plan, launch()).unwrap();
    b.clipboard_paste_evidence(&p, &d, PasteEvidenceV1::Accepted("Grüße\r\nX"), evidence())
        .unwrap();
    let r = b.complete_paste(&plan, publish()).unwrap();
    assert_eq!(r.local, Some(TransferLocalDispositionV1::Applied));
    assert!(r.fresh_local_decision && !r.view_refresh_failed);
    assert_ne!(b.token(), old);
    assert_eq!(b.content().text(), "Grüße\nX\nB");
    assert!(b.widget(&old, Action::Edit(Edit::Insert('!'))).is_err());
    let current = b.token();
    let refreshes = b.full_projection_refreshes();
    assert!(
        !b.complete_paste(&plan, publish())
            .unwrap()
            .fresh_local_decision
    );
    assert_eq!(b.token(), current);
    assert_eq!(b.full_projection_refreshes(), refreshes);
    command(&mut b, Command::Undo);
    assert_eq!(b.content().text(), "A\nB");
    let undone = b.token();
    b.complete_paste(&plan, publish()).unwrap();
    assert_eq!(b.content().text(), "A\nB");
    assert_eq!(b.token(), undone);
    command(&mut b, Command::Redo);
    command(&mut b, Command::Save);
    assert_eq!(
        fs::read(f.path()).unwrap(),
        "\u{feff}Grüße\r\nX\r\nB".as_bytes()
    );
    b.clipboard_paste_evidence(&p, &d, PasteEvidenceV1::Quiescent, evidence())
        .unwrap();
    b.retire_clipboard(&plan, retire()).unwrap();
    b.retire_clipboard_provider(&p, retire()).unwrap();
    b.close(false).unwrap();
}

#[test]
fn pending_original_paste_can_complete_in_background_without_changing_active_attachment() {
    let a = Fixture::new(b"A\r\n");
    let other = Fixture::new(b"B\n");
    let (mut b, p) = setup(&[a.path(), other.path()]);
    select_a(&mut b);
    let first = b.token();
    let plan = b.begin_paste(&first, &p, admit()).unwrap();
    command(&mut b, Command::NextDocument);
    let second = b.token();
    assert!(b.begin_paste(&first, &p, admit()).is_err());
    let d = b.dispatch_clipboard_paste(&plan, launch()).unwrap();
    b.clipboard_paste_evidence(&p, &d, PasteEvidenceV1::Accepted("ß"), evidence())
        .unwrap();
    let r = b.complete_paste(&plan, publish()).unwrap();
    assert!(r.fresh_local_decision && !r.target_is_active);
    assert_eq!(b.active_document(), 1);
    assert_eq!(b.content().text(), "B\n");
    assert_eq!(b.token(), second);
    assert!(!b.dirty());
    b.clipboard_paste_evidence(&p, &d, PasteEvidenceV1::Quiescent, evidence())
        .unwrap();
    b.retire_clipboard(&plan, retire()).unwrap();
    b.retire_clipboard_provider(&p, retire()).unwrap();
    command(&mut b, Command::NextDocument);
    assert_eq!(b.content().text(), "ß\n");
    assert!(b.dirty());
    command(&mut b, Command::Save);
    assert_eq!(fs::read(a.path()).unwrap(), "ß\r\n".as_bytes());
    assert_eq!(fs::read(other.path()).unwrap(), b"B\n");
    b.close(false).unwrap();
}

#[test]
fn equal_text_view_effect_refreshes_but_true_empty_noop_does_not_spend_attachment() {
    let f = Fixture::new(b"A\r\nB");
    let (mut b, p) = setup(&[f.path()]);
    select_a(&mut b);
    let old = b.token();
    let plan = b.begin_paste(&old, &p, admit()).unwrap();
    let d = b.dispatch_clipboard_paste(&plan, launch()).unwrap();
    b.clipboard_paste_evidence(&p, &d, PasteEvidenceV1::Accepted("A"), evidence())
        .unwrap();
    let r = b.complete_paste(&plan, publish()).unwrap();
    assert_eq!(r.local, Some(TransferLocalDispositionV1::Applied));
    assert_ne!(b.token(), old);
    assert!(b.content().cursor().selection.is_none());
    assert!(!b.dirty());
    b.clipboard_paste_evidence(&p, &d, PasteEvidenceV1::Quiescent, evidence())
        .unwrap();
    b.retire_clipboard(&plan, retire()).unwrap();
    let old = b.token();
    let refreshes = b.full_projection_refreshes();
    let plan = b.begin_paste(&old, &p, admit()).unwrap();
    let d = b.dispatch_clipboard_paste(&plan, launch()).unwrap();
    b.clipboard_paste_evidence(&p, &d, PasteEvidenceV1::Accepted(""), evidence())
        .unwrap();
    let r = b.complete_paste(&plan, publish()).unwrap();
    assert_eq!(r.local, Some(TransferLocalDispositionV1::Unchanged));
    assert_eq!(b.token(), old);
    assert_eq!(b.full_projection_refreshes(), refreshes);
    b.clipboard_paste_evidence(&p, &d, PasteEvidenceV1::Quiescent, evidence())
        .unwrap();
    b.retire_clipboard(&plan, retire()).unwrap();
    b.retire_clipboard_provider(&p, retire()).unwrap();
    b.close(false).unwrap();
}
