use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Mode {
    Both,
    Up,
    Down,
}

#[derive(Clone, PartialEq, Debug)]
pub struct Step {
    pub relative: String,
    pub size: u64,
    pub mtime: Option<u64>,
    pub fresh: bool,
}

#[derive(Default, Clone, PartialEq, Debug)]
pub struct Plan {
    pub up: Vec<Step>,
    pub down: Vec<Step>,
    pub same: usize,
}

pub type Files = BTreeMap<String, (u64, Option<u64>)>;

pub fn local_files(root: &Path) -> Files {
    let mut out = Files::new();
    let mut stack = vec![(root.to_path_buf(), String::new())];
    while let Some((dir, prefix)) = stack.pop() {
        let Ok(list) = std::fs::read_dir(&dir) else {
            continue;
        };
        for item in list.flatten() {
            let name = item.file_name().to_string_lossy().into_owned();
            let relative = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            let Ok(meta) = item.metadata() else {
                continue;
            };
            if meta.is_dir() {
                stack.push((item.path(), relative));
            } else if meta.is_file() {
                let mtime = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map(|d| d.as_secs());
                out.insert(relative, (meta.len(), mtime));
            }
        }
    }
    out
}

pub fn plan(local: &Files, remote: &Files, mode: Mode, by_time: bool) -> Plan {
    let mut plan = Plan::default();
    let step = |relative: &str, (size, mtime): (u64, Option<u64>), fresh| Step {
        relative: relative.to_owned(),
        size,
        mtime,
        fresh,
    };
    for (name, here) in local {
        match remote.get(name) {
            None if mode != Mode::Down => plan.up.push(step(name, *here, true)),
            None => {}
            Some(there) => {
                let newer_here = match (here.1, there.1) {
                    (Some(a), Some(b)) if a > b + 2 => Some(true),
                    (Some(a), Some(b)) if b > a + 2 => Some(false),
                    _ => None,
                };
                let differs = here.0 != there.0 || (by_time && newer_here.is_some());
                if !differs {
                    plan.same += 1;
                    continue;
                }
                let upload = match mode {
                    Mode::Up => true,
                    Mode::Down => false,
                    Mode::Both => newer_here.unwrap_or(true),
                };
                if upload {
                    plan.up.push(step(name, *here, false));
                } else {
                    plan.down.push(step(name, *there, false));
                }
            }
        }
    }
    if mode != Mode::Up {
        for (name, there) in remote {
            if !local.contains_key(name) {
                plan.down.push(step(name, *there, true));
            }
        }
    }
    plan
}

pub struct Dialog {
    pub remote_root: String,
    pub local_root: PathBuf,
    pub local: Files,
    pub remote: Files,
    pub mode: Mode,
    pub by_time: bool,
}

impl Dialog {
    pub fn plan(&self) -> Plan {
        plan(&self.local, &self.remote, self.mode, self.by_time)
    }
}

pub fn local_path(root: &Path, relative: &str) -> Option<PathBuf> {
    let mut path = root.to_path_buf();
    for part in relative.split('/') {
        if part.is_empty() || part == "." || part == ".." || part.contains(['\\', ':', '\0']) {
            return None;
        }
        path.push(part);
    }
    Some(path)
}

pub fn set_mtime(path: &Path, mtime: u64) {
    if let Ok(file) = std::fs::File::options().write(true).open(path) {
        let _ = file.set_modified(UNIX_EPOCH + std::time::Duration::from_secs(mtime));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(list: &[(&str, u64, u64)]) -> Files {
        list.iter()
            .map(|(name, size, time)| ((*name).to_owned(), (*size, Some(*time))))
            .collect()
    }

    #[test]
    fn newer_side_wins_and_nothing_is_deleted() {
        let local = files(&[
            ("a.php", 10, 100),
            ("b.php", 20, 500),
            ("only-here.js", 5, 100),
            ("same.css", 7, 100),
        ]);
        let remote = files(&[
            ("a.php", 12, 900),
            ("b.php", 21, 100),
            ("sub/only-there.js", 9, 100),
            ("same.css", 7, 101),
        ]);
        let both = plan(&local, &remote, Mode::Both, true);
        let names = |steps: &[Step]| steps.iter().map(|s| s.relative.clone()).collect::<Vec<_>>();
        assert_eq!(names(&both.up), ["b.php", "only-here.js"]);
        assert_eq!(names(&both.down), ["a.php", "sub/only-there.js"]);
        assert_eq!(both.same, 1);
        let up = plan(&local, &remote, Mode::Up, true);
        assert_eq!(names(&up.up), ["a.php", "b.php", "only-here.js"]);
        assert!(up.down.is_empty());
        let down = plan(&local, &remote, Mode::Down, true);
        assert!(down.up.is_empty());
        assert_eq!(names(&down.down), ["a.php", "b.php", "sub/only-there.js"]);
    }

    #[test]
    fn server_names_cannot_escape_the_chosen_folder() {
        let root = Path::new("/work/site");
        assert_eq!(
            local_path(root, "css/app.css"),
            Some(root.join("css").join("app.css"))
        );
        for bad in [
            "../x",
            "a/../../x",
            "a//b",
            "..\\..\\x",
            "C:\\x",
            "a/./b",
            "",
        ] {
            assert_eq!(local_path(root, bad), None, "{bad}");
        }
    }

    #[test]
    fn equal_size_with_other_time_counts_only_when_asked() {
        let local = files(&[("x", 10, 100)]);
        let remote = files(&[("x", 10, 900)]);
        assert_eq!(plan(&local, &remote, Mode::Both, false).same, 1);
        assert_eq!(plan(&local, &remote, Mode::Both, true).down.len(), 1);
    }
}
