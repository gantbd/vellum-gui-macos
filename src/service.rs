use iced::{
    Element, Subscription, Task, event,
    keyboard::{self, Key, Modifiers},
    widget::{button, column, row, text, text_editor, text_input},
    window,
};
use std::{path::PathBuf, time::Duration};
use vellum_sdk_client::{
    protocol::ServicePort,
    workspace::{PersistenceProjectionV1, ProjectionFreshnessV1, WorkspaceClientV1},
};

#[derive(Clone)]
pub enum ServiceMessage {
    Widget(text_editor::Action),
    PathChanged(String),
    AdmitFile,
    AdmitDirectory,
    Save,
    NextDocument,
    RetryExact,
    Poll,
    NativeClose(window::Id),
    Exit,
    ConfirmExit,
    CancelExit,
}

impl std::fmt::Debug for ServiceMessage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Widget(_) => "Widget(<redacted>)",
            Self::PathChanged(_) => "PathChanged(<redacted>)",
            Self::AdmitFile => "AdmitFile",
            Self::AdmitDirectory => "AdmitDirectory",
            Self::Save => "Save",
            Self::NextDocument => "NextDocument",
            Self::RetryExact => "RetryExact",
            Self::Poll => "Poll",
            Self::NativeClose(_) => "NativeClose",
            Self::Exit => "Exit",
            Self::ConfirmExit => "ConfirmExit",
            Self::CancelExit => "CancelExit",
        })
    }
}

enum PendingEffect {
    Replace {
        start: u64,
        end: u64,
        inserted: String,
    },
    Save,
}

/// Iced projection over the presentation-neutral workspace client. The GUI
/// owns only framework content/cursor state and literal form input; source,
/// membership, persistence and replay authority remain in the service client.
pub struct ServiceGui<T> {
    workspace: Option<WorkspaceClientV1<T>>,
    content: text_editor::Content,
    path: String,
    notice: String,
    confirming_exit: bool,
    pending: Option<PendingEffect>,
    projectable: bool,
    exit_requested: bool,
}

impl<T: ServicePort> ServiceGui<T> {
    fn failed(notice: String) -> Self {
        Self {
            workspace: None,
            content: text_editor::Content::new(),
            path: String::new(),
            notice,
            confirming_exit: false,
            pending: None,
            projectable: false,
            exit_requested: false,
        }
    }

    pub fn open(mut workspace: WorkspaceClientV1<T>) -> Self {
        let result = initialize(&mut workspace);
        let mut gui = Self {
            workspace: Some(workspace),
            content: text_editor::Content::new(),
            path: String::new(),
            notice: result.err().map_or_else(String::new, display_error),
            confirming_exit: false,
            pending: None,
            projectable: false,
            exit_requested: false,
        };
        gui.synchronize_content();
        gui
    }

    pub fn projection(&self) -> Option<&vellum_sdk_client::workspace::WorkspaceProjectionV1> {
        self.workspace.as_ref().map(WorkspaceClientV1::projection)
    }

    pub fn title(&self) -> String {
        let Some(projection) = self.projection() else {
            return "Vellum service — disconnected".into();
        };
        let count = projection
            .inventory()
            .map_or(0, |inventory| inventory.documents.len());
        let active = projection
            .inventory()
            .zip(projection.selected())
            .and_then(|(inventory, selected)| {
                inventory
                    .documents
                    .iter()
                    .position(|document| *document == selected)
            })
            .map_or(0, |index| index + 1);
        let dirty = matches!(
            projection.persistence(),
            PersistenceProjectionV1::Dirty | PersistenceProjectionV1::Indeterminate
        );
        format!(
            "Vellum service — document {active}/{count}{}{}",
            if dirty { " *" } else { "" },
            if projection.snapshot_freshness() == ProjectionFreshnessV1::Current && self.projectable
            {
                ""
            } else {
                " [editing locked]"
            }
        )
    }

    pub fn update(&mut self, message: ServiceMessage) -> Task<ServiceMessage> {
        let task = self.update_component(message);
        if self.exit_requested {
            iced::exit()
        } else {
            task
        }
    }

