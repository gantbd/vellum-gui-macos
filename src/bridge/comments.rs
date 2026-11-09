//! Annotation UI routing over the original editor/native owners.
use super::*;
use vellum_editor::{
    CommentDraft, CommentHandle, CommentHistoryDirection, CommentPointProfile, CommentRow,
};
impl Bridge {
    pub fn comment_file_bound(&self) -> bool {
        self.editor.comment_file_bound()
    }
    pub fn comment_file_needs_load(&self) -> bool {
        self.editor.comment_file_needs_load()
    }
    pub fn comments_dirty(&mut self) -> Result<bool, BridgeError> {
        Ok(self.editor.comments_dirty()?)
    }
    pub fn plan_comment_file(
        &mut self,
        token: &ViewToken,
        path: PathBuf,
        load: bool,
    ) -> Result<vellum_editor::CommentFilePlan, BridgeError> {
        self.check(token)?;
        Ok(self.editor.plan_comment_file(path, load)?)
    }
    pub fn cancel_comment_file_plan(&mut self) {
        self.editor.cancel_comment_file_plan();
    }
    pub fn confirm_comment_file(
        &mut self,
        token: &ViewToken,
        plan: &vellum_editor::CommentFilePlan,
    ) -> Result<(), BridgeError> {
        self.check(token)?;
        let result = self.editor.confirm_comment_file(plan);
        self.comment_result(result)
    }
    pub fn comment_file_action(
        &mut self,
        token: &ViewToken,
        load: bool,
    ) -> Result<(), BridgeError> {
        self.check(token)?;
        let result = if load {
            self.editor.load_comment_file()
        } else {
            self.editor.save_comment_file()
        };
        self.comment_result(result)
    }
    pub fn release_failed_comment_read(&mut self, token: &ViewToken) -> Result<(), BridgeError> {
        self.check(token)?;
        let result = self.editor.retire_failed_comment_read().map(|_| ());
        self.comment_result(result)
    }
    pub fn refresh_comments(&mut self, token: &ViewToken) -> Result<(), BridgeError> {
        self.check(token)?;
        self.editor.comments()?;
        Ok(())
    }
    pub fn cached_comments(&self) -> &[CommentRow] {
        self.editor
            .cached_comments(&self.original.version)
            .unwrap_or(&[])
    }
    pub fn comments_locked(&self) -> bool {
        self.editor.comments_locked()
    }
    pub fn comment_body_limit(&self) -> Option<usize> {
        self.editor.comment_body_limit()
    }
    pub fn begin_comment(
        &mut self,
        token: &ViewToken,
        profile: CommentPointProfile,
    ) -> Result<CommentDraft, BridgeError> {
        self.check(token)?;
        if self
            .editor
            .native_selection()?
            .is_none_or(|s| s.selections.selections().len() != 1)
        {
            return Err(BridgeError::Position);
        }
        Ok(self.editor.begin_comment(profile)?)
    }
    fn comment_result(&mut self, result: Result<(), EditorError>) -> Result<(), BridgeError> {
        self.dirty = self.editor.is_dirty()?;
        self.status = self.editor.status().into();
        result.map_err(Into::into)
    }
    pub fn create_comment(
        &mut self,
        token: &ViewToken,
        draft: &CommentDraft,
        body: &str,
    ) -> Result<(), BridgeError> {
        self.check(token)?;
        let result = self.editor.create_comments(draft, body).map(|_| ());
        self.comment_result(result)
    }
    pub fn update_comment(
        &mut self,
        token: &ViewToken,
        handle: &CommentHandle,
        body: &str,
    ) -> Result<(), BridgeError> {
        self.check(token)?;
        let result = self.editor.update_comment(handle, body);
        self.comment_result(result)
    }
    pub fn delete_comment(
        &mut self,
        token: &ViewToken,
        handle: &CommentHandle,
    ) -> Result<(), BridgeError> {
        self.check(token)?;
        let result = self.editor.delete_comment(handle);
        self.comment_result(result)
    }
    pub fn reattach_comment(
        &mut self,
        token: &ViewToken,
        handle: &CommentHandle,
        profile: CommentPointProfile,
    ) -> Result<(), BridgeError> {
        let draft = self.begin_comment(token, profile)?;
        let result = self.editor.reattach_comment(handle, &draft);
        self.comment_result(result)
    }
    pub fn reverse_comment(
        &mut self,
        token: &ViewToken,
        direction: CommentHistoryDirection,
    ) -> Result<(), BridgeError> {
        self.check(token)?;
        let before = self.editor.native_selection()?.and_then(|s| s.context());
        let result = self.editor.reverse_comment(direction).map(|_| ());
        let after = self.editor.native_selection()?.and_then(|s| s.context());
        if self.editor.version()? != self.original.version || before != after {
            self.token = ViewToken::fresh();
            self.refresh_projection()?;
        }
        self.comment_result(result)
    }
    pub fn lock_comments(&mut self, token: &ViewToken, locked: bool) -> Result<(), BridgeError> {
        self.check(token)?;
        Ok(self.editor.set_comments_locked(locked)?)
    }
}
