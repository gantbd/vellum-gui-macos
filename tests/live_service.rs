#![cfg(target_os = "macos")]

use iced::widget::text_editor;
use std::{path::PathBuf, time::Duration};
use vellum_gui_macos::{ServiceGui, ServiceMessage};
use vellum_sdk_client::{
    transport::{RemoteTransport, TransportLimits, UnixConnector},
    workspace::{PersistenceProjectionV1, WorkspaceClientV1},
};

#[test]
#[ignore = "started by tests/live/service_journey.sh against a real daemon"]
fn real_daemon_open_edit_save_disconnect_journey() {
    let socket = PathBuf::from(
        std::env::var_os("VELLUM_LIVE_SOCKET").expect("live harness supplies socket"),
    );
    let connector = UnixConnector::with_timeout(socket, Duration::from_secs(2)).unwrap();
    let transport = RemoteTransport::new(connector, TransportLimits::reference()).unwrap();
    let workspace = WorkspaceClientV1::connect(transport).unwrap();
    let mut gui = ServiceGui::open(workspace);

    assert_eq!(
        gui.projection().unwrap().snapshot().unwrap().text.as_ref(),
        "alpha"
    );
    drop(gui.update(ServiceMessage::Widget(text_editor::Action::Edit(
        text_editor::Edit::Insert('X'),
    ))));
    assert_eq!(
        gui.projection().unwrap().snapshot().unwrap().text.as_ref(),
        "Xalpha"
    );
    assert_eq!(
        gui.projection().unwrap().persistence(),
        PersistenceProjectionV1::Dirty
    );

    drop(gui.update(ServiceMessage::Save));
    assert_eq!(
        gui.projection().unwrap().persistence(),
        PersistenceProjectionV1::Clean
    );
    drop(gui.update(ServiceMessage::ConfirmExit));
    assert!(gui.projection().is_none());
}
