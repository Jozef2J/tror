use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use serde::Deserialize;

const TIMEOUT: Duration = Duration::from_secs(30);

const GUIDE_EN: &str = "TrorPad plugins

Plugins are written in Rust and compiled to WebAssembly. Put each plugin into its own folder here:
plugin.json + plugin.wasm, or install a zip with Plugins > Manage plugins > Install from zip.

Documentation, the SDK and a full example:
https://github.com/Jozef2J/tror/blob/main/sdk/README.md

Old-style plugins (*.json files that run an external program) still work; see sort-lines.json.
";

const GUIDE: &str = r##"Плагины TrorPad

Есть два вида плагинов.

1. Плагины на Rust (WebAssembly) — основной вид.
Плагин — это папка с файлами plugin.json и plugin.wasm. Код работает
внутри TrorPad в песочнице и получает только те разрешения, которые
указаны в plugin.json (их показывают перед установкой). Пароли плагины
не получают никогда.

Полное описание, библиотека и пример (на английском):
https://github.com/Jozef2J/tror/blob/main/sdk/README.md

{
  "id": "text-tools",
  "name": "Текстовые инструменты",
  "version": "1.0.0",
  "wasm": "plugin.wasm",
  "permissions": ["fs"],
  "events": ["saved", "opened", "changed", "activeChanged"],
  "commands": [
    { "id": "sort", "title": "Сортировать строки", "key": "Ctrl+Alt+S" }
  ],
  "panels": [
    { "id": "words", "title": "Частые слова", "dock": "left" }
  ],
  "settings": [
    { "key": "indent", "title": "Отступ", "type": "choice", "options": ["2", "4"], "default": "4" }
  ]
}

id       — уникальное имя плагина; name и version видны в окне плагинов.
commands — пункты меню «Плагины», key — необязательное сочетание клавиш.
events   — события: "saved" (файл сохранён), "opened" (файл открыт),
           "changed" (через 0,4 с после последней правки, один раз
           на версию), "activeChanged" (сменилась активная вкладка).
panels   — панели плагина; dock: "left", "bottom" или "center".
settings — настройки (type: "text", "bool", "number", "choice"),
           окно «Настройки…» в меню плагина.

permissions — разрешения:
  без разрешений — текст и выделение активного документа, панели,
           окна ввода, уведомления, собственное хранилище плагина;
  "fs"      — read_file, write_file: файлы только внутри папки проекта,
           выход за неё (в том числе по ссылкам) запрещён;
  "network" — http, get: только http и https, ожидание до 20 с,
           ответ до 10 МБ, коды 4xx/5xx возвращаются как обычный ответ;
  "hosts"   — hosts, remote_list, remote_read, remote_write: профили
           серверов (имя, протокол, адрес, папка); подключается сам
           TrorPad с паролем из хранилища ОС, неизвестный ключ SFTP
           отклоняется;
  "db"      — databases, db_query: SQL к сохранённым профилям баз,
           ответ {columns, rows, affected, truncated}.

Плагин пишется на Rust с библиотекой trorpad-plugin:

  [lib]
  crate-type = ["cdylib"]

  [dependencies]
  trorpad-plugin = { git = "https://github.com/Jozef2J/tror" }

  use trorpad_plugin::{Plugin, host};

  #[derive(Default)]
  struct Upper;

  impl Plugin for Upper {
      fn command(&mut self, _id: &str) -> Result<(), String> {
          let text = host::selected_text()?;
          host::replace_selection(&text.to_uppercase())
      }
  }

  trorpad_plugin::export!(Upper);

Сборка: rustup target add wasm32-unknown-unknown
        cargo build --release --target wasm32-unknown-unknown
Готовый .wasm из target/wasm32-unknown-unknown/release переименуйте
в plugin.wasm и положите рядом с plugin.json в отдельную папку здесь
или упакуйте оба файла в zip и установите через
«Плагины → Управление плагинами… → Установить из zip…».