    /// Handles one pane message without terminating an embedding application.
    pub fn update_component(&mut self, message: ServiceMessage) -> Task<ServiceMessage> {
        match message {
            ServiceMessage::Widget(action) => self.edit(action),
            ServiceMessage::PathChanged(path) => {
                if path.len() <= 4096 {
                    self.path = path;
                    self.notice.clear();
                } else {
                    self.notice = "Path exceeds the 4096-byte frontend limit".into();
                }
            }
            ServiceMessage::AdmitFile => self.admit(false),
            ServiceMessage::AdmitDirectory => self.admit(true),
            ServiceMessage::Save => self.save(),
            ServiceMessage::NextDocument => self.next_document(),
            ServiceMessage::RetryExact => self.retry_exact(),
            ServiceMessage::Poll => self.poll(),
            ServiceMessage::NativeClose(_) | ServiceMessage::Exit => return self.request_exit(),
            ServiceMessage::ConfirmExit => return self.disconnect_and_exit(),
            ServiceMessage::CancelExit => {
                self.confirming_exit = false;
                self.notice.clear();
            }
        }
        Task::none()
    }

    pub fn subscription(&self) -> Subscription<ServiceMessage> {
        let close = window::close_requests().map(ServiceMessage::NativeClose);
        Subscription::batch([close, self.subscription_component()])
    }

