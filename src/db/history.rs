use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
pub(super) struct History {
    entries: Vec<(String, String)>,
    #[serde(skip)]
    path: Option<std::path::PathBuf>,
}

pub(super) const HISTORY_MAX: usize = 300;

impl History {
    pub(super) fn load(path: Option<std::path::PathBuf>) -> Self {
        let mut history: History = path
            .as_deref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        history.path = path;
        history
    }

    pub(super) fn push(&mut self, profile: &str, sql: &str) {
        self.entries.retain(|(p, s)| !(p == profile && s == sql));
        self.entries.push((profile.to_owned(), sql.to_owned()));
        let extra = self.entries.len().saturating_sub(HISTORY_MAX);
        self.entries.drain(..extra);
        if let Some(path) = &self.path
            && let Ok(text) = serde_json::to_string(self)
        {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = crate::remote::write_private(path, text.as_bytes());
        }
    }

    pub(super) fn recent(&self, profile: &str) -> impl Iterator<Item = &str> {
        self.entries
            .iter()
            .rev()
            .filter(move |(p, _)| p == profile)
            .map(|(_, s)| s.as_str())
    }
}

#[cfg(test)]
mod history_tests {
    use super::*;

    #[test]
    fn history_dedupes_limits_and_persists() {
        let dir = std::env::temp_dir().join(format!("tror-history-{}", std::process::id()));
        let path = dir.join("db_history.json");
        let mut history = History::load(Some(path.clone()));
        history.push("a", "SELECT 1");
        history.push("b", "SELECT 2");
        history.push("a", "SELECT 3");
        history.push("a", "SELECT 1");
        assert_eq!(history.recent("a").collect::<Vec<_>>(), vec!["SELECT 1", "SELECT 3"]);
        for i in 0..HISTORY_MAX + 5 {
            history.push("c", &format!("SELECT {i}"));
        }
        assert_eq!(history.entries.len(), HISTORY_MAX);
        let loaded = History::load(Some(path));
        assert_eq!(loaded.entries.len(), HISTORY_MAX);
        assert_eq!(loaded.recent("c").next(), Some(format!("SELECT {}", HISTORY_MAX + 4).as_str()));
        std::fs::remove_dir_all(dir).ok();
    }
}