Методы плагина: command (обязателен), event, answer, panel_open,
panel_event. Ответ на prompt и pick приходит в answer, нажатия
в панели — в panel_event, открытие панели — в panel_open.

Что доступно в host:
  документ  — text, selection, selected_text, info, replace,
              replace_selection, select;
  сообщения — notify, log, log_error;
  данные    — open_tab, read_file, write_file, storage_get,
              storage_set, setting;
  окна      — prompt (ввод строки), pick (выбор из списка);
  панели    — panel (содержимое), show_panel;
  пометки   — decorate(ключ, список), clear_decorations(ключ):
              Decoration::highlight (подсветка), underline (волнистое
              подчёркивание), gutter (метка на поле номеров), к каждой
              .color("#rrggbb") и .message(...) — подсказка при наведении;
              до 5000 пометок на ключ;
  с разрешениями — http, get, hosts, remote_list, remote_read,
              remote_write, databases, db_query.

Панель описывается функциями модуля ui: column, row, heading, text,
strong, small, code, button, primary, input, multiline, checkbox,
select, list, progress, separator, space, table, tree, node.
Поля ввода присылают событие по Enter или при уходе фокуса с новым
значением; флажки, списки, таблицы и деревья — сразу.
Позиции в тексте считаются в символах, а не в байтах (помогают
функции модуля text: byte_of, slice, lines_span, chars).

Ограничения: на один вызов — до 4·10⁹ единиц работы (около 8 с),
память — до 512 МБ. Каждый плагин работает в своём потоке, окно
не замирает. Все правки одной команды отменяются одним Ctrl+Z; если
файл успели изменить, правки не применяются. Если плагин зациклится
или упадёт, TrorPad остановит его до «Перечитать» и запишет причину
в журнал. Версия интерфейса — API 2: в её пределах ничего не удаляется
и не меняется несовместимо. Пример — плагин «Текстовые инструменты»
в выпусках на GitHub (trorpad-plugin-text-tools.zip).

2. Внешние программы.
Плагин — это файл .json в этой папке. Он описывает внешнюю программу:
TrorPad передаёт ей текст через стандартный ввод и забирает результат
из стандартного вывода. Программа может быть на чём угодно: PHP, Python,
Node, shell, готовая утилита.

{
  "name": "Форматировать JSON",
  "command": "python3",
  "args": ["-m", "json.tool", "--no-ensure-ascii"],
  "input": "selection",
  "output": "replace",
  "shortcut": "Ctrl+Alt+J"
}

input   — что получает программа:
          "selection" — выделенный текст, а если ничего не выделено, весь файл;
          "document"  — весь файл;
          "none"      — ничего.
output  — что делать с результатом:
          "replace"   — заменить то, что было передано;
          "insert"    — вставить в позицию курсора;
          "new_tab"   — открыть в новой вкладке;
          "message"   — показать первую строку в уведомлении, всё остальное — в журнале;
          "none"      — ничего.
shortcut — необязательное сочетание клавиш: Ctrl, Alt, Shift и клавиша.

В аргументах подставляются {file}, {line}, {column}, {syntax}.
Те же значения доступны программе в переменных окружения
TROR_FILE, TROR_LINE, TROR_COLUMN, TROR_SYNTAX.
Текст передаётся и принимается в UTF-8. Программа запускается в папке файла.
Если программа завершилась с ошибкой, текст не меняется, а её сообщение
попадает в журнал. На выполнение даётся 30 секунд.

После изменения файлов выберите «Плагины → Перечитать».
"##;

const EXAMPLE: &str = r#"{
  "name": "Сортировать строки (пример)",
  "command": "sort",
  "args": [],
  "input": "selection",
  "output": "replace"
}
"#;

#[derive(Deserialize, Clone, Copy, PartialEq, Debug, Default)]
#[serde(rename_all = "snake_case")]
pub enum Input {
    #[default]
    Selection,
    Document,
    None,
}