    /// Service polling and pane-local shortcuts without owning window close.
    pub fn subscription_component(&self) -> Subscription<ServiceMessage> {
        let keys = event::listen_with(|event, status, _| {
            if status != event::Status::Ignored {
                return None;
            }
            let iced::Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. }) = event
            else {
                return None;
            };
            service_shortcut(&key, modifiers)
        });
        let poll = iced::time::every(Duration::from_millis(250)).map(|_| ServiceMessage::Poll);
        Subscription::batch([keys, poll])
    }

    pub fn view(&self) -> Element<'_, ServiceMessage> {
        let projection = self.projection();
        let editor = text_editor(&self.content)
            .font(iced::Font::MONOSPACE)
            .height(iced::Length::Fill)
            .on_action(ServiceMessage::Widget);
        let open = row![
            text_input("Literal service-side file or directory locator", &self.path)
                .on_input(ServiceMessage::PathChanged),
            button("Add file").on_press(ServiceMessage::AdmitFile),
            button("Add directory").on_press(ServiceMessage::AdmitDirectory),
        ]
        .spacing(8);
        let controls = row![
            button("Save").on_press(ServiceMessage::Save),
            button("Next file").on_press(ServiceMessage::NextDocument),
            button("Retry exact")
                .on_press_maybe(self.pending.as_ref().map(|_| ServiceMessage::RetryExact)),
            button("Quit").on_press(ServiceMessage::Exit),
        ]
        .spacing(8);
        let status = projection.map_or_else(
            || "Disconnected".into(),
            |projection| {
                format!(
                    "Service {:?} / {:?}",
                    projection.snapshot_freshness(),
                    projection.persistence()
                )
            },
        );
        let mut layout = column![
            controls,
            open,
            editor,
            text(status),
            text(&self.notice),
            text("Service projection: Cmd-S saves, F6 cycles documents, Cmd-Q exits. Source and persistence remain service-owned."),
        ]
        .spacing(8)
        .padding(12);
        if self.confirming_exit {
            layout = layout.push(
                row![
                    button("Disconnect and close").on_press(ServiceMessage::ConfirmExit),
                    button("Cancel").on_press(ServiceMessage::CancelExit),
                ]
                .spacing(8),
            );
        }
        layout.into()
    }

    fn edit(&mut self, action: text_editor::Action) {
        if self.pending.is_some() || !self.editable() {
            self.notice = "Service projection is not current; editing refused".into();
            return;
        }
        let before = self.content.text();
        self.content.perform(action);
        let after = self.content.text();
        let Some((start, end, inserted)) = single_replacement(&before, &after) else {
            return;
        };
        let (Ok(start), Ok(end)) = (u64::try_from(start), u64::try_from(end)) else {
            self.notice = "Edit position exceeds service range".into();
            self.synchronize_content();
            return;
        };
        let result = self
            .workspace
            .as_mut()
            .expect("editable workspace")
            .replace(start, end, inserted.clone());
        match result {
            Ok(()) => {
                self.notice.clear();
                self.synchronize_content_if_changed();
            }
            Err(error) => {
                if self.indeterminate() {
                    self.pending = Some(PendingEffect::Replace {
                        start,
                        end,
                        inserted,
                    });
                }
                self.notice = display_error(error);
                self.synchronize_content();
            }
        }
    }

    fn admit(&mut self, directory: bool) {
        if self.pending.is_some() {
            self.notice = "Resolve the indeterminate operation before another command".into();
            return;
        }
        if self.path.is_empty() {
            self.notice = "Enter a literal locator first".into();
            return;
        }
        let locator = self.path.clone();
        let result = self.workspace.as_mut().map_or_else(
            || Err("service disconnected".into()),
            |workspace| {
                let result = if directory {
                    workspace.admit_directory(locator)
                } else {
                    workspace.admit_file(locator)
                };
                result
                    .and_then(|()| workspace.refresh_inventory())
                    .and_then(|()| ensure_selection(workspace))
                    .map_err(display_error)
            },
        );
        match result {
            Ok(()) => {
                self.path.clear();
                self.notice = "Workspace membership updated".into();
                self.synchronize_content();
            }
            Err(error) => self.notice = error,
        }
    }

    fn save(&mut self) {
        if self.pending.is_some() || !self.editable() {
            self.notice = "Service projection is not current; save refused".into();
            return;
        }
        let result = self.workspace.as_mut().expect("editable workspace").save();
        match result {
            Ok(()) => self.notice = "Save completed".into(),
            Err(error) => {
                if self.indeterminate() {
                    self.pending = Some(PendingEffect::Save);
                }
                self.notice = display_error(error);
            }
        }
    }

    fn retry_exact(&mut self) {
        let Some(pending) = self.pending.take() else {
            return;
        };
        let Some(workspace) = self.workspace.as_mut() else {
            self.pending = Some(pending);
            return;
        };
        let result = match &pending {
            PendingEffect::Replace {
                start,
                end,
                inserted,
            } => workspace.replace(*start, *end, inserted.clone()),
            PendingEffect::Save => workspace.save(),
        };
        match result {
            Ok(()) => {
                self.notice = "Exact operation replay completed".into();
                self.synchronize_content();
            }
            Err(error) => {
                self.pending = Some(pending);
                self.notice = display_error(error);
                self.synchronize_content();
            }
        }
    }

    fn next_document(&mut self) {
        if self.pending.is_some() {
            self.notice = "Resolve the indeterminate operation before changing documents".into();
            return;
        }
        let result = self
            .workspace
            .as_mut()
            .ok_or_else(|| "service disconnected".to_owned())
            .and_then(|workspace| next_document(workspace).map_err(display_error));
        match result {
            Ok(()) => {
                self.notice.clear();
                self.synchronize_content();
            }
            Err(error) => self.notice = error,
        }
    }

    fn poll(&mut self) {
        if self.pending.is_some() {
            return;
        }
        let result: Result<(), vellum_sdk_client::workspace::WorkspaceClientErrorV1> =
            self.workspace.as_mut().map_or(Ok(()), |workspace| {
                workspace.poll_changes()?;
                if workspace.projection().inventory_freshness() == ProjectionFreshnessV1::Stale {
                    workspace.refresh_inventory()?;
                }
                if workspace.projection().snapshot_freshness() == ProjectionFreshnessV1::Stale {
                    workspace.refresh_snapshot()?;
                }
                Ok(())
            });
        match result {
            Ok(()) => self.synchronize_content_if_changed(),
            Err(error) => self.notice = display_error(error),
        }
    }

    fn request_exit(&mut self) -> Task<ServiceMessage> {
        if self.indeterminate() || self.pending.is_some() {
            self.notice = "Resolve the indeterminate operation by exact replay before exit".into();
            return Task::none();
        }
        if self
            .projection()
            .is_some_and(|projection| projection.persistence() == PersistenceProjectionV1::Dirty)
            && !self.confirming_exit
        {
            self.confirming_exit = true;
            self.notice = "Unsaved service document; confirm disconnect and close".into();
            return Task::none();
        }
        self.disconnect_and_exit()
    }

    fn disconnect_and_exit(&mut self) -> Task<ServiceMessage> {
        let Some(workspace) = self.workspace.take() else {
            self.exit_requested = true;
            return Task::none();
        };
        match workspace.disconnect() {
            Ok(_) => {
                self.exit_requested = true;
                Task::none()
            }
            Err(failure) => {
                let (workspace, error) = failure.into_parts();
                self.workspace = Some(workspace);
                self.notice = format!("service disconnect refused: {error:?}");
                Task::none()
            }
        }
    }

    /// Lifecycle output only; component mode never terminates the process.
    pub const fn exit_requested(&self) -> bool {
        self.exit_requested
    }

    fn editable(&self) -> bool {
        self.projection().is_some_and(|projection| {
            projection.snapshot_freshness() == ProjectionFreshnessV1::Current
                && projection.snapshot().is_some()
                && projection.persistence() != PersistenceProjectionV1::Indeterminate
                && self.projectable
        })
    }

    fn indeterminate(&self) -> bool {
        self.projection().is_some_and(|projection| {
            projection.persistence() == PersistenceProjectionV1::Indeterminate
        })
    }

    fn synchronize_content_if_changed(&mut self) {
        let source = self
            .projection()
            .and_then(|projection| projection.snapshot())
            .map(|snapshot| snapshot.text.as_ref());
        if source.is_some_and(|source| self.content.text() != source) || source.is_none() {
            self.synchronize_content();
        }
    }

    fn synchronize_content(&mut self) {
        let source = self
            .projection()
            .and_then(|projection| projection.snapshot())
            .map_or_else(String::new, |snapshot| snapshot.text.to_string());
        self.content = crate::bridge::projected_content(&source);
        self.projectable = self.content.text() == source;
        if !self.projectable && !source.is_empty() {
            self.notice =
                "Service snapshot cannot be represented exactly by this Iced editor; editing locked"
                    .into();
        }
    }
}

