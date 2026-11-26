use std::collections::BTreeMap;
use crate::widgets::Lit;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align2, Button, ComboBox, FontId, Key, Modifiers, RichText, ScrollArea, Sense, Stroke,
    TextEdit, Ui,
};

use crate::widgets::{dialog, dialog_button};
use crate::remote::{self, Profile, Store};
use crate::theme::Palette;
use crate::trace;

mod dialogs;
mod engine;
mod grid;
mod search;
mod history;
mod sql;
mod structure;

pub use crate::remote::{DbKind, DbProfile};
pub use engine::{MAX_ROWS, Source, Table};
use engine::*;
use history::History;
use sql::*;

enum Cmd {
    Query(String, Option<Source>),
    Databases,
    Tables(String),
    Batch(Vec<String>, String, Option<Source>),
    Import(Vec<String>),
    Search(Find),
    Structure(Source),
    Alter(Vec<String>, Source),
}

#[derive(Clone, Default)]
struct Find {
    schema: String,
    table: Option<String>,
    needle: String,
    case: bool,
    exact: bool,
}

impl Find {
    fn condition(&self, kind: DbKind, columns: &[String]) -> String {
        let pattern = if self.exact {
            self.needle.clone()
        } else {
            let escaped = self
                .needle
                .replace('!', "!!")
                .replace('%', "!%")
                .replace('_', "!_");
            format!("%{escaped}%")
        };
        let value = kind.literal(Some(&pattern));
        columns
            .iter()
            .map(|c| {
                let name = kind.quote(c);
                let text = match kind {
                    DbKind::MySql => format!("CAST({name} AS CHAR)"),
                    DbKind::Postgres => format!("{name}::text"),
                    DbKind::Sqlite => format!("CAST({name} AS TEXT)"),
                };
                match (self.exact, self.case, kind) {
                    (true, true, DbKind::MySql) => format!("BINARY {text} = {value}"),
                    (true, false, _) => format!("LOWER({text}) = LOWER({value})"),
                    (true, true, _) => format!("{text} = {value}"),
                    (false, true, DbKind::MySql) => {
                        format!("{text} LIKE BINARY {value} ESCAPE '!'")
                    }
                    (false, false, DbKind::Postgres) => format!("{text} ILIKE {value} ESCAPE '!'"),
                    (false, true, DbKind::Sqlite) => {
                        format!(
                            "({text} LIKE {value} ESCAPE '!' AND INSTR({text}, {}) > 0)",
                            kind.literal(Some(&self.needle))
                        )
                    }
                    (false, false, DbKind::MySql) => {
                        format!("LOWER({text}) LIKE LOWER({value}) ESCAPE '!'")
                    }
                    _ => format!("{text} LIKE {value} ESCAPE '!'"),
                }
            })
            .collect::<Vec<_>>()
            .join(" OR ")
    }
}

type Hits = Vec<(String, u64, String)>;

enum Event {
    Connected(String),
    Databases(Vec<String>),
    Tables(String, Vec<String>),
    Result(Table, Option<Source>, Vec<String>),
    Imported(usize, u64),
    Found(Hits, usize),
    Structure(Box<structure::Structure>),
    Error(String),
    Closed(String),
}

struct Conn {
    tx: Sender<Cmd>,
    rx: Receiver<Event>,
}

pub fn one_shot(store: &Store, name: &str, sql: &str) -> Result<serde_json::Value, String> {
    let profile = store
        .databases
        .iter()
        .find(|p| p.name == name)
        .cloned()
        .ok_or_else(|| tf!("подключение «{name}» не найдено", "connection \"{name}\" not found"))?;
    let mut ssh = None;
    if !profile.tunnel.is_empty() && profile.kind != DbKind::Sqlite {
        let host = store
            .profiles
            .iter()
            .find(|p| p.name == profile.tunnel)
            .ok_or_else(|| tf!("хост «{}» для туннеля не найден", "host \"{}\" for the tunnel not found", profile.tunnel))?;
        let key = format!("{}:{}", host.host.trim(), host.port);
        ssh = Some((host.clone(), store.known_hosts.get(&key).cloned()));
    }
    let (mut engine, _, _tunnel) = link(&profile, &ssh)?;
    let table = engine.query(sql)?;
    Ok(serde_json::json!({
        "columns": table.columns,
        "rows": table.rows,
        "affected": table.affected,
        "truncated": table.truncated,
    }))
}

fn test_profile(profile: DbProfile, ssh: Ssh) -> Result<String, String> {
    let (mut engine, version, _tunnel) = link(&profile, &ssh)?;
    engine.query("SELECT 1")?;
    Ok(version)
}

type Ssh = Option<(Profile, Option<String>)>;

fn link(profile: &DbProfile, ssh: &Ssh) -> Result<(Engine, String, Option<remote::Tunnel>), String> {
    let mut host = profile.host.trim().to_owned();
    let mut port = profile.port;
    let mut tunnel = None;
    if profile.kind != DbKind::Sqlite
        && let Some((ssh, known)) = ssh
    {
        let open = remote::tunnel(ssh, known.as_deref(), &host, port)
            .map_err(|e| tf!("туннель: {e}", "tunnel: {e}"))?;
        host = "127.0.0.1".into();
        port = open.port;
        tunnel = Some(open);
    }
    let (engine, version) = open_engine(profile, host, port)?;
    Ok((engine, version, tunnel))
}

const KEEPALIVE: Duration = Duration::from_secs(25);