#[derive(Deserialize, Clone, Copy, PartialEq, Debug, Default)]
#[serde(rename_all = "snake_case")]
pub enum Output {
    #[default]
    Replace,
    Insert,
    NewTab,
    Message,
    None,
}

#[derive(Deserialize, Clone, Debug)]
pub struct Plugin {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub input: Input,
    #[serde(default)]
    pub output: Output,
    #[serde(default)]
    pub shortcut: String,
}

pub struct Context {
    pub file: String,
    pub dir: Option<PathBuf>,
    pub line: usize,
    pub column: usize,
    pub syntax: String,
}

pub struct Finished {
    pub plugin: Plugin,
    pub doc: u64,
    pub version: u64,
    pub range: std::ops::Range<usize>,
    pub result: Result<String, String>,
    pub stderr: String,
}

pub struct Plugins {
    pub dir: Option<PathBuf>,
    pub list: Vec<Plugin>,
    pub wasm: Vec<crate::wasm::Worker>,
    done: Receiver<crate::wasm::Done>,
    pub errors: Vec<String>,
    pub off: Vec<String>,
    pub running: usize,
    tx: Sender<Finished>,
    rx: Receiver<Finished>,
}

pub fn load(dir: &Path) -> (Vec<Plugin>, Vec<String>) {
    let mut list = Vec::new();
    let mut errors = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return (list, errors);
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("json"))
        })
        .collect();
    files.sort();
    for file in files {
        let name = file
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let parsed = std::fs::read_to_string(&file)
            .map_err(|e| e.to_string())
            .and_then(|text| serde_json::from_str::<Plugin>(&text).map_err(|e| e.to_string()));
        match parsed {
            Ok(plugin) if plugin.name.trim().is_empty() || plugin.command.trim().is_empty() => {
                errors.push(tf!(
                    "{name}: нужны поля name и command",
                    "{name}: the name and command fields are required"
                ));
            }
            Ok(plugin) => list.push(plugin),
            Err(e) => errors.push(format!("{name}: {e}")),
        }
    }
    (list, errors)
}

pub fn peek_manifest(archive: &Path) -> Result<crate::wasm::Manifest, String> {
    let files = unzip(archive)?;
    let json = files
        .iter()
        .filter(|(name, _)| name == "plugin.json" || name.ends_with("/plugin.json"))
        .min_by_key(|(name, _)| name.len())
        .map(|(_, data)| data.clone())
        .ok_or(tr!("в архиве нет plugin.json"))?;
    serde_json::from_slice(&json).map_err(|e| format!("plugin.json: {e}"))
}

pub(crate) fn unzip(archive: &Path) -> Result<Vec<(String, Vec<u8>)>, String> {
    use std::io::Read;
    let data = std::fs::read(archive).map_err(|e| e.to_string())?;
    if data.len() > 64 << 20 {
        return Err(tr!("архив больше 64 МБ").into());
    }
    let u16_at = |i: usize| -> Result<usize, String> {
        data.get(i..i + 2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]) as usize)
            .ok_or_else(|| tr!("повреждённый архив").to_owned())
    };
    let u32_at = |i: usize| -> Result<usize, String> {
        data.get(i..i + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
            .ok_or_else(|| tr!("повреждённый архив").to_owned())
    };
    let end = (0..data.len().saturating_sub(21))
        .rev()
        .find(|&i| data[i..].starts_with(&[0x50, 0x4b, 0x05, 0x06]))
        .ok_or(tr!("это не zip-архив"))?;
    let count = u16_at(end + 10)?;
    let mut at = u32_at(end + 16)?;
    let mut out = Vec::new();
    let mut total = 0usize;
    for _ in 0..count {
        if u32_at(at)? != 0x0201_4b50 {
            return Err(tr!("повреждённый архив").into());
        }
        let method = u16_at(at + 10)?;
        let packed = u32_at(at + 20)?;
        let size = u32_at(at + 24)?;
        let name_len = u16_at(at + 28)?;
        let extra = u16_at(at + 30)?;
        let comment = u16_at(at + 32)?;
        let local = u32_at(at + 42)?;
        let name = String::from_utf8_lossy(
            data.get(at + 46..at + 46 + name_len)
                .ok_or(tr!("повреждённый архив"))?,
        )
        .replace('\\', "/");
        at += 46 + name_len + extra + comment;
        if name.ends_with('/') {
            continue;
        }
        total += size;
        if total > 256 << 20 {
            return Err(tr!("распакованный архив больше 256 МБ").into());
        }
        let start = local + 30 + u16_at(local + 26)? + u16_at(local + 28)?;
        let raw = data
            .get(start..start + packed)
            .ok_or(tr!("повреждённый архив"))?;
        let bytes = match method {
            0 => raw.to_vec(),
            8 => {
                let mut buf = Vec::with_capacity(size);
                flate2::read::DeflateDecoder::new(raw)
                    .take(size as u64 + 1)
                    .read_to_end(&mut buf)
                    .map_err(|e| e.to_string())?;
                buf
            }
            other => {
                return Err(tf!(
                    "{name}: неподдерживаемое сжатие {other}",
                    "{name}: unsupported compression {other}"
                ));
            }
        };
        out.push((name, bytes));
    }
    Ok(out)
}

