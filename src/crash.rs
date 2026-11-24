use std::path::PathBuf;

pub fn dir() -> Option<PathBuf> {
    eframe::storage_dir("Tror").map(|d| d.join("crashes"))
}

pub fn install() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(dir) = dir()
            && std::fs::create_dir_all(&dir).is_ok()
        {
            let stamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S");
            let thread = std::thread::current();
            let text = format!(
                "{}\n\nTrorPad {} · {} {}\n{}\nthread: {}\n\n{info}\n\n{}\n\n--- log ---\n{}\n",
                tr!(
                    "Отчёт о сбое TrorPad. Перед отправкой проверьте журнал внизу: в нём могут быть имена серверов и запросы к базам. Пароли в журнал не пишутся."
                ),
                env!("CARGO_PKG_VERSION"),
                std::env::consts::OS,
                std::env::consts::ARCH,
                stamp,
                thread.name().unwrap_or("?"),
                std::backtrace::Backtrace::force_capture(),
                crate::trace::recent().join("\n"),
            );
            let _ = std::fs::write(dir.join(format!("crash-{stamp}.txt")), text);
        }
        previous(info);
    }));
}

pub fn pending() -> Option<PathBuf> {
    let dir = dir()?;
    let mut found: Vec<PathBuf> = std::fs::read_dir(&dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("crash-") && n.ends_with(".txt"))
        })
        .collect();
    found.sort();
    let newest = found.pop()?;
    for path in found {
        let _ = std::fs::rename(&path, seen(&path));
    }
    let shown = seen(&newest);
    std::fs::rename(&newest, &shown).ok()?;
    Some(shown)
}

fn seen(path: &std::path::Path) -> PathBuf {
    let dir = path.with_file_name("shown");
    let _ = std::fs::create_dir_all(&dir);
    dir.join(path.file_name().unwrap_or_default())
}