fn initialize<T: ServicePort>(
    workspace: &mut WorkspaceClientV1<T>,
) -> Result<(), vellum_sdk_client::workspace::WorkspaceClientErrorV1> {
    workspace.refresh_inventory()?;
    ensure_selection(workspace)
}

fn ensure_selection<T: ServicePort>(
    workspace: &mut WorkspaceClientV1<T>,
) -> Result<(), vellum_sdk_client::workspace::WorkspaceClientErrorV1> {
    if workspace.projection().selected().is_some() && workspace.projection().snapshot().is_some() {
        return Ok(());
    }
    let Some(first) = workspace
        .projection()
        .inventory()
        .and_then(|inventory| inventory.documents.first().copied())
    else {
        return Ok(());
    };
    if !workspace.projection().view_open() {
        workspace.open_view()?;
    } else if workspace.projection().selected().is_none() {
        workspace.select_document(first)?;
    }
    workspace.refresh_snapshot()
}

fn next_document<T: ServicePort>(
    workspace: &mut WorkspaceClientV1<T>,
) -> Result<(), vellum_sdk_client::workspace::WorkspaceClientErrorV1> {
    if workspace.projection().inventory_freshness() != ProjectionFreshnessV1::Current {
        workspace.refresh_inventory()?;
    }
    let Some(inventory) = workspace.projection().inventory() else {
        return Ok(());
    };
    if inventory.documents.is_empty() {
        return Ok(());
    }
    let current = workspace
        .projection()
        .selected()
        .and_then(|selected| {
            inventory
                .documents
                .iter()
                .position(|document| *document == selected)
        })
        .unwrap_or(0);
    let next = inventory.documents[(current + 1) % inventory.documents.len()];
    workspace.select_document(next)?;
    workspace.refresh_snapshot()
}

fn display_error(error: impl std::fmt::Debug) -> String {
    format!("service workspace refused: {error:?}")
}

fn service_shortcut(key: &Key, modifiers: Modifiers) -> Option<ServiceMessage> {
    match key.as_ref() {
        Key::Character("s" | "S") if modifiers.command() || modifiers.control() => {
            Some(ServiceMessage::Save)
        }
        Key::Character("q" | "Q") if modifiers.command() || modifiers.control() => {
            Some(ServiceMessage::Exit)
        }
        Key::Named(keyboard::key::Named::F6) => Some(ServiceMessage::NextDocument),
        _ => None,
    }
}

fn single_replacement(before: &str, after: &str) -> Option<(usize, usize, String)> {
    if before == after {
        return None;
    }
    let prefix = before
        .chars()
        .zip(after.chars())
        .take_while(|(left, right)| left == right)
        .map(|(character, _)| character.len_utf8())
        .sum::<usize>();
    let before_tail = &before[prefix..];
    let after_tail = &after[prefix..];
    let suffix = before_tail
        .chars()
        .rev()
        .zip(after_tail.chars().rev())
        .take_while(|(left, right)| left == right)
        .map(|(character, _)| character.len_utf8())
        .sum::<usize>();
    let end = before.len() - suffix;
    let inserted_end = after.len() - suffix;
    Some((prefix, end, after[prefix..inserted_end].to_owned()))
}