#[cfg(windows)]
fn quiet(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    command.creation_flags(0x0800_0000);
}

#[cfg(not(windows))]
fn quiet(_command: &mut Command) {}

pub fn execute(
    plugin: &Plugin,
    context: &Context,
    input: &str,
) -> (Result<String, String>, String) {
    let fill = |arg: &String| {
        arg.replace("{file}", &context.file)
            .replace("{line}", &context.line.to_string())
            .replace("{column}", &context.column.to_string())
            .replace("{syntax}", &context.syntax)
    };
    let mut command = Command::new(plugin.command.trim());
    quiet(&mut command);
    command
        .args(plugin.args.iter().map(fill))
        .env("TROR_FILE", &context.file)
        .env("TROR_LINE", context.line.to_string())
        .env("TROR_COLUMN", context.column.to_string())
        .env("TROR_SYNTAX", &context.syntax)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(dir) = &context.dir {
        command.current_dir(dir);
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(e) => {
            return (
                Err(tf!(
                    "не удалось запустить «{}»: {e}",
                    "could not start \"{}\": {e}",
                    plugin.command
                )),
                String::new(),
            );
        }
    };
    let mut stdin = child.stdin.take();
    let data = input.as_bytes().to_vec();
    let writer = std::thread::spawn(move || {
        if let Some(pipe) = &mut stdin {
            let _ = pipe.write_all(&data);
        }
    });
    let mut pipes = (child.stdout.take(), child.stderr.take());
    let out = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(pipe) = &mut pipes.0 {
            let _ = pipe.read_to_end(&mut bytes);
        }
        bytes
    });
    let err = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(pipe) = &mut pipes.1 {
            let _ = pipe.read_to_end(&mut bytes);
        }
        bytes
    });
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if started.elapsed() > TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(tr!("не уложился в 30 секунд и был остановлен").to_owned());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(15)),
            Err(e) => break Err(e.to_string()),
        }
    };
    let _ = writer.join();
    let stdout = String::from_utf8_lossy(&out.join().unwrap_or_default()).into_owned();
    let stderr = String::from_utf8_lossy(&err.join().unwrap_or_default())
        .trim()
        .to_owned();
    let result = match status {
        Ok(status) if status.success() => Ok(stdout),
        Ok(status) => Err(match stderr.lines().next() {
            Some(line) => line.to_owned(),
            None => tf!(
                "завершился с кодом {}",
                "exited with code {}",
                status.code().unwrap_or(-1)
            ),
        }),
        Err(e) => Err(e),
    };
    (result, stderr)
}

impl Plugins {
    pub fn new(dir: Option<PathBuf>, off: Vec<String>) -> Self {
        let (tx, rx) = channel();
        let done = channel().1;
        let mut plugins = Self {
            dir,
            list: Vec::new(),
            wasm: Vec::new(),
            done,
            errors: Vec::new(),
            off,
            running: 0,
            tx,
            rx,
        };
        plugins.reload();
        plugins
    }

