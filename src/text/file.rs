use std::path::PathBuf;

use encoding_rs::Encoding;

use crate::text::document::{Document, Eol, content_hash};

impl Document {
    pub fn recovered(
        id: u64,
        untitled: u32,
        path: Option<PathBuf>,
        remote: Option<(String, String)>,
        name: Option<String>,
    ) -> Self {
        let mut doc = Self::empty(id, untitled);
        doc.path = path;
        doc.remote = remote;
        doc.name = name;
        doc
    }

    pub fn has_path(&self) -> bool {
        self.path.is_some()
    }

    pub fn is_remote(&self) -> bool {
        self.remote.is_some()
    }

    pub fn is_new(&self) -> bool {
        self.path.is_none() && self.remote.is_none()
    }

    pub fn set_syntax(&mut self, syntax: String) {
        if self.syntax != syntax {
            self.syntax = syntax;
        }
    }

    pub fn prefer_html(&mut self) {
        if self.syntax == "Plain Text" {
            self.syntax = "HTML".into();
        }
    }

    pub fn present_as(&mut self, name: String, syntax: String) {
        self.remote = None;
        self.name = Some(name);
        self.set_syntax(syntax);
    }

    pub fn set_eol(&mut self, eol: Eol) {
        if self.eol != eol {
            self.eol = eol;
            self.mark_modified();
        }
    }

    pub fn set_encoding(&mut self, encoding: &'static Encoding, bom: bool) {
        if self.encoding != encoding || self.bom != bom {
            self.encoding = encoding;
            self.bom = bom;
            self.mark_modified();
        }
    }

    pub fn reload(&mut self) -> Result<(), String> {
        self.reopen_as(self.encoding)
    }

    pub fn check_on_disk(&mut self) {
        if let Some(path) = &self.path {
            let missing = !path.exists();
            self.set_missing(missing);
        }
    }

    pub fn set_missing(&mut self, missing: bool) {
        self.missing = missing;
        self.missing_ack = self.missing_ack && missing;
    }

    pub fn ack_missing(&mut self) {
        self.missing_ack = true;
    }

    pub fn begin_upload(&mut self, data: &[u8]) -> Option<u64> {
        self.pending_hash = Some(content_hash(data));
        self.server_hash
    }

    pub fn finish_upload(&mut self, version: u64) {
        self.mark_saved(version);
        if let Some(hash) = self.pending_hash.take() {
            self.server_hash = Some(hash);
        }
    }

    pub fn forget_server_copy(&mut self) {
        self.server_hash = None;
    }

    pub fn save_as_path(&mut self, path: PathBuf) -> Result<(), String> {
        let previous = self.path.replace(path);
        let saved = self.save();
        if saved.is_err() {
            self.path = previous;
        }
        saved
    }

    pub fn rename_path(&mut self, path: PathBuf, syntax: String) {
        self.path = Some(path);
        self.set_syntax(syntax);
    }

    pub fn rename_remote(&mut self, path: String) {
        if let Some((_, current)) = &mut self.remote {
            *current = path;
        }
    }

    pub fn move_path(&mut self, moved: &[(PathBuf, PathBuf)]) {
        let Some(path) = self.path.clone() else {
            return;
        };
        for (from, to) in moved {
            if let Ok(rest) = path.strip_prefix(from) {
                self.path = Some(if rest.as_os_str().is_empty() {
                    to.clone()
                } else {
                    to.join(rest)
                });
                break;
            }
        }
    }

    pub fn toggle_pin(&mut self) {
        self.pinned = !self.pinned;
    }

    pub fn pin_if_listed(&mut self, pinned: &[PathBuf]) {
        self.pinned = self.path.as_ref().is_some_and(|p| pinned.contains(p));
    }
}