fn serve(profile: DbProfile, ssh: Ssh, cmds: Receiver<Cmd>, events: Sender<Event>, wake: &dyn Fn()) {
    let send = |event: Event| {
        let _ = events.send(event);
        wake();
    };
    let target = match profile.kind {
        DbKind::Sqlite => format!("sqlite://{}", profile.file.trim()),
        DbKind::MySql => format!(
            "mysql://{}@{}:{}/{}",
            profile.user.trim(),
            profile.host.trim(),
            profile.port,
            profile.database.trim()
        ),
        DbKind::Postgres => format!(
            "postgres://{}@{}:{}/{}",
            profile.user.trim(),
            profile.host.trim(),
            profile.port,
            profile.database.trim()
        ),
    };
    trace::line(format!("[DB] connecting to {target}"), false);
    let (mut engine, version, mut tunnel) = match link(&profile, &ssh) {
        Ok(ok) => ok,
        Err(e) => {
            trace::line(format!("[DB] {e}"), true);
            send(Event::Closed(e));
            return;
        }
    };
    trace::server(format!("[DB] connected, {version}"));
    send(Event::Connected(version));
    let mut used: Option<String> = None;
    let mut last = Instant::now();
    let mut lost = false;
    let revive = |engine: &mut Engine, tunnel: &mut Option<remote::Tunnel>, used: &Option<String>| {
        let (fresh, _, open) = link(&profile, &ssh)?;
        *engine = fresh;
        *tunnel = open;
        if let Some(sql) = used {
            let _ = engine.query(sql);
        }
        trace::line("[DB] reconnected", false);
        Ok::<(), String>(())
    };
    loop {
        let cmd = match cmds.recv_timeout(KEEPALIVE) {
            Ok(cmd) => cmd,
            Err(RecvTimeoutError::Timeout) => {
                if !lost && !engine.alive() {
                    trace::line("[DB] connection lost", true);
                    lost = revive(&mut engine, &mut tunnel, &used).is_err();
                }
                continue;
            }
            Err(RecvTimeoutError::Disconnected) => break,
        };
        if (lost || last.elapsed() > Duration::from_secs(8)) && !engine.alive() {
            trace::line("[DB] connection lost", true);
            if let Err(e) = revive(&mut engine, &mut tunnel, &used) {
                lost = true;
                trace::line(format!("[DB] {e}"), true);
                send(Event::Error(tf!("Соединение потеряно, переподключиться не удалось: {e}", "Connection lost, could not reconnect: {e}")));
                continue;
            }
        }
        lost = false;
        let result = match cmd {
            Cmd::Query(sql, source) => {
                trace::client(format!("[DB] > {}", sql.trim()));
                engine.query(&sql).map(|table| {
                    trace::server(format!(
                        "[DB] {} rows, {} affected, {:.1} ms",
                        table.rows.len(),
                        table.affected,
                        table.elapsed.as_secs_f64() * 1000.0
                    ));
                    if sql.get(..4).is_some_and(|w| w.eq_ignore_ascii_case("use "))
                        || sql.get(..16).is_some_and(|w| w.eq_ignore_ascii_case("set search_path "))
                    {
                        used = Some(sql.clone());
                    }
                    let keys = source
                        .as_ref()
                        .map(|s| engine.primary_key(s))
                        .unwrap_or_default();
                    Event::Result(table, source, keys)
                })
            }
            Cmd::Databases => engine.databases().map(Event::Databases),
            Cmd::Tables(db) => engine.tables(&db).map(|tables| Event::Tables(db, tables)),
            Cmd::Import(statements) => {
                trace::client(format!("[DB] import, statements: {}", statements.len()));
                engine.import(&statements).map(|affected| {
                    trace::server(format!("[DB] committed, {affected} rows affected"));
                    Event::Imported(statements.len(), affected)
                })
            }
            Cmd::Search(find) => {
                trace::client(format!("[DB] search in {}", find.schema));
                let kind = engine.kind();
                let tables = match &find.table {
                    Some(table) => Ok(vec![table.clone()]),
                    None => engine.tables(&find.schema),
                };
                tables.map(|tables| {
                    let mut found = Vec::new();
                    for table in &tables {
                        let full = match kind {
                            DbKind::Sqlite => kind.quote(table),
                            _ => format!("{}.{}", kind.quote(&find.schema), kind.quote(table)),
                        };
                        let Ok(columns) = engine.query(&format!("SELECT * FROM {full} LIMIT 0"))
                        else {
                            continue;
                        };
                        if columns.columns.is_empty() {
                            continue;
                        }
                        let condition = find.condition(kind, &columns.columns);
                        let count = engine
                            .query(&format!("SELECT COUNT(*) FROM {full} WHERE {condition}"))
                            .ok()
                            .and_then(|t| t.rows.first()?.first()?.as_deref()?.parse::<u64>().ok())
                            .unwrap_or(0);
                        if count > 0 {
                            found.push((
                                table.clone(),
                                count,
                                format!("SELECT * FROM {full} WHERE {condition} LIMIT 200"),
                            ));
                        }
                    }
                    trace::server(format!(
                        "[DB] found in {} of {} tables",
                        found.len(),
                        tables.len()
                    ));
                    Event::Found(found, tables.len())
                })
            }
            Cmd::Structure(source) => {
                trace::client(format!("[DB] structure of {}.{}", source.schema, source.table));
                engine.structure(&source).map(|s| Event::Structure(Box::new(s)))
            }
            Cmd::Alter(statements, source) => engine.batch(&statements).and_then(|_| {
                trace::server("[DB] structure changed");
                engine.structure(&source).map(|s| Event::Structure(Box::new(s)))
            }),
            Cmd::Batch(statements, refresh, source) => engine.batch(&statements).and_then(|affected| {
                trace::server(format!("[DB] committed, {affected} rows affected"));
                let table = engine.query(&refresh)?;
                let keys = source
                    .as_ref()
                    .map(|s| engine.primary_key(s))
                    .unwrap_or_default();
                Ok(Event::Result(table, source, keys))
            }),
        };
        last = Instant::now();
        match result {
            Ok(event) => send(event),
            Err(e) => {
                trace::line(format!("[DB] {e}"), true);
                send(Event::Error(e));
            }
        }
    }
    trace::line("[DB] disconnected", false);
}

struct Schema {
    name: String,
    tables: Option<Vec<String>>,
    open: bool,
}

struct Editor {
    drafts: Vec<DbProfile>,
    index: usize,
    show_password: bool,
    probe: Option<Receiver<Result<String, String>>>,
    result: Option<Result<String, String>>,
}

#[derive(Default)]
struct Edits {
    cells: BTreeMap<(usize, usize), Option<String>>,
    deleted: Vec<usize>,
    added: Vec<Vec<Option<String>>>,
    editing: Option<(usize, usize, String)>,
    confirm: Option<Vec<String>>,
}

impl Edits {
    fn count(&self) -> usize {
        self.cells.len() + self.deleted.len() + self.added.len()
    }
}

#[derive(Clone, Copy, PartialEq, Default)]
enum Mode {
    #[default]
    Query,
    Search,
    Structure,
}

#[derive(Default)]
pub struct DbPanel {
    selected: usize,
    conn: Option<Conn>,
    kind: DbKind,
    version: Option<String>,
    schemas: Vec<Schema>,
    current: String,
    sql: String,
    last_sql: String,
    result: Option<Table>,
    source: Option<Source>,
    keys: Vec<String>,
    edits: Edits,
    widths: Vec<f32>,
    picked: Option<(usize, usize)>,
    status: String,
    error: bool,
    busy: bool,
    scroll_x: f32,
    editor: Option<Editor>,
    profile: String,
    history: Option<History>,
    import: Option<(String, Vec<String>)>,
    searching: bool,
    mode: Mode,
    structure: structure::State,
    find: Find,
    found: Option<(Hits, usize)>,
}