    pub fn reload(&mut self) {
        let (list, mut errors) = self.dir.as_deref().map(load).unwrap_or_default();
        let (wasm, wasm_errors) = self
            .dir
            .as_deref()
            .map(|d| crate::wasm::load_enabled(d, &self.off))
            .unwrap_or_default();
        errors.extend(wasm_errors);
        let (done_tx, done_rx) = channel();
        self.list = list;
        self.wasm = wasm
            .into_iter()
            .enumerate()
            .map(|(i, loaded)| crate::wasm::Worker::spawn(i, loaded, done_tx.clone()))
            .collect();
        self.done = done_rx;
        self.errors = errors;
    }

    pub fn finished(&mut self) -> Vec<crate::wasm::Done> {
        let done: Vec<crate::wasm::Done> = self.done.try_iter().collect();
        for item in &done {
            if let Some(worker) = self.wasm.get_mut(item.plugin) {
                worker.busy = worker.busy.saturating_sub(1);
                if item.outcome.broken {
                    worker.broken = item.outcome.error.clone();
                }
            }
        }
        done
    }

    pub fn install(&mut self, archive: &Path) -> Result<crate::wasm::Manifest, String> {
        let dir = self.dir.clone().ok_or(tr!("нет папки данных программы"))?;
        let files = unzip(archive)?;
        let (prefix, json) = files
            .iter()
            .filter(|(name, _)| name == "plugin.json" || name.ends_with("/plugin.json"))
            .min_by_key(|(name, _)| name.len())
            .map(|(name, data)| {
                (
                    name.trim_end_matches("plugin.json").to_owned(),
                    data.clone(),
                )
            })
            .ok_or(tr!("в архиве нет plugin.json"))?;
        let manifest: crate::wasm::Manifest =
            serde_json::from_slice(&json).map_err(|e| format!("plugin.json: {e}"))?;
        let safe = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.';
        if manifest.id.is_empty() || !manifest.id.chars().all(safe) || manifest.id.starts_with('.')
        {
            return Err(tr!("недопустимый id плагина").into());
        }
        let target = dir.join(&manifest.id);
        let staging = dir.join(format!(".{}-new", manifest.id));
        let _ = std::fs::remove_dir_all(&staging);
        for (name, data) in &files {
            let Some(rest) = name.strip_prefix(&prefix) else {
                continue;
            };
            if rest.is_empty() || rest.ends_with('/') {
                continue;
            }
            let relative = Path::new(rest);
            if relative
                .components()
                .any(|c| !matches!(c, std::path::Component::Normal(_)))
            {
                return Err(tf!(
                    "недопустимый путь в архиве: {name}",
                    "invalid path in the archive: {name}"
                ));
            }
            let out = staging.join(relative);
            if let Some(parent) = out.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            std::fs::write(&out, data).map_err(|e| e.to_string())?;
        }
        if !staging.join(&manifest.wasm).is_file() {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(tf!(
                "в архиве нет {}",
                "the archive has no {}",
                manifest.wasm
            ));
        }
        for keep in ["storage.json", "settings.json"] {
            let old = target.join(keep);
            if old.is_file() {
                let _ = std::fs::copy(&old, staging.join(keep));
            }
        }
        let _ = std::fs::remove_dir_all(&target);
        std::fs::rename(&staging, &target).map_err(|e| e.to_string())?;
        self.reload();
        Ok(manifest)
    }

    pub fn remove(&mut self, folder: &Path) -> Result<(), String> {
        let dir = self.dir.clone().ok_or(tr!("нет папки данных программы"))?;
        if folder.parent() != Some(dir.as_path()) {
            return Err(tr!("папка вне каталога плагинов").into());
        }
        std::fs::remove_dir_all(folder).map_err(|e| e.to_string())?;
        self.reload();
        Ok(())
    }