enum ServiceConnector {
    Plain(vellum_sdk_client::transport::UnixConnector),
    Authenticated(vellum_sdk_client::transport::AuthenticatedUnixConnector),
}

impl vellum_sdk_client::transport::FrameExchange for ServiceConnector {
    fn exchange(
        &mut self,
        request: &[u8],
        max_reply_bytes: usize,
    ) -> Result<Vec<u8>, vellum_sdk_client::transport::TransportError> {
        match self {
            Self::Plain(connector) => connector.exchange(request, max_reply_bytes),
            Self::Authenticated(connector) => connector.exchange(request, max_reply_bytes),
        }
    }
}

type ServiceTransport = vellum_sdk_client::transport::RemoteTransport<ServiceConnector>;

pub(crate) fn run_service_mode(
    args: &[std::ffi::OsString],
) -> Result<(), Box<dyn std::error::Error>> {
    let (socket, credential) = match args {
        [service, socket] if service == "--service" => (PathBuf::from(socket), None),
        [service, socket, flag, credential] if service == "--service" && flag == "--credential" => {
            (PathBuf::from(socket), Some(PathBuf::from(credential)))
        }
        _ => {
            return Err(std::io::Error::other(
                "usage: vellum-gui-macos --service SOCKET [--credential CREDENTIAL]",
            )
            .into());
        }
    };
    iced::application(
        move || match connect_service(&socket, credential.as_deref()) {
            Ok(workspace) => ServiceGui::<ServiceTransport>::open(workspace),
            Err(error) => ServiceGui::<ServiceTransport>::failed(error),
        },
        ServiceGui::update,
        ServiceGui::view,
    )
    .title(ServiceGui::title)
    .subscription(ServiceGui::subscription)
    .exit_on_close_request(false)
    .window_size((960.0, 720.0))
    .run()?;
    Ok(())
}

fn connect_service(
    socket: &std::path::Path,
    credential: Option<&std::path::Path>,
) -> Result<WorkspaceClientV1<ServiceTransport>, String> {
    use vellum_sdk_client::transport::{
        AuthenticatedUnixConnector, RemoteTransport, TransportLimits, UnixConnector,
    };
    let connector = if let Some(credential) = credential {
        let mut store = vellum_adapter_local_authentication::LocalCredentialStoreV1::default();
        let record = store
            .load(credential)
            .map_err(|error| format!("credential refused: {error:?}"))?;
        let provider =
            vellum_adapter_local_authentication::LocalCredentialClaimProviderV1::new(record);
        ServiceConnector::Authenticated(AuthenticatedUnixConnector::with_provider(
            socket.to_owned(),
            provider,
        ))
    } else {
        ServiceConnector::Plain(UnixConnector::new(socket.to_owned()))
    };
    let transport = RemoteTransport::new(connector, TransportLimits::reference())
        .map_err(|error| format!("transport refused: {error:?}"))?;
    WorkspaceClientV1::connect(transport)
        .map_err(|error| format!("service connection refused: {error:?}"))
}

#[cfg(test)]
mod tests {
    use super::{ServiceGui, ServiceMessage, single_replacement};
    use iced::widget::text_editor;
    use std::{collections::VecDeque, sync::Arc};
    use vellum_sdk_client::{
        protocol::{
            ClientId, OutcomeV1, ReplyV1, RequestV1, SaveApplicationV1, SaveReceiptV1,
            SaveSynchronizationV1, ServiceAccessErrorV1, ServiceControlErrorV1, ServicePort,
            SnapshotV1, VersionWitness, ViewOpenReceiptV1, ViewOpenStatusV1, WorkspaceDocumentIdV1,
            WorkspaceIdV1, WorkspaceInventoryV1,
        },
        workspace::{PersistenceProjectionV1, WorkspaceClientV1},
    };

    struct ScriptedPort {
        replies: VecDeque<Result<ReplyV1, ServiceAccessErrorV1>>,
    }

    impl ServicePort for ScriptedPort {
        fn connect(&mut self) -> Result<ClientId, ServiceControlErrorV1> {
            Ok(ClientId::new(1).unwrap())
        }

        fn execute(&mut self, _: RequestV1) -> Result<ReplyV1, ServiceAccessErrorV1> {
            self.replies.pop_front().unwrap()
        }

        fn disconnect(&mut self, _: ClientId) -> Result<(), ServiceControlErrorV1> {
            Ok(())
        }
    }

    fn workspace() -> WorkspaceIdV1 {
        WorkspaceIdV1::new(7).unwrap()
    }