impl DbPanel {
    pub fn modal_open(&self) -> bool {
        self.editor.is_some() || self.edits.confirm.is_some() || self.structure_modal()
    }

    fn send(&mut self, cmd: Cmd) {
        if let Some(conn) = &self.conn
            && conn.tx.send(cmd).is_ok()
        {
            self.busy = true;
        }
    }

    fn run(&mut self, sql: String) {
        let sql = sql.trim().trim_end_matches(';').trim().to_owned();
        if sql.is_empty() {
            return;
        }
        self.status.clear();
        let source = source_of(&sql, &self.current);
        self.last_sql = sql.clone();
        let profile = self.profile.clone();
        self.history
            .get_or_insert_with(|| {
                History::load(eframe::storage_dir("Tror").map(|d| d.join("db_history.json")))
            })
            .push(&profile, &sql);
        self.send(Cmd::Query(sql, source));
    }

    fn run_on(&mut self, sql: String, source: Option<Source>) {
        self.status.clear();
        self.last_sql = sql.clone();
        self.send(Cmd::Query(sql, source));
    }

    fn connect(&mut self, store: &Store, ctx: &egui::Context) {
        let Some(profile) = store.databases.get(self.selected).cloned() else {
            return;
        };
        let mut ssh = None;
        if !profile.tunnel.is_empty() && profile.kind != DbKind::Sqlite {
            let Some(host) = store.profiles.iter().find(|p| p.name == profile.tunnel) else {
                self.status = tf!("Хост «{}» для туннеля не найден", "Host \"{}\" for the tunnel not found", profile.tunnel);
                self.error = true;
                return;
            };
            let key = format!("{}:{}", host.host.trim(), host.port);
            ssh = Some((host.clone(), store.known_hosts.get(&key).cloned()));
        }
        let (tx, cmds) = channel();
        let (events, rx) = channel();
        let ctx = ctx.clone();
        self.profile = profile.name.clone();
        self.kind = profile.kind;
        self.current = match profile.kind {
            DbKind::Sqlite => "main".into(),
            DbKind::Postgres => "public".into(),
            DbKind::MySql => profile.database.trim().to_owned(),
        };
        std::thread::spawn(move || {
            serve(profile, ssh, cmds, events, &move || ctx.request_repaint())
        });
        self.conn = Some(Conn { tx, rx });
        self.version = None;
        self.schemas.clear();
        self.result = None;
        self.structure = structure::State::default();
        self.edits = Edits::default();
        self.status = tr!("Подключение…").into();
        self.error = false;
        self.busy = true;
    }

    fn disconnect(&mut self) {
        self.structure = structure::State::default();
        self.conn = None;
        self.version = None;
        self.schemas.clear();
        self.busy = false;
        self.edits = Edits::default();
    }