    pub fn busy(&self) -> Option<&str> {
        self.wasm
            .iter()
            .find(|w| w.busy > 0)
            .map(|w| w.manifest.name.as_str())
    }

    pub fn count(&self) -> usize {
        self.list.len() + self.wasm.len()
    }

    pub fn prepare(&mut self) -> Result<PathBuf, String> {
        let dir = self.dir.clone().ok_or(tr!("нет папки данных программы"))?;
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let (name, text) = if crate::i18n::english() {
            ("HOW-TO-WRITE-PLUGINS.txt", GUIDE_EN)
        } else {
            ("КАК-ПИСАТЬ-ПЛАГИНЫ.txt", GUIDE)
        };
        let guide = dir.join(name);
        if std::fs::read_to_string(&guide).ok().as_deref() != Some(text) {
            std::fs::write(&guide, text).map_err(|e| e.to_string())?;
        }
        let empty = load(&dir).0.is_empty() && crate::wasm::load_all(&dir).0.is_empty();
        let example = dir.join("sort-lines.json");
        if empty && !example.exists() {
            std::fs::write(&example, EXAMPLE).map_err(|e| e.to_string())?;
        }
        self.reload();
        Ok(dir)
    }

    pub fn start(
        &mut self,
        plugin: Plugin,
        context: Context,
        input: String,
        (doc, version): (u64, u64),
        range: std::ops::Range<usize>,
        wake: impl Fn() + Send + 'static,
    ) {
        let tx = self.tx.clone();
        self.running += 1;
        std::thread::spawn(move || {
            let (result, stderr) = execute(&plugin, &context, &input);
            let _ = tx.send(Finished {
                plugin,
                doc,
                version,
                range,
                result,
                stderr,
            });
            wake();
        });
    }

