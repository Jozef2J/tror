use super::TrorApp;

mod manager;
mod run;
mod ui;

pub(in crate::app) use manager::Manager;
pub(in crate::app) use ui::{PluginDialog, PluginPanel};

impl TrorApp {
    pub(in crate::app) fn active_target(&self) -> Option<usize> {
        (!self.docs.is_empty()).then_some(self.active)
    }

    pub(in crate::app) fn plugin_send(
        &mut self,
        plugin: usize,
        message: serde_json::Value,
        target: Option<usize>,
    ) {
        use crate::plugins::wasm::{Job, Snapshot};
        let root = self.project.root().map(std::path::Path::to_path_buf);
        let doc = target.and_then(|i| self.docs.get(i));
        let snap = match doc {
            Some(doc) => Some(Snapshot {
                rope: doc.rope(),
                sel: doc.sel,
                path: doc.location(),
                syntax: doc.syntax.clone(),
                root,
            }),
            None => root.map(|root| Snapshot {
                rope: Default::default(),
                sel: (0, 0),
                path: None,
                syntax: String::new(),
                root: Some(root),
            }),
        };
        let doc = doc.map(|d| (d.id, d.version));
        let ctx = self.ctx.clone();
        self.plugins.send(
            plugin,
            Job {
                message,
                snap,
                doc,
                wake: Box::new(move || ctx.request_repaint()),
            },
        );
    }
}
