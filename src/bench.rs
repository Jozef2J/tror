use std::time::{Duration, Instant};

use crate::document::Document;
use crate::highlight::{Highlighter, Syntaxes};
use crate::search::SearchState;

struct Case {
    name: &'static str,
    limit_ms: f64,
    took: Duration,
}

fn median(mut runs: Vec<Duration>) -> Duration {
    runs.sort();
    runs[runs.len() / 2]
}

fn time<T>(rounds: usize, mut f: impl FnMut() -> T) -> Duration {
    let runs = (0..rounds)
        .map(|_| {
            let start = Instant::now();
            std::hint::black_box(f());
            start.elapsed()
        })
        .collect();
    median(runs)
}

fn sample(lines: usize) -> String {
    let mut text = String::with_capacity(lines * 64);
    for i in 0..lines {
        text.push_str(&format!(
            "<?php $item_{i} = ['id' => {i}, 'name' => \"Товар {i}\", 'price' => {}]; // строка\n",
            i % 997
        ));
    }
    text
}

pub fn run() -> i32 {
    let dir = std::env::temp_dir().join(format!("tror-bench-{}", std::process::id()));
    if std::fs::create_dir_all(&dir).is_err() {
        return 2;
    }
    let big = sample(300_000);
    let file = dir.join("big.php");
    if std::fs::write(&file, &big).is_err() {
        return 2;
    }
    let mut cases = Vec::new();
    let mut add = |name, limit_ms, took| {
        cases.push(Case {
            name,
            limit_ms,
            took,
        })
    };

    add("Load syntax definitions", 300.0, time(3, Syntaxes::new));
    add(
        "Open a 25 MB file",
        400.0,
        time(3, || Document::open(1, &file).map(|d| d.line_count())),
    );
    let doc = match Document::open(1, &file) {
        Ok(doc) => doc,
        Err(_) => return 2,
    };
    add(
        "Regex search in 25 MB",
        600.0,
        time(3, || {
            let mut search = SearchState::default();
            search.query = r"Товар \d+7\b".into();
            search.regex = true;
            search.update(&doc, None);
            search.matches.len()
        }),
    );
    let mut edit = doc;
    add(
        "10,000 inserts in the middle of 25 MB",
        300.0,
        time(1, || {
            let middle = edit.len() / 2;
            edit.sel = (middle, middle);
            for _ in 0..10_000 {
                edit.insert_text("x");
            }
        }),
    );
    let syn = Syntaxes::new();
    let php = sample(5_000);
    let rope = ropey::Rope::from_str(&php);
    add(
        "Highlight 5,000 lines of PHP",
        2500.0,
        time(3, || {
            let mut hl = Highlighter::default();
            hl.process(&syn, &rope, "PHP", 1, 0..5_000, None)
        }),
    );
    let mut html = String::from("<html><body>");
    for i in 0..20_000 {
        html.push_str(&format!(
            "<h2>Раздел {i}</h2><p>Текст <b>жирный</b> и <a href=\"/p/{i}\">ссылка</a>.</p><ul><li>пункт</li></ul>"
        ));
    }
    html.push_str("</body></html>");
    add(
        "Visual editor: parse and write 2 MB of HTML",
        400.0,
        time(3, || {
            let parsed = crate::visual::model::parse(&html);
            crate::visual::model::serialize(&parsed).len()
        }),
    );
    let _ = std::fs::remove_dir_all(&dir);
    println!("### TrorPad {} speed\n", env!("CARGO_PKG_VERSION"));
    println!("| Operation | Time, ms | Limit, ms | |");
    println!("|---|---:|---:|---|");
    let mut failed = 0;
    for case in &cases {
        let ms = case.took.as_secs_f64() * 1000.0;
        let ok = ms <= case.limit_ms;
        if !ok {
            failed += 1;
        }
        println!(
            "| {} | {:.1} | {:.0} | {} |",
            case.name,
            ms,
            case.limit_ms,
            if ok { "✅" } else { "❌" }
        );
    }
    i32::from(failed > 0)
}