    pub fn poll(&mut self) -> Vec<Finished> {
        let done: Vec<Finished> = self.rx.try_iter().collect();
        self.running = self.running.saturating_sub(done.len());
        done
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stored_zip(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();
        for (name, data) in files {
            let offset = out.len() as u32;
            let local = |buf: &mut Vec<u8>, sig: u32| {
                buf.extend(sig.to_le_bytes());
            };
            local(&mut out, 0x0403_4b50);
            out.extend([20u8, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
            out.extend(0u32.to_le_bytes());
            out.extend((data.len() as u32).to_le_bytes());
            out.extend((data.len() as u32).to_le_bytes());
            out.extend((name.len() as u16).to_le_bytes());
            out.extend(0u16.to_le_bytes());
            out.extend(name.as_bytes());
            out.extend(*data);
            local(&mut central, 0x0201_4b50);
            central.extend([20u8, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
            central.extend(0u32.to_le_bytes());
            central.extend((data.len() as u32).to_le_bytes());
            central.extend((data.len() as u32).to_le_bytes());
            central.extend((name.len() as u16).to_le_bytes());
            central.extend([0u8; 12]);
            central.extend(offset.to_le_bytes());
            central.extend(name.as_bytes());
        }
        let start = out.len() as u32;
        let size = central.len() as u32;
        out.extend(central);
        out.extend(0x0605_4b50u32.to_le_bytes());
        out.extend([0u8; 4]);
        out.extend((files.len() as u16).to_le_bytes());
        out.extend((files.len() as u16).to_le_bytes());
        out.extend(size.to_le_bytes());
        out.extend(start.to_le_bytes());
        out.extend(0u16.to_le_bytes());
        out
    }

    #[test]
    fn plugin_zip_is_installed_and_traversal_rejected() {
        let dir = std::env::temp_dir().join(format!("tror-zip-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = br#"{"id":"demo","name":"Demo","permissions":["network"]}"#;
        let good = dir.join("good.zip");
        std::fs::write(
            &good,
            stored_zip(&[
                ("demo/plugin.json", manifest),
                ("demo/plugin.wasm", b"\0asm"),
            ]),
        )
        .unwrap();
        assert_eq!(
            peek_manifest(&good).unwrap().permissions,
            vec!["network".to_owned()]
        );
        let mut plugins = Plugins::new(Some(dir.join("plugins")), Vec::new());
        std::fs::create_dir_all(dir.join("plugins")).unwrap();
        plugins.install(&good).unwrap();
        assert!(dir.join("plugins/demo/plugin.wasm").is_file());
        let bad = dir.join("bad.zip");
        std::fs::write(
            &bad,
            stored_zip(&[
                ("plugin.json", manifest),
                ("plugin.wasm", b"x"),
                ("../evil", b"x"),
            ]),
        )
        .unwrap();
        assert!(
            plugins
                .install(&bad)
                .unwrap_err()
                .contains("недопустимый путь")
        );
        assert!(!dir.join("plugins/evil").exists() && !dir.join("evil").exists());
        plugins.remove(&dir.join("plugins/demo")).unwrap();
        assert!(!dir.join("plugins/demo").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn context() -> Context {
        Context {
            file: "/tmp/a b.php".into(),
            dir: None,
            line: 7,
            column: 3,
            syntax: "PHP".into(),
        }
    }

    #[test]
    fn manifest_parses_with_defaults() {
        let plugin: Plugin = serde_json::from_str(EXAMPLE).unwrap();
        assert_eq!(plugin.command, "sort");
        assert_eq!(
            (plugin.input, plugin.output),
            (Input::Selection, Output::Replace)
        );
        let plugin: Plugin = serde_json::from_str(
            r#"{"name":"x","command":"php","args":["-l","{file}"],"input":"none","output":"message","shortcut":"Ctrl+Alt+L"}"#,
        )
        .unwrap();
        assert_eq!(
            (plugin.input, plugin.output),
            (Input::None, Output::Message)
        );
        assert!(crate::keys::parse(&plugin.shortcut).is_some());
        assert!(serde_json::from_str::<Plugin>(r#"{"name":"x"}"#).is_err());
    }

    #[test]
    fn folder_is_prepared_and_loaded() {
        let dir = std::env::temp_dir().join(format!("tror_plugins_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut plugins = Plugins::new(Some(dir.clone()), Vec::new());
        assert!(plugins.list.is_empty());
        plugins.prepare().unwrap();
        assert_eq!(plugins.list.len(), 1);
        std::fs::write(dir.join("broken.json"), "{oops").unwrap();
        std::fs::write(dir.join("empty.json"), r#"{"name":" ","command":"x"}"#).unwrap();
        plugins.reload();
        assert_eq!(plugins.list.len(), 1);
        assert_eq!(plugins.errors.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn program_receives_text_and_environment() {
        let plugin = |command: &str, args: &[&str]| Plugin {
            name: "t".into(),
            command: command.into(),
            args: args.iter().map(|a| (*a).to_owned()).collect(),
            input: Input::Selection,
            output: Output::Replace,
            shortcut: String::new(),
        };
        let (result, _) = execute(&plugin("sort", &[]), &context(), "б\nа\nв\n");
        assert_eq!(result.unwrap(), "а\nб\nв\n");
        let (result, _) = execute(
            &plugin(
                "sh",
                &["-c", "echo \"$TROR_LINE:$TROR_SYNTAX:$1\"", "x", "{file}"],
            ),
            &context(),
            "",
        );
        assert_eq!(result.unwrap(), "7:PHP:/tmp/a b.php\n");
        let (result, stderr) = execute(
            &plugin("sh", &["-c", "echo bad >&2; exit 3"]),
            &context(),
            "keep",
        );
        assert_eq!(result.unwrap_err(), "bad");
        assert_eq!(stderr, "bad");
        let big = "строка\n".repeat(200_000);
        let (result, _) = execute(&plugin("cat", &[]), &context(), &big);
        assert_eq!(result.unwrap().len(), big.len());
        assert!(
            execute(&plugin("tror-no-such-program", &[]), &context(), "")
                .0
                .is_err()
        );
    }
}
