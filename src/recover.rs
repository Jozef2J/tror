use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::document::Document;

const LIMIT: usize = 16 * 1024 * 1024;
const INDEX: &str = "index.json";

#[derive(Serialize, Deserialize, Clone, PartialEq)]
pub struct Entry {
    pub id: u64,
    pub path: Option<PathBuf>,
    pub remote: Option<(String, String)>,
    pub untitled: u32,
}

#[derive(Default)]
pub struct Recovery {
    written: HashMap<u64, u64>,
    listed: Vec<Entry>,
    pub last: f64,
}

pub fn dir() -> Option<PathBuf> {
    eframe::storage_dir("Tror").map(|dir| dir.join("recover"))
}

impl Recovery {
    pub fn snapshot(&mut self, dir: &Path, docs: &[Document]) -> Result<(), String> {
        let dirty: Vec<&Document> = docs
            .iter()
            .filter(|d| d.dirty() && d.rope.len_bytes() <= LIMIT)
            .collect();
        if dirty.is_empty() && self.listed.is_empty() {
            return Ok(());
        }
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        for doc in &dirty {
            if self.written.get(&doc.id) == Some(&doc.version) {
                continue;
            }
            let file = dir.join(format!("{}.txt", doc.id));
            let part = dir.join(format!("{}.part", doc.id));
            let out = std::fs::File::create(&part).map_err(|e| e.to_string())?;
            doc.rope
                .write_to(std::io::BufWriter::new(out))
                .map_err(|e| e.to_string())?;
            std::fs::rename(&part, &file).map_err(|e| e.to_string())?;
            self.written.insert(doc.id, doc.version);
        }
        let entries: Vec<Entry> = dirty
            .iter()
            .map(|d| Entry {
                id: d.id,
                path: d.path.clone(),
                remote: d.remote.clone(),
                untitled: d.untitled,
            })
            .collect();
        for old in &self.listed {
            if !entries.iter().any(|e| e.id == old.id) {
                let _ = std::fs::remove_file(dir.join(format!("{}.txt", old.id)));
                self.written.remove(&old.id);
            }
        }
        if entries != self.listed {
            let json = serde_json::to_vec(&entries).map_err(|e| e.to_string())?;
            let part = dir.join("index.part");
            std::fs::write(&part, json).map_err(|e| e.to_string())?;
            std::fs::rename(&part, dir.join(INDEX)).map_err(|e| e.to_string())?;
            self.listed = entries;
        }
        Ok(())
    }
}

pub fn pending(dir: &Path) -> Vec<(Entry, String)> {
    let Ok(bytes) = std::fs::read(dir.join(INDEX)) else {
        return Vec::new();
    };
    let entries: Vec<Entry> = serde_json::from_slice(&bytes).unwrap_or_default();
    entries
        .into_iter()
        .filter_map(|entry| {
            let text = std::fs::read_to_string(dir.join(format!("{}.txt", entry.id))).ok()?;
            Some((entry, text))
        })
        .collect()
}

pub fn clear(dir: &Path) {
    let Ok(list) = std::fs::read_dir(dir) else {
        return;
    };
    for item in list.flatten() {
        let path = item.path();
        if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("txt" | "part" | "json")
        ) {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsaved_text_survives_and_saved_documents_are_dropped() {
        let dir = std::env::temp_dir().join(format!("tror-recover-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut docs = vec![Document::empty(7, 1), Document::empty(8, 2)];
        docs[0].insert_text("несохранённое\nвторая строка");
        let mut recovery = Recovery::default();
        recovery.snapshot(&dir, &docs).unwrap();
        let found = pending(&dir);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0.id, 7);
        assert_eq!(found[0].1, "несохранённое\nвторая строка");
        docs[1].insert_text("ещё");
        docs.remove(0);
        recovery.snapshot(&dir, &docs).unwrap();
        let found = pending(&dir);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0.id, 8);
        assert!(!dir.join("7.txt").exists());
        clear(&dir);
        assert!(pending(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
