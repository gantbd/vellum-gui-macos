//! Provider-neutral clipboard attachment/projection routing. No platform I/O.
use super::*;
use vellum_editor::{ClipboardPlan, ClipboardReport};
use vellum_session::host::copy_transfer::*;

impl Bridge {
    /// Called only for an explicit source gesture. Old attachments refuse before
    /// lazy provider binding; the reference app is a trusted local-owner host.
    pub fn native_clipboard(
        &mut self,
        token: &ViewToken,
        operation: Option<vellum_clipboard_macos::Operation>,
    ) -> Result<String, BridgeError> {
        self.clipboard_attachment(token)?;
        let access = vellum_clipboard_macos::DesktopAccess {
            control: true,
            source_read: true,
            source_edit: true,
            clipboard_read: true,
            clipboard_write: true,
            retention: true,
        };
        let clipboard = self
            .native_clipboard
            .as_mut()
            .ok_or(BridgeError::Position)?;
        let result = match operation {
            Some(operation) => clipboard.perform(&mut self.editor, operation, access),
            None => clipboard.resume(&mut self.editor, access),
        };
        self.status = result
            .as_ref()
            .map_or_else(|e| e.to_string(), |r| r.notice());
        // A native publication may precede a cleanup error. Reflect actual source
        // truth, spend the old attachment first, and never relabel it as rollback.
        if self.editor.version()? != self.original.version {
            self.token = ViewToken::fresh();
            if let Err(error) = self.refresh_projection() {
                self.status = "Native clipboard decision recorded; view refresh failed".into();
                return Err(error);
            }
        }
        self.dirty = self.editor.is_dirty()?;
        result.map(|r| r.notice()).map_err(Into::into)
    }
    pub fn native_clipboard_stage(&self) -> Option<vellum_clipboard_macos::ClipboardStage> {
        self.native_clipboard.as_ref().map(|c| c.stage())
    }
    pub fn native_clipboard_usage(&self) -> Option<vellum_clipboard_macos::Usage> {
        self.native_clipboard.as_ref().and_then(|c| c.usage())
    }
    pub fn configure_clipboard(
        &mut self,
        limits: CopyLimitsV1,
        paste: Option<PasteLimitsV1>,
    ) -> Result<(), BridgeError> {
        Ok(self.editor.configure_clipboard(limits, paste)?)
    }
    pub fn register_clipboard_provider(
        &mut self,
        registration: CopyProviderRegistrationV1,
    ) -> Result<CopyProviderPortV1, BridgeError> {
        Ok(self.editor.register_clipboard_provider(registration)?)
    }
    pub fn retire_clipboard_provider(
        &mut self,
        provider: &CopyProviderPortV1,
        facts: CopyRetirementFactsV1,
    ) -> Result<(), BridgeError> {
        Ok(self.editor.retire_clipboard_provider(provider, facts)?)
    }
    fn clipboard_attachment(&mut self, token: &ViewToken) -> Result<(), BridgeError> {
        self.check(token)?;
        // The clipboard workflow remains qualified for one selection only;
        // secondary painting does not broaden clipboard interaction semantics.
        // Refuse hidden targets rather than exporting/editing only a primary.
        if self
            .editor
            .native_selection()?
            .is_none_or(|s| s.selections.selections().len() != 1)
        {
            return Err(BridgeError::Position);
        }
        Ok(())
    }
    pub fn begin_copy(
        &mut self,
        token: &ViewToken,
        provider: &CopyProviderPortV1,
        facts: CopyAdmissionFactsV1,
    ) -> Result<ClipboardPlan, BridgeError> {
        self.clipboard_attachment(token)?;
        Ok(self.editor.begin_copy(provider, facts)?)
    }
    pub fn begin_cut(
        &mut self,
        token: &ViewToken,
        provider: &CopyProviderPortV1,
        facts: CutAdmissionFactsV1,
    ) -> Result<ClipboardPlan, BridgeError> {
        self.clipboard_attachment(token)?;
        Ok(self.editor.begin_cut(provider, facts)?)
    }
    pub fn begin_paste(
        &mut self,
        token: &ViewToken,
        provider: &CopyProviderPortV1,
        facts: PasteAdmissionFactsV1,
    ) -> Result<ClipboardPlan, BridgeError> {
        self.clipboard_attachment(token)?;
        Ok(self.editor.begin_paste(provider, facts)?)
    }
    /// Pending work uses its original plan, not the currently focused token.
    pub fn dispatch_clipboard_export(
        &mut self,
        plan: &ClipboardPlan,
        facts: CopyDispatchFactsV1,
    ) -> Result<CopyDispatchV1, BridgeError> {
        Ok(self.editor.dispatch_clipboard_export(plan, facts)?)
    }
    pub fn dispatch_clipboard_paste(
        &mut self,
        plan: &ClipboardPlan,
        facts: PasteDispatchFactsV1,
    ) -> Result<CopyDispatchV1, BridgeError> {
        Ok(self.editor.dispatch_clipboard_paste(plan, facts)?)
    }
    pub fn release_clipboard_export(
        &mut self,
        provider: &CopyProviderPortV1,
        dispatch: &CopyDispatchV1,
        facts: CopyReleaseFactsV1,
    ) -> Result<String, BridgeError> {
        Ok(self
            .editor
            .release_clipboard_export(provider, dispatch, facts)?)
    }
    pub fn clipboard_export_evidence(
        &mut self,
        provider: &CopyProviderPortV1,
        dispatch: &CopyDispatchV1,
        evidence: CopyEvidenceV1,
        facts: CopyEvidenceFactsV1,
    ) -> Result<CopyEvidenceDispositionV1, BridgeError> {
        Ok(self
            .editor
            .clipboard_export_evidence(provider, dispatch, evidence, facts)?)
    }
    pub fn clipboard_paste_evidence(
        &mut self,
        provider: &CopyProviderPortV1,
        dispatch: &CopyDispatchV1,
        evidence: PasteEvidenceV1<'_>,
        facts: PasteEvidenceFactsV1,
    ) -> Result<CopyEvidenceDispositionV1, BridgeError> {
        Ok(self
            .editor
            .clipboard_paste_evidence(provider, dispatch, evidence, facts)?)
    }
    pub fn clipboard_report(
        &mut self,
        plan: &ClipboardPlan,
    ) -> Result<ClipboardReport, BridgeError> {
        Ok(self.editor.clipboard_report(plan)?)
    }
    pub fn complete_cut(
        &mut self,
        plan: &ClipboardPlan,
        facts: CutPublicationFactsV1,
    ) -> Result<ClipboardReport, BridgeError> {
        let report = self.editor.complete_cut(plan, facts)?;
        Ok(self.adopt_clipboard_report(report))
    }
    pub fn complete_paste(
        &mut self,
        plan: &ClipboardPlan,
        facts: PastePublicationFactsV1,
    ) -> Result<ClipboardReport, BridgeError> {
        let report = self.editor.complete_paste(plan, facts)?;
        Ok(self.adopt_clipboard_report(report))
    }
    fn adopt_clipboard_report(&mut self, mut report: ClipboardReport) -> ClipboardReport {
        self.status = self.editor.status().into();
        if report.fresh_local_decision
            && report.target_is_active
            && report.local == Some(TransferLocalDispositionV1::Applied)
        {
            self.token = ViewToken::fresh();
            // A projection failure cannot relabel an already applied native edit.
            if self.refresh_projection().is_err() {
                report.view_refresh_failed = true;
                self.status = "Clipboard local decision recorded; visual refresh failed".into();
            }
        }
        report
    }
    pub fn stop_clipboard(&mut self, plan: &ClipboardPlan) -> Result<(), BridgeError> {
        Ok(self.editor.stop_clipboard(plan)?)
    }
    pub fn retire_clipboard(
        &mut self,
        plan: &ClipboardPlan,
        facts: CopyRetirementFactsV1,
    ) -> Result<(), BridgeError> {
        Ok(self.editor.retire_clipboard(plan, facts)?)
    }
    pub fn clipboard_usage(&self) -> Option<CopyUsageV1> {
        self.editor.clipboard_usage()
    }
}