    fn document() -> WorkspaceDocumentIdV1 {
        WorkspaceDocumentIdV1::new(workspace(), 9).unwrap()
    }

    fn controller(
        replies: impl IntoIterator<Item = Result<ReplyV1, ServiceAccessErrorV1>>,
    ) -> WorkspaceClientV1<ScriptedPort> {
        WorkspaceClientV1::connect(ScriptedPort {
            replies: replies.into_iter().collect(),
        })
        .unwrap()
    }

    fn inventory() -> ReplyV1 {
        ReplyV1::Accepted(OutcomeV1::WorkspaceInventory(WorkspaceInventoryV1 {
            workspace: workspace(),
            documents: vec![document()],
            roots: Vec::new(),
        }))
    }

    fn opened() -> ReplyV1 {
        ReplyV1::Accepted(OutcomeV1::ViewOpened(ViewOpenReceiptV1 {
            document: document(),
            status: ViewOpenStatusV1::Opened,
        }))
    }

    fn snapshot(witness: u64, revision: u64, text: &'static str) -> SnapshotV1 {
        SnapshotV1 {
            witness: VersionWitness::new(witness).unwrap(),
            revision,
            text: Arc::from(text),
        }
    }

    #[test]
    fn replacement_projection_is_unicode_bounded_and_total_for_insertion_and_deletion() {
        assert_eq!(
            single_replacement("a🙂c", "a界🙂c"),
            Some((1, 1, "界".into()))
        );
        assert_eq!(
            single_replacement("a🙂c", "ac"),
            Some((1, 5, String::new()))
        );
        assert_eq!(single_replacement("same", "same"), None);
    }

    #[test]
    fn component_disconnect_emits_lifecycle_without_owning_process_exit() {
        let mut gui = ServiceGui::<ScriptedPort>::failed("unavailable".into());
        let _ = gui.update_component(ServiceMessage::Exit);
        assert!(gui.exit_requested());
    }

    #[test]
    fn service_gui_projects_framework_edit_and_save_through_witnessed_client() {
        let gui = ServiceGui::open(controller([
            Ok(inventory()),
            Ok(opened()),
            Ok(ReplyV1::Accepted(OutcomeV1::Snapshot(snapshot(
                3, 4, "source",
            )))),
            Ok(ReplyV1::Accepted(OutcomeV1::Replaced(snapshot(
                5, 5, "Xsource",
            )))),
            Ok(ReplyV1::Accepted(OutcomeV1::Saved(SaveReceiptV1 {
                application: SaveApplicationV1::Written,
                synchronization: SaveSynchronizationV1::Succeeded,
                no_future_effects: true,
                quiescent: true,
                adapter_failure: false,
                cleanup_failed: false,
                baseline_published: true,
                outcome_unknown: false,
            }))),
        ]));
        let mut gui = gui;
        let _ = gui.update(ServiceMessage::Widget(text_editor::Action::Edit(
            text_editor::Edit::Insert('X'),
        )));
        assert_eq!(gui.content.text(), "Xsource");
        assert_eq!(
            gui.projection().unwrap().persistence(),
            PersistenceProjectionV1::Dirty
        );
        let _ = gui.update(ServiceMessage::Save);
        assert_eq!(
            gui.projection().unwrap().persistence(),
            PersistenceProjectionV1::Clean
        );
    }

    #[test]
    fn uncertain_framework_edit_is_locked_and_exactly_replayed() {
        let mut gui = ServiceGui::open(controller([
            Ok(inventory()),
            Ok(opened()),
            Ok(ReplyV1::Accepted(OutcomeV1::Snapshot(snapshot(
                3, 4, "source",
            )))),
            Err(ServiceAccessErrorV1::OutcomeUnknown),
            Ok(ReplyV1::Accepted(OutcomeV1::Replaced(snapshot(
                5, 5, "Xsource",
            )))),
        ]));
        let _ = gui.update(ServiceMessage::Widget(text_editor::Action::Edit(
            text_editor::Edit::Insert('X'),
        )));
        assert!(gui.pending.is_some());
        assert_eq!(gui.content.text(), "source");
        assert_eq!(
            gui.projection().unwrap().persistence(),
            PersistenceProjectionV1::Indeterminate
        );
        let _ = gui.update(ServiceMessage::RetryExact);
        assert!(gui.pending.is_none());
        assert_eq!(gui.content.text(), "Xsource");
    }
}