    pub fn poll(&mut self) {
        let Some(conn) = &self.conn else {
            return;
        };
        let events: Vec<Event> = conn.rx.try_iter().collect();
        for event in events {
            self.busy = false;
            match event {
                Event::Connected(version) => {
                    self.status = tf!("Подключено: {version}", "Connected: {version}");
                    self.error = false;
                    self.version = Some(version);
                    self.send(Cmd::Databases);
                }
                Event::Databases(names) => {
                    let current = self.current.clone();
                    self.schemas = names
                        .into_iter()
                        .map(|name| Schema {
                            name,
                            tables: None,
                            open: false,
                        })
                        .collect();
                    if let Some(schema) = self.schemas.iter_mut().find(|s| s.name == current) {
                        schema.open = true;
                        self.send(Cmd::Tables(current));
                    } else if self.schemas.len() == 1 {
                        let name = self.schemas[0].name.clone();
                        self.schemas[0].open = true;
                        self.current = name.clone();
                        self.send(Cmd::Tables(name));
                    }
                }
                Event::Tables(db, tables) => {
                    if let Some(schema) = self.schemas.iter_mut().find(|s| s.name == db) {
                        schema.tables = Some(tables);
                    }
                }
                Event::Result(table, source, keys) => {
                    self.status = if table.columns.is_empty() {
                        tf!("Выполнено: затронуто строк {}, {:.1} мс. {}", "Done: rows affected {}, {:.1} ms. {}",
                            table.affected,
                            table.elapsed.as_secs_f64() * 1000.0,
                            table.info
                        )
                    } else {
                        tf!("Строк: {}{}, {:.1} мс", "Rows: {}{}, {:.1} ms",
                            table.rows.len(),
                            if table.truncated {
                                tf!(" (показаны первые {})", " (first {} shown)", max_rows())
                            } else {
                                String::new()
                            },
                            table.elapsed.as_secs_f64() * 1000.0
                        )
                    };
                    self.error = false;
                    self.widths = table
                        .columns
                        .iter()
                        .enumerate()
                        .map(|(i, name)| {
                            let widest = table
                                .rows
                                .iter()
                                .take(300)
                                .map(|row| row[i].as_deref().map_or(4, |c| c.chars().count()))
                                .max()
                                .unwrap_or(0);
                            widest.max(name.chars().count()).clamp(4, 48) as f32
                        })
                        .collect();
                    self.picked = None;
                    self.keys = keys
                        .into_iter()
                        .filter(|k| table.columns.contains(k))
                        .collect();
                    self.source = source;
                    self.edits = Edits::default();
                    self.result = Some(table);
                }
                Event::Found(found, total) => {
                    let hits: u64 = found.iter().map(|f| f.1).sum();
                    self.status = if found.is_empty() {
                        tf!(
                            "Ничего не найдено, таблиц просмотрено: {total}",
                            "Nothing found, tables checked: {total}"
                        )
                    } else {
                        tf!(
                            "Найдено строк: {hits}, таблиц: {} из {total}",
                            "Rows found: {hits}, tables: {} of {total}",
                            found.len()
                        )
                    };
                    self.error = false;
                    self.found = Some((found, total));
                    self.searching = false;
                }
                Event::Imported(count, affected) => {
                    self.status = tf!(
                        "Импорт выполнен: запросов {count}, затронуто строк {affected}",
                        "Import done: statements {count}, rows affected {affected}"
                    );
                    self.error = false;
                    self.send(Cmd::Databases);
                }
                Event::Structure(data) => {
                    if self.structure.loading.as_ref() == Some(&data.source) {
                        self.structure.loading = None;
                    }
                    if self.status == tr!("Изменение структуры…") {
                        self.status = tr!("Структура изменена").into();
                        self.error = false;
                    }
                    if self.structure.data.as_ref().map(|d| &d.source) != Some(&data.source) {
                        self.structure.picked = None;
                    }
                    self.structure.data = Some(*data);
                }
                Event::Error(e) => {
                    self.status = e;
                    self.error = true;
                    self.structure.loading = None;
                }
                Event::Closed(e) => {
                    self.status = tf!("Нет подключения: {e}", "Not connected: {e}");
                    self.error = true;
                    self.conn = None;
                    self.version = None;
                    self.schemas.clear();
                    return;
                }
            }
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, pal: &Palette, store: &mut Store, font_size: f32) {
        let ctx = ui.ctx().clone();
        self.poll();
        ui.add_space(8.0);
        if store.databases.is_empty() {
            crate::widgets::wrapped_row(ui, 8.0, |ui| {
                ui.label(RichText::new(tr!("Подключений к базам пока нет.")).color(pal.subtext));
                if ui
                    .add(Button::new(tr!("Создать подключение")))
                    .clicked()
                {
                    self.open_editor(store, true);
                }
            });
            return;
        }
        self.selected = self.selected.min(store.databases.len() - 1);
        let active = self.conn.is_some();
        let names: Vec<(String, String)> = store
            .databases
            .iter()
            .map(|p| (p.name.clone(), p.kind.label().to_owned()))
            .collect();
        let list_id = egui::Id::unique("tror_db_profile");
        let mut action = crate::widgets::profile_dialogs(&ctx, pal, list_id, &names);
        crate::widgets::wrapped_row(ui, 8.0, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let width = (ui.available_width() - 32.0).max(40.0);
                ui.add_enabled_ui(!active, |ui| {
                    let combo = ComboBox::from_id_salt(list_id)
                        .width(width)
                        .truncate()
                        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                        .selected_text(names[self.selected].0.clone())
                        .show_ui(ui, |ui| {
                            if let Some(a) = crate::widgets::profile_list(
                                ui,
                                list_id,
                                &names,
                                Some(self.selected),
                                tr!("Добавить подключение…"),
                            ) {
                                action = Some(a);
                            }
                        });
                    let current = self.selected;
                    let name = names[current].0.clone();
                    if let Some(a) =
                        crate::widgets::profile_context(ui, list_id, &combo.response, current, &name)
                    {
                        action = Some(a);
                    }
                });
                if crate::widgets::plus_button(ui, pal, egui::vec2(26.0, 24.0))
                    .on_hover_text(tr!("Добавить подключение"))
                    .clicked()
                {
                    action = Some(crate::widgets::ProfileAction::Add);
                }
            });
        });
        match action {
            Some(crate::widgets::ProfileAction::Select(i)) => self.selected = i,
            Some(crate::widgets::ProfileAction::Edit(i)) => {
                self.selected = i;
                self.open_editor(store, false);
            }
            Some(crate::widgets::ProfileAction::Rename(i, name)) => {
                if let Some(profile) = store.databases.get_mut(i) {
                    profile.name = name;
                    let _ = store.save();
                }
            }
            Some(crate::widgets::ProfileAction::Delete(i)) => {
                if i < store.databases.len() && !active {
                    store.databases.remove(i);
                    let _ = store.save();
                    self.selected = self.selected.min(store.databases.len().saturating_sub(1));
                    if store.databases.is_empty() {
                        return;
                    }
                }
            }
            Some(crate::widgets::ProfileAction::Add) => self.open_editor(store, true),
            None => {}
        }
        crate::widgets::wrapped_row(ui, 8.0, |ui| {
            let label = if active { tr!("Отключить") } else { tr!("Подключить") };
            if ui.add(Button::new(label)).clicked() {
                if active {
                    self.disconnect();
                    self.status = tr!("Отключено").into();
                    self.error = false;
                } else {
                    self.connect(store, &ctx);
                }
            }
            if self.busy {
                ui.spinner();
            }
            ui.add(
                egui::Label::new(RichText::new(&self.status).small().color(if self.error {
                    pal.warn
                } else {
                    pal.subtext
                }))
                .truncate(),
            );
        });
        ui.add_space(6.0);
        if self.version.is_none() {
            if !self.busy {
                ui.add_space(4.0);
                crate::widgets::wrapped_row(ui, 8.0, |ui| {
                    ui.label(
                        RichText::new(tr!("Выберите подключение и нажмите «Подключить». Здесь появятся таблицы, редактор запросов, поиск по базе и структура таблиц."))
                            .small()
                            .color(pal.subtext),
                    );
                });
            }
            return;
        }
        let rect = ui.available_rect_before_wrap();
        let (tree_rect, main_rect) = if rect.width() < 520.0 {
            let (top, bottom) = rect.split_top_bottom_at_fraction(0.32);
            ui.painter().line_segment(
                [top.left_bottom(), top.right_bottom()],
                Stroke::new(1.0, pal.border),
            );
            (top, bottom.shrink2(egui::vec2(0.0, 4.0)))
        } else {
            let tree_w = (rect.width() * 0.24).clamp(150.0, 280.0);
            let (left, right) = rect.split_left_right_at_x(rect.left() + tree_w);
            ui.painter().line_segment(
                [left.right_top(), left.right_bottom()],
                Stroke::new(1.0, pal.border),
            );
            (left, right)
        };
        let mut tree_ui =
            ui.new_child(egui::UiBuilder::new().max_rect(tree_rect.shrink2(egui::vec2(8.0, 0.0))));
        self.tree(&mut tree_ui, pal);
        let mut main_ui =
            ui.new_child(egui::UiBuilder::new().max_rect(main_rect.shrink2(egui::vec2(8.0, 0.0))));
        self.main(&mut main_ui, pal, font_size);
        ui.advance_cursor_after_rect(rect);
    }

    fn table_queries(&self, schema: &str, table: &str) -> Vec<(&'static str, String)> {
        let kind = self.kind;
        let full = format!("{}.{}", kind.quote(schema), kind.quote(table));
        let first = (tr!("Первые 200 строк"), format!("SELECT * FROM {full} LIMIT 200"));
        let count = (tr!("Число строк"), format!("SELECT COUNT(*) FROM {full}"));
        match kind {
            DbKind::MySql => vec![
                first,
                (tr!("Столбцы"), format!("SHOW FULL COLUMNS FROM {full}")),
                (tr!("Индексы"), format!("SHOW INDEX FROM {full}")),
                (tr!("SQL создания"), format!("SHOW CREATE TABLE {full}")),
                count,
            ],
            DbKind::Postgres => vec![
                first,
                (
                    tr!("Столбцы"),
                    format!(
                        "SELECT column_name, data_type, is_nullable, column_default FROM information_schema.columns WHERE table_schema = {} AND table_name = {} ORDER BY ordinal_position",
                        kind.literal(Some(schema)),
                        kind.literal(Some(table))
                    ),
                ),
                (
                    tr!("Индексы"),
                    format!(
                        "SELECT indexname, indexdef FROM pg_indexes WHERE schemaname = {} AND tablename = {}",
                        kind.literal(Some(schema)),
                        kind.literal(Some(table))
                    ),
                ),
                count,
            ],
            DbKind::Sqlite => vec![
                first,
                (
                    tr!("Столбцы"),
                    format!("PRAGMA {}.table_info({})", kind.quote(schema), kind.quote(table)),
                ),
                (
                    tr!("Индексы"),
                    format!("PRAGMA {}.index_list({})", kind.quote(schema), kind.quote(table)),
                ),
                (
                    tr!("SQL создания"),
                    format!(
                        "SELECT sql FROM {}.sqlite_master WHERE name = {}",
                        kind.quote(schema),
                        kind.literal(Some(table))
                    ),
                ),
                count,
            ],
        }
    }

    fn tree(&mut self, ui: &mut Ui, pal: &Palette) {
        let mut refresh = false;
        let mut csv_into: Option<(String, String)> = None;
        let kind = self.kind;
        ui.horizontal(|ui| {
            let title = match kind {
                DbKind::MySql => tr!("Базы и таблицы"),
                DbKind::Postgres => tr!("Схемы и таблицы"),
                DbKind::Sqlite => tr!("Таблицы"),
            };
            ui.label(RichText::new(title).small().color(pal.faint));
            if crate::widgets::refresh_button(ui, pal)
                .on_hover_text(tr!("Обновить список"))
                .clicked()
            {
                refresh = true;
            }
        });
        let mut toggle = None;
        let mut run: Option<(String, String, bool)> = None;
        let mut show: Option<(String, String)> = None;
        ScrollArea::vertical()
            .id_salt("tror_db_tree")
            .auto_shrink(false)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                for (index, schema) in self.schemas.iter().enumerate() {
                    let current = schema.name == self.current;
                    let mark = if schema.open { "▼" } else { "▶" };
                    let text = RichText::new(format!("{mark}  {}", schema.name))
                        .color(if current { pal.accent } else { pal.text });
                    if ui.add(Button::new(text).frame(false).truncate()).clicked() {
                        toggle = Some(index);
                    }
                    if !schema.open {
                        continue;
                    }
                    match &schema.tables {
                        None => {
                            ui.label(RichText::new("      …").color(pal.faint));
                        }
                        Some(tables) => {
                            for table in tables {
                                let response = ui.add(
                                    Button::new(
                                        RichText::new(format!("      {table}")).color(pal.subtext),
                                    )
                                    .frame(false)
                                    .truncate(),
                                );
                                let queries = self.table_queries(&schema.name, table);
                                if response.clicked() {
                                    if self.mode == Mode::Structure {
                                        show = Some((schema.name.clone(), table.clone()));
                                    } else {
                                        run = Some((schema.name.clone(), queries[0].1.clone(), true));
                                    }
                                }
                                response.context_menu(|ui| {
                                    if ui.button(tr!("Структура")).clicked() {
                                        show = Some((schema.name.clone(), table.clone()));
                                        ui.close();
                                    }
                                    for (label, sql) in &queries {
                                        if ui.button(*label).clicked() {
                                            run = Some((schema.name.clone(), sql.clone(), true));
                                            ui.close();
                                        }
                                    }
                                    ui.separator();
                                    if ui.button(tr!("Импорт CSV в эту таблицу…")).clicked() {
                                        csv_into = Some((schema.name.clone(), table.clone()));
                                        ui.close();
                                    }
                                });
                            }
                        }
                    }
                }
            });
        if refresh {
            self.send(Cmd::Databases);
        }
        if let Some(index) = toggle {
            let schema = &mut self.schemas[index];
            schema.open = !schema.open;
            if schema.open {
                let name = schema.name.clone();
                self.current = name.clone();
                self.send(Cmd::Tables(name.clone()));
                if kind == DbKind::MySql {
                    self.run(format!("USE {}", kind.quote(&name)));
                }
            }
        }
        if let Some((schema, table)) = csv_into
            && let Some(path) = rfd::FileDialog::new()
                .add_filter("CSV", &["csv", "tsv", "txt"])
                .pick_file()
        {
            let full = match kind {
                DbKind::Sqlite => kind.quote(&table),
                _ => format!("{}.{}", kind.quote(&schema), kind.quote(&table)),
            };
            let built = std::fs::read(&path)
                .map_err(|e| e.to_string())
                .and_then(|bytes| csv_inserts(&parse_csv(&String::from_utf8_lossy(&bytes)), kind, &full));
            match built {
                Ok(statements) => {
                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    self.import = Some((format!("{name} → {table}"), statements));
                }
                Err(e) => {
                    self.status = tf!("CSV не импортирован: {e}", "CSV not imported: {e}");
                    self.error = true;
                }
            }
        }
        if let Some((schema, table)) = show {
            self.current = schema.clone();
            self.mode = Mode::Structure;
            self.open_structure(Source { schema, table });
        }
        if let Some((schema, sql, show)) = run {
            self.current = schema;
            if show {
                self.sql = sql.clone();
            }
            self.run(sql);
        }
    }

    fn main(&mut self, ui: &mut Ui, pal: &Palette, font_size: f32) {
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            crate::widgets::segmented(
                ui,
                pal,
                &mut self.mode,
                &[
                    (Mode::Query, tr!("Запрос")),
                    (Mode::Search, tr!("Поиск")),
                    (Mode::Structure, tr!("Структура")),
                ],
                96.0,
            );
        });
        ui.add_space(6.0);
        if self.mode == Mode::Search {
            self.search_view(ui, pal);
            return;
        }
        if self.mode == Mode::Structure {
            self.structure_view(ui, pal, font_size);
            return;
        }
        let font = FontId::monospace(font_size - 1.0);
        let run_key = ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Enter));
        let output = TextEdit::multiline(&mut self.sql)
            .id_salt("tror_db_sql")
            .font(font.clone())
            .hint_text(
                tr!("SQL-запрос — Ctrl+Enter для выполнения (выделенный фрагмент выполняется отдельно)"),
            )
            .desired_rows(4)
            .desired_width(f32::INFINITY)
            .show(ui);
        let selected = output
            .cursor_range
            .map(|range| range.as_sorted_char_range())
            .filter(|range| !range.is_empty())
            .map(|range| {
                self.sql
                    .chars()
                    .skip(range.start.0)
                    .take(range.end.0 - range.start.0)
                    .collect::<String>()
            });
        let mut export = None;
        let mut apply = false;
        let mut discard = false;
        let mut add_row = false;
        ui.horizontal_wrapped(|ui| {
            let run = ui
                .add_enabled(
                    !self.busy,
                    crate::widgets::primary(tr!("Выполнить"), pal),
                ).lit(ui)
                .clicked();
            if (run || (run_key && output.response.has_focus())) && !self.busy {
                self.run(selected.unwrap_or_else(|| self.sql.clone()));
            }
            let history = self.history.get_or_insert_with(|| {
                History::load(eframe::storage_dir("Tror").map(|d| d.join("db_history.json")))
            });
            let recent: Vec<String> = history.recent(&self.profile).take(40).map(str::to_owned).collect();
            ui.add_enabled_ui(!recent.is_empty(), |ui| {
                ui.menu_button(tr!("История"), |ui| {
                    ui.set_min_width(360.0);
                    ui.set_max_width(520.0);
                    egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                        for sql in &recent {
                            let line: String = sql.split_whitespace().collect::<Vec<_>>().join(" ");
                            let short: String = line.chars().take(90).collect();
                            let label = if short.len() < line.len() { format!("{short}…") } else { short };
                            if ui
                                .add(egui::Button::new(RichText::new(label).font(font.clone())).truncate())
                                .on_hover_text(sql)
                                .clicked()
                            {
                                self.sql.clone_from(sql);
                                output.response.request_focus();
                                ui.close();
                            }
                        }
                    });
                });
            });
            if ui
                .add_enabled(!self.busy, egui::Button::new(tr!("Импорт SQL…")))
                .on_hover_text(tr!("Выполнить запросы из файла .sql (например, дамп базы) в одной транзакции"))
                .clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("SQL", &["sql", "txt"])
                    .pick_file()
            {
                match std::fs::read(&path) {
                    Ok(bytes) => {
                        let text = String::from_utf8_lossy(&bytes);
                        let statements = split_sql(&text);
                        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        self.import = Some((name, statements));
                    }
                    Err(e) => {
                        self.status = tf!("Не удалось прочитать {}: {e}", "Could not read {}: {e}", path.display());
                        self.error = true;
                    }
                }
            }
            if self.result.as_ref().is_some_and(|t| !t.columns.is_empty()) {
                ui.separator();
                if ui.button(tr!("Экспорт…")).on_hover_text(tr!("В файл CSV, JSON или SQL — формат выбирается в окне сохранения")).clicked() {
                    export = Some(true);
                }
                if ui
                    .button(tr!("Копировать (TSV)"))
                    .on_hover_text(tr!("Вся таблица результата в буфер обмена"))
                    .clicked()
                {
                    export = Some(false);
                }
            }
            if self.editable() {
                if ui
                    .button(tr!("+ Строка"))
                    .on_hover_text(tr!("Добавить строку в таблицу"))
                    .clicked()
                {
                    add_row = true;
                }
                let count = self.edits.count();
                if count > 0 {
                    if ui
                        .add(
                            crate::widgets::primary(tf!("Применить изменения ({count})", "Apply changes ({count})"), pal),
                        ).lit(ui)
                        .clicked()
                    {
                        apply = true;
                    }
                    if ui.button(tr!("Отменить")).clicked() {
                        discard = true;
                    }
                }
            }
            if !self.current.is_empty() {
                let what = match self.kind {
                    DbKind::MySql => tr!("база"),
                    DbKind::Postgres => tr!("схема"),
                    DbKind::Sqlite => tr!("база"),
                };
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!("{what}: {}", self.current))
                            .small()
                            .color(pal.subtext),
                    );
                });
            }
        });
        if discard {
            self.edits = Edits::default();
        }
        if add_row && let Some(table) = &self.result {
            self.edits.added.push(vec![None; table.columns.len()]);
        }
        if apply {
            let statements = self.statements();
            if !statements.is_empty() {
                self.edits.confirm = Some(statements);
            }
        }
        if let (Some(to_file), Some(table)) = (export, &self.result) {
            if to_file {
                let name = self
                    .source
                    .as_ref()
                    .map_or_else(|| "result".to_owned(), |s| s.table.clone());
                if let Some(path) = rfd::FileDialog::new()
                    .set_file_name(format!("{name}.csv"))
                    .add_filter(tr!("CSV, разделитель «;»"), &["csv"])
                    .add_filter("JSON", &["json"])
                    .add_filter(tr!("SQL — команды INSERT"), &["sql"])
                    .save_file()
                {
                    let text = match path.extension().and_then(|e| e.to_str()) {
                        Some(e) if e.eq_ignore_ascii_case("json") => json(table),
                        Some(e) if e.eq_ignore_ascii_case("sql") => inserts(table, self.kind, &name),
                        _ => csv(table, ';'),
                    };
                    match std::fs::write(&path, text) {
                        Ok(()) => self.status = tf!("Сохранено: {}", "Saved: {}", path.display()),
                        Err(e) => {
                            self.status = tf!("Не сохранено: {e}", "Not saved: {e}");
                            self.error = true;
                        }
                    }
                }
            } else {
                ui.ctx().copy_text(csv(table, '\t'));
            }
        }
        if let Some(table) = &self.result
            && !table.columns.is_empty()
            && self.source.is_some()
            && self.keys.is_empty()
        {
            ui.label(
                RichText::new(tr!("У таблицы нет первичного ключа — правка в сетке недоступна, меняйте данные запросами."))
                    .small()
                    .color(pal.faint),
            );
        } else if self.editable() {
            ui.label(
                RichText::new(tr!("Двойной щелчок по ячейке — правка, Enter — принять, Esc — отмена. Правый щелчок — NULL и удаление строки."))
                    .small()
                    .color(pal.faint),
            );
        }
        ui.add_space(4.0);
        self.grid(ui, pal, &font, font_size);
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_quoted_and_csv_is_escaped() {
        assert_eq!(quote_name("b_user"), "`b_user`");
        assert_eq!(quote_name("we`ird"), "`we``ird`");
        assert_eq!(DbKind::Postgres.quote("a\"b"), "\"a\"\"b\"");
        assert_eq!(DbKind::MySql.literal(Some("it's \\")), "'it''s \\\\'");
        assert_eq!(DbKind::Sqlite.literal(Some("it's \\")), "'it''s \\'");
        assert_eq!(DbKind::Sqlite.literal(None), "NULL");
        let table = Table {
            columns: vec!["id".into(), "title".into()],
            rows: vec![
                vec![Some("1".into()), Some("a;b".into())],
                vec![Some("2".into()), None],
                vec![Some("3".into()), Some("say \"hi\"".into())],
            ],
            affected: 0,
            info: String::new(),
            elapsed: Duration::ZERO,
            truncated: false,
        };
        assert_eq!(
            csv(&table, ';'),
            "id;title\n1;\"a;b\"\n2;\n3;\"say \"\"hi\"\"\"\n"
        );
    }

    #[test]
    fn result_exports_as_json_and_inserts() {
        let table = Table {
            columns: vec!["id".into(), "name".into()],
            rows: vec![
                vec![Some("1".into()), Some("O'Neil".into())],
                vec![Some("2".into()), None],
            ],
            ..Table::empty()
        };
        let parsed: serde_json::Value = serde_json::from_str(&json(&table)).unwrap();
        assert_eq!(parsed[0]["name"], "O'Neil");
        assert!(parsed[1]["name"].is_null());
        assert_eq!(
            inserts(&table, DbKind::MySql, "users"),
            "INSERT INTO `users` (`id`, `name`) VALUES ('1', 'O''Neil');\nINSERT INTO `users` (`id`, `name`) VALUES ('2', NULL);\n"
        );
    }

    #[test]
    fn values_become_text() {
        assert_eq!(cell(mysql::Value::NULL), None);
        assert_eq!(cell(mysql::Value::Int(-5)).as_deref(), Some("-5"));
        assert_eq!(
            cell(mysql::Value::Bytes("привет".as_bytes().to_vec())).as_deref(),
            Some("привет")
        );
        assert_eq!(
            cell(mysql::Value::Date(2026, 3, 9, 4, 5, 6, 0)).as_deref(),
            Some("2026-03-09 04:05:06")
        );
        let long = cell(mysql::Value::Bytes(vec![b'x'; MAX_CELL + 50])).unwrap();
        assert_eq!(long.chars().count(), MAX_CELL + 1);
    }

    #[test]
    fn sources_are_found_in_simple_selects() {
        assert_eq!(
            source_of("SELECT * FROM `shop`.`goods` LIMIT 200", "x"),
            Some(Source {
                schema: "shop".into(),
                table: "goods".into()
            })
        );
        assert_eq!(
            source_of("select * from b_iblock_element where ID > 5;", "site"),
            Some(Source {
                schema: "site".into(),
                table: "b_iblock_element".into()
            })
        );
        assert_eq!(source_of("SELECT id FROM t", "x"), None);
        assert_eq!(source_of("SELECT * FROM a JOIN b ON 1", "x"), None);
    }

    #[test]
    fn sqlite_editing_roundtrip() {
        let dir = std::env::temp_dir().join(format!("tror-db-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("shop.db");
        let _ = std::fs::remove_file(&path);
        let profile = DbProfile {
            kind: DbKind::Sqlite,
            file: path.display().to_string(),
            ..Default::default()
        };
        let (mut engine, version) = open_engine(&profile, String::new(), 0).unwrap();
        assert!(version.starts_with("SQLite"));
        engine
            .query("CREATE TABLE goods (id INTEGER PRIMARY KEY, title TEXT, price REAL); INSERT INTO goods (title, price) VALUES ('Чайник', 990.5), ('Кружка', NULL)")
            .unwrap();
        assert_eq!(engine.databases().unwrap(), vec!["main"]);
        assert_eq!(engine.tables("main").unwrap(), vec!["goods"]);
        let source = Source {
            schema: "main".into(),
            table: "goods".into(),
        };
        assert_eq!(engine.primary_key(&source), vec!["id"]);
        let mut panel = DbPanel {
            kind: DbKind::Sqlite,
            source: Some(source.clone()),
            keys: vec!["id".into()],
            result: Some(engine.query("SELECT * FROM goods").unwrap()),
            ..Default::default()
        };
        assert_eq!(panel.result.as_ref().unwrap().rows[0][1].as_deref(), Some("Чайник"));
        panel.set_cell(1, 2, Some("150".into()));
        panel.set_cell(0, 1, Some("Чайник 'Ретро'".into()));
        panel.set_cell(0, 1, Some("Чайник".into()));
        panel.edits.deleted.push(0);
        panel.edits.added.push(vec![None, Some("Ложка".into()), Some("12".into())]);
        let statements = panel.statements();
        assert_eq!(
            statements,
            vec![
                "UPDATE \"main\".\"goods\" SET \"price\" = '150' WHERE \"id\" = '2'".to_owned(),
                "DELETE FROM \"main\".\"goods\" WHERE \"id\" = '1'".to_owned(),
                "INSERT INTO \"main\".\"goods\" (\"title\", \"price\") VALUES ('Ложка', '12')".to_owned(),
            ]
        );
        engine.batch(&statements).unwrap();
        let after = engine.query("SELECT title, price FROM goods ORDER BY id").unwrap();
        assert_eq!(
            after.rows,
            vec![
                vec![Some("Кружка".into()), Some("150".into())],
                vec![Some("Ложка".into()), Some("12".into())],
            ]
        );
        assert!(engine.batch(&["UPDATE nope SET x = 1".into()]).is_err());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    #[ignore]
    fn live_mysql_server() {
        let profile = DbProfile {
            user: "jozef".into(),
            password: "secret".into(),
            ..Default::default()
        };
        let (mut engine, _) = open_engine(&profile, "127.0.0.1".into(), 3307).unwrap();
        let dbs = engine.databases().unwrap();
        assert!(dbs.iter().any(|d| d == "shop"), "{dbs:?}");
        let table = engine.query("SELECT id, title, price FROM shop.goods").unwrap();
        assert_eq!(table.columns, ["id", "title", "price"]);
        assert_eq!(table.rows.len(), 3);
        assert_eq!(table.rows[1][1].as_deref(), Some("Чайник"));
        assert_eq!(table.rows[2][2], None);
        assert!(engine.query("SELEC nonsense").is_err());
    }

    #[test]
    #[ignore]
    fn live_postgres_server() {
        let profile = DbProfile {
            kind: DbKind::Postgres,
            user: "jozef".into(),
            password: "secret".into(),
            database: "shop".into(),
            ..Default::default()
        };
        let (mut engine, version) = open_engine(&profile, "127.0.0.1".into(), 5432).unwrap();
        assert!(version.starts_with("PostgreSQL"), "{version}");
        engine
            .query("DROP TABLE IF EXISTS goods; CREATE TABLE goods (id serial PRIMARY KEY, title text, price numeric); INSERT INTO goods (title, price) VALUES ('Чайник', 990.5), ('Кружка', NULL)")
            .unwrap();
        assert!(engine.databases().unwrap().contains(&"public".to_owned()));
        assert!(engine.tables("public").unwrap().contains(&"goods".to_owned()));
        let source = Source {
            schema: "public".into(),
            table: "goods".into(),
        };
        assert_eq!(engine.primary_key(&source), vec!["id"]);
        let table = engine.query("SELECT * FROM goods ORDER BY id").unwrap();
        assert_eq!(table.rows[0][1].as_deref(), Some("Чайник"));
        assert_eq!(table.rows[1][2], None);
        engine
            .batch(&["UPDATE \"public\".\"goods\" SET \"price\" = '5' WHERE \"id\" = '2'".into()])
            .unwrap();
        let table = engine.query("SELECT price FROM goods WHERE id = 2").unwrap();
        assert_eq!(table.rows[0][0].as_deref(), Some("5"));
        assert!(engine.query("SELEC nonsense").is_err());
    }
}
#[cfg(test)]
mod idle {
    use super::*;

    #[test]
    #[ignore]
    fn live_idle_reconnect() {
        let profile = DbProfile {
            user: "t".into(),
            password: "t".into(),
            port: 3306,
            ..Default::default()
        };
        let (mut raw, _) = open_engine(&profile, "127.0.0.1".into(), 3306).unwrap();
        std::thread::sleep(Duration::from_secs(7));
        eprintln!("raw after idle: {:?}", raw.query("SELECT 1").err());
        let (tx, cmds) = channel();
        let (events, rx) = channel();
        std::thread::spawn(move || serve(profile, None, cmds, events, &|| {}));
        assert!(matches!(rx.recv().unwrap(), Event::Connected(_)));
        tx.send(Cmd::Query("USE demo".into(), None)).unwrap();
        assert!(matches!(rx.recv().unwrap(), Event::Result(..)));
        tx.send(Cmd::Query("SELECT v FROM x".into(), None)).unwrap();
        assert!(matches!(rx.recv().unwrap(), Event::Result(..)));
        std::thread::sleep(Duration::from_secs(9));
        tx.send(Cmd::Query("SELECT v FROM x".into(), None)).unwrap();
        match rx.recv().unwrap() {
            Event::Result(t, ..) => assert_eq!(t.rows[0][0].as_deref(), Some("a")),
            Event::Error(e) => panic!("{e}"),
            _ => panic!(),
        }
    }
}
#[cfg(test)]
mod idle_pg {
    use super::*;

    #[test]
    #[ignore]
    fn live_pg_idle_reconnect() {
        let profile = DbProfile {
            kind: DbKind::Postgres,
            user: "postgres".into(),
            port: 5433,
            database: "postgres".into(),
            ..Default::default()
        };
        let (mut raw, _) = open_engine(&profile, "127.0.0.1".into(), 5433).unwrap();
        std::thread::sleep(Duration::from_secs(7));
        eprintln!("raw after idle: {:?}", raw.query("SELECT 1").err());
        let (tx, cmds) = channel();
        let (events, rx) = channel();
        std::thread::spawn(move || serve(profile, None, cmds, events, &|| {}));
        assert!(matches!(rx.recv().unwrap(), Event::Connected(_)));
        std::thread::sleep(Duration::from_secs(9));
        tx.send(Cmd::Query("SELECT v FROM x".into(), None)).unwrap();
        match rx.recv().unwrap() {
            Event::Result(t, ..) => assert_eq!(t.rows[0][0].as_deref(), Some("a")),
            Event::Error(e) => panic!("{e}"),
            _ => panic!(),
        }
    }
}

#[cfg(test)]
mod sql_tests {
    use super::*;

    #[test]
    fn sql_files_split_and_csv_becomes_inserts() {
        let dump = "-- comment; here\nCREATE TABLE t (a TEXT); /* x; */\nINSERT INTO t VALUES ('a;b', \"c\\\";d\");\n/*!40101 SET NAMES utf8 */;\nDELIMITER //\nCREATE TRIGGER x BEGIN SELECT 1; END//\nDELIMITER ;\nSELECT 2;";
        let parts = split_sql(dump);
        assert_eq!(
            parts,
            [
                "CREATE TABLE t (a TEXT)",
                "INSERT INTO t VALUES ('a;b', \"c\\\";d\")",
                "/*!40101 SET NAMES utf8 */",
                "CREATE TRIGGER x BEGIN SELECT 1; END",
                "SELECT 2"
            ]
        );
        let rows = parse_csv("\u{feff}id;name;note\r\n1;\"Иван; мл.\";\n2;\"\";\"say \"\"hi\"\"\"\n");
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[1], [Some("1".into()), Some("Иван; мл.".into()), None]);
        assert_eq!(rows[2], [Some("2".into()), Some(String::new()), Some("say \"hi\"".into())]);
        let inserts = csv_inserts(&rows, DbKind::Sqlite, "\"t\"").unwrap();
        assert_eq!(inserts[0], "INSERT INTO \"t\" (\"id\", \"name\", \"note\") VALUES ('1', 'Иван; мл.', NULL)");
        assert!(csv_inserts(&parse_csv("a,b\n1\n"), DbKind::Sqlite, "t").is_err());
        let path = std::env::temp_dir().join(format!("tror-import-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let profile = DbProfile {
            kind: DbKind::Sqlite,
            file: path.display().to_string(),
            ..Default::default()
        };
        let (mut engine, _) = open_engine(&profile, String::new(), 0).unwrap();
        engine
            .import(&split_sql("CREATE TABLE t (id INTEGER, name TEXT, note TEXT);\n-- x\n"))
            .unwrap();
        assert_eq!(engine.import(&inserts).unwrap(), 2);
        let mut broken = inserts.clone();
        broken.push("INSERT INTO nowhere VALUES (1)".into());
        let error = engine.import(&broken).unwrap_err();
        assert!(error.contains("3"), "{error}");
        let count = engine.query("SELECT COUNT(*), SUM(note IS NULL) FROM t").unwrap();
        assert_eq!(count.rows[0][0].as_deref(), Some("2"));
        assert_eq!(count.rows[0][1].as_deref(), Some("1"));
        drop(engine);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn search_conditions_per_engine() {
        let cols = vec!["a".to_owned(), "b".to_owned()];
        let find = |case, exact| Find {
            needle: "50%_x".into(),
            case,
            exact,
            ..Default::default()
        };
        assert_eq!(
            find(false, false).condition(DbKind::MySql, &cols),
            "LOWER(CAST(`a` AS CHAR)) LIKE LOWER('%50!%!_x%') ESCAPE '!' OR LOWER(CAST(`b` AS CHAR)) LIKE LOWER('%50!%!_x%') ESCAPE '!'"
        );
        assert_eq!(
            find(true, false).condition(DbKind::Postgres, &cols[..1]),
            "\"a\"::text LIKE '%50!%!_x%' ESCAPE '!'"
        );
        assert_eq!(
            find(false, false).condition(DbKind::Postgres, &cols[..1]),
            "\"a\"::text ILIKE '%50!%!_x%' ESCAPE '!'"
        );
        assert_eq!(
            find(true, true).condition(DbKind::MySql, &cols[..1]),
            "BINARY CAST(`a` AS CHAR) = '50%_x'"
        );
    }
}
