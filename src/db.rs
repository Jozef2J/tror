use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align2, Button, ComboBox, FontId, Key, Modifiers, RichText, ScrollArea, Sense, Stroke,
    TextEdit, Ui,
};
use mysql::prelude::Queryable;
use serde::{Deserialize, Serialize};

use crate::app::{dialog, dialog_button};
use crate::remote::{self, Profile, Store};
use crate::theme::Palette;
use crate::trace;

const MAX_ROWS: usize = 5000;
const MAX_CELL: usize = 4000;

#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
#[serde(default)]
pub struct DbProfile {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    #[serde(with = "crate::remote::masked")]
    pub password: String,
    pub database: String,
    pub tunnel: String,
}

impl Default for DbProfile {
    fn default() -> Self {
        Self {
            name: String::new(),
            host: "127.0.0.1".into(),
            port: 3306,
            user: String::new(),
            password: String::new(),
            database: String::new(),
            tunnel: String::new(),
        }
    }
}

pub struct Table {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Option<String>>>,
    pub affected: u64,
    pub info: String,
    pub elapsed: Duration,
    pub truncated: bool,
}

enum Cmd {
    Query(String),
    Databases,
    Tables(String),
}

enum Event {
    Connected(String),
    Databases(Vec<String>),
    Tables(String, Vec<String>),
    Result(Table),
    Error(String),
    Closed(String),
}

struct Conn {
    tx: Sender<Cmd>,
    rx: Receiver<Event>,
}

fn cell(value: mysql::Value) -> Option<String> {
    use mysql::Value;
    Some(match value {
        Value::NULL => return None,
        Value::Bytes(bytes) => {
            let text = String::from_utf8_lossy(&bytes);
            if text.len() > MAX_CELL {
                let cut = (0..=MAX_CELL)
                    .rev()
                    .find(|i| text.is_char_boundary(*i))
                    .unwrap_or(0);
                format!("{}…", &text[..cut])
            } else {
                text.into_owned()
            }
        }
        Value::Int(v) => v.to_string(),
        Value::UInt(v) => v.to_string(),
        Value::Float(v) => v.to_string(),
        Value::Double(v) => v.to_string(),
        Value::Date(y, mo, d, h, mi, s, us) => {
            if us > 0 {
                format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}.{us:06}")
            } else {
                format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}")
            }
        }
        Value::Time(negative, days, h, mi, s, _) => format!(
            "{}{:02}:{mi:02}:{s:02}",
            if negative { "-" } else { "" },
            days * 24 + h as u32
        ),
    })
}

fn run_query(conn: &mut mysql::Conn, sql: &str) -> Result<Table, String> {
    let started = Instant::now();
    let mut result = conn.query_iter(sql).map_err(explain)?;
    let mut table = Table {
        columns: Vec::new(),
        rows: Vec::new(),
        affected: 0,
        info: String::new(),
        elapsed: Duration::ZERO,
        truncated: false,
    };
    while let Some(set) = result.iter() {
        let columns: Vec<String> = set
            .columns()
            .as_ref()
            .iter()
            .map(|c| c.name_str().into_owned())
            .collect();
        let affected = set.affected_rows();
        let info = set.info_str().into_owned();
        let mut rows = Vec::new();
        let mut truncated = false;
        for row in set {
            let row = row.map_err(explain)?;
            if rows.len() >= MAX_ROWS {
                truncated = true;
                continue;
            }
            rows.push(row.unwrap().into_iter().map(cell).collect());
        }
        if !columns.is_empty() || table.columns.is_empty() {
            table.columns = columns;
            table.rows = rows;
            table.truncated = truncated;
        }
        table.affected += affected;
        if !info.is_empty() {
            table.info = info;
        }
    }
    table.elapsed = started.elapsed();
    Ok(table)
}

fn names(conn: &mut mysql::Conn, sql: &str) -> Result<Vec<String>, String> {
    Ok(run_query(conn, sql)?
        .rows
        .into_iter()
        .filter_map(|row| row.into_iter().next().flatten())
        .collect())
}

fn explain(error: mysql::Error) -> String {
    match error {
        mysql::Error::MySqlError(e) => format!("ERROR {} ({}): {}", e.code, e.state, e.message),
        other => other.to_string(),
    }
}

pub fn quote_name(name: &str) -> String {
    format!("`{}`", name.replace('`', "``"))
}

fn serve(
    profile: DbProfile,
    ssh: Option<(Profile, Option<String>)>,
    cmds: Receiver<Cmd>,
    events: Sender<Event>,
    wake: &dyn Fn(),
) {
    let send = |event: Event| {
        let _ = events.send(event);
        wake();
    };
    let mut host = profile.host.trim().to_owned();
    let mut port = profile.port;
    let mut _tunnel = None;
    if let Some((ssh, known)) = ssh {
        match remote::tunnel(&ssh, known.as_deref(), &host, port) {
            Ok(tunnel) => {
                host = "127.0.0.1".into();
                port = tunnel.port;
                _tunnel = Some(tunnel);
            }
            Err(e) => {
                trace::line(format!("[DB] tunnel failed: {e}"), true);
                send(Event::Closed(e));
                return;
            }
        }
    }
    trace::line(
        format!(
            "[DB] connecting to mysql://{}@{host}:{port}/{}",
            profile.user.trim(),
            profile.database.trim()
        ),
        false,
    );
    let database = profile.database.trim();
    let opts = mysql::OptsBuilder::new()
        .ip_or_hostname(Some(host))
        .tcp_port(port)
        .user(Some(profile.user.trim()))
        .pass(Some(profile.password.as_str()))
        .db_name((!database.is_empty()).then_some(database))
        .prefer_socket(false)
        .tcp_connect_timeout(Some(Duration::from_secs(12)));
    let mut conn = match mysql::Conn::new(opts) {
        Ok(conn) => conn,
        Err(e) => {
            let e = explain(e);
            trace::line(format!("[DB] {e}"), true);
            send(Event::Closed(e));
            return;
        }
    };
    let (a, b, c) = conn.server_version();
    let version = format!("{a}.{b}.{c}");
    trace::line(format!("[DB] connected, server version {version}"), false);
    send(Event::Connected(version));
    for cmd in cmds {
        let sql = match &cmd {
            Cmd::Query(sql) => sql.clone(),
            Cmd::Databases => "SHOW DATABASES".into(),
            Cmd::Tables(db) => format!("SHOW TABLES FROM {}", quote_name(db)),
        };
        trace::line(format!("[DB] > {}", sql.trim()), false);
        let result = match cmd {
            Cmd::Query(sql) => run_query(&mut conn, &sql).map(|table| {
                trace::line(
                    format!(
                        "[DB] {} rows, {} affected, {:.1} ms",
                        table.rows.len(),
                        table.affected,
                        table.elapsed.as_secs_f64() * 1000.0
                    ),
                    false,
                );
                Event::Result(table)
            }),
            Cmd::Databases => names(&mut conn, &sql).map(Event::Databases),
            Cmd::Tables(db) => names(&mut conn, &sql).map(|tables| Event::Tables(db, tables)),
        };
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
    profile: DbProfile,
    index: Option<usize>,
    show_password: bool,
}

#[derive(Default)]
pub struct DbPanel {
    selected: usize,
    conn: Option<Conn>,
    version: Option<String>,
    schemas: Vec<Schema>,
    current: String,
    sql: String,
    result: Option<Table>,
    widths: Vec<f32>,
    picked: Option<(usize, usize)>,
    status: String,
    error: bool,
    busy: bool,
    scroll_x: f32,
    editor: Option<Editor>,
}

fn csv(table: &Table, separator: char) -> String {
    let field = |text: &str| {
        if text.contains([separator, '"', '\n', '\r']) {
            format!("\"{}\"", text.replace('"', "\"\""))
        } else {
            text.to_owned()
        }
    };
    let mut out = String::new();
    let mut line = |cells: Vec<String>| {
        out.push_str(&cells.join(&separator.to_string()));
        out.push('\n');
    };
    line(table.columns.iter().map(|c| field(c)).collect());
    for row in &table.rows {
        line(
            row.iter()
                .map(|c| field(c.as_deref().unwrap_or("")))
                .collect(),
        );
    }
    out
}

impl DbPanel {
    pub fn modal_open(&self) -> bool {
        self.editor.is_some()
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
        self.send(Cmd::Query(sql));
    }

    fn connect(&mut self, store: &Store, ctx: &egui::Context) {
        let Some(profile) = store.databases.get(self.selected).cloned() else {
            return;
        };
        let mut ssh = None;
        if !profile.tunnel.is_empty() {
            let Some(host) = store.profiles.iter().find(|p| p.name == profile.tunnel) else {
                self.status = format!("Хост «{}» для туннеля не найден", profile.tunnel);
                self.error = true;
                return;
            };
            let key = format!("{}:{}", host.host.trim(), host.port);
            ssh = Some((host.clone(), store.known_hosts.get(&key).cloned()));
        }
        let (tx, cmds) = channel();
        let (events, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            serve(profile, ssh, cmds, events, &move || ctx.request_repaint())
        });
        self.current = store.databases[self.selected].database.trim().to_owned();
        self.conn = Some(Conn { tx, rx });
        self.version = None;
        self.schemas.clear();
        self.result = None;
        self.status = "Подключение…".into();
        self.error = false;
        self.busy = true;
    }

    fn disconnect(&mut self) {
        self.conn = None;
        self.version = None;
        self.schemas.clear();
        self.busy = false;
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
                    self.status = format!("Подключено, сервер {version}");
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
                    }
                }
                Event::Tables(db, tables) => {
                    if let Some(schema) = self.schemas.iter_mut().find(|s| s.name == db) {
                        schema.tables = Some(tables);
                    }
                }
                Event::Result(table) => {
                    self.status = if table.columns.is_empty() {
                        format!(
                            "Выполнено: затронуто строк {}, {:.1} мс. {}",
                            table.affected,
                            table.elapsed.as_secs_f64() * 1000.0,
                            table.info
                        )
                    } else {
                        format!(
                            "Строк: {}{}, {:.1} мс",
                            table.rows.len(),
                            if table.truncated {
                                format!(" (показаны первые {MAX_ROWS})")
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
                    self.result = Some(table);
                }
                Event::Error(e) => {
                    self.status = e;
                    self.error = true;
                }
                Event::Closed(e) => {
                    self.status = format!("Нет подключения: {e}");
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
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                ui.label(RichText::new("Подключений к базам пока нет.").color(pal.subtext));
                if ui
                    .add(Button::new("Создать подключение").fill(pal.surface))
                    .clicked()
                {
                    self.open_editor(store, true);
                }
            });
            return;
        }
        self.selected = self.selected.min(store.databases.len() - 1);
        let active = self.conn.is_some();
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            let mut edit = false;
            ui.add_enabled_ui(!active, |ui| {
                ComboBox::from_id_salt("tror_db_profile")
                    .width(220.0)
                    .selected_text(store.databases[self.selected].name.clone())
                    .show_ui(ui, |ui| {
                        for (i, profile) in store.databases.iter().enumerate() {
                            ui.selectable_value(&mut self.selected, i, profile.name.as_str());
                        }
                        ui.separator();
                        if ui.button("Настроить подключения…").clicked() {
                            edit = true;
                        }
                    });
            });
            if edit {
                self.open_editor(store, false);
            }
            let label = if active {
                "Отключить"
            } else {
                "Подключить"
            };
            if ui.add(Button::new(label).fill(pal.surface)).clicked() {
                if active {
                    self.disconnect();
                    self.status = "Отключено".into();
                    self.error = false;
                } else {
                    self.connect(store, &ctx);
                }
            }
            if self.busy {
                ui.spinner();
            }
            ui.add(
                egui::Label::new(RichText::new(&self.status).size(12.5).color(if self.error {
                    pal.warn
                } else {
                    pal.subtext
                }))
                .truncate(),
            );
        });
        ui.add_space(6.0);
        if self.version.is_none() {
            return;
        }
        let rect = ui.available_rect_before_wrap();
        let tree_w = (rect.width() * 0.24).clamp(150.0, 280.0);
        let (tree_rect, main_rect) = rect.split_left_right_at_x(rect.left() + tree_w);
        ui.painter().line_segment(
            [tree_rect.right_top(), tree_rect.right_bottom()],
            Stroke::new(1.0, pal.border),
        );
        let mut tree_ui =
            ui.new_child(egui::UiBuilder::new().max_rect(tree_rect.shrink2(egui::vec2(8.0, 0.0))));
        self.tree(&mut tree_ui, pal);
        let mut main_ui =
            ui.new_child(egui::UiBuilder::new().max_rect(main_rect.shrink2(egui::vec2(8.0, 0.0))));
        self.main(&mut main_ui, pal, font_size);
        ui.advance_cursor_after_rect(rect);
    }

    fn tree(&mut self, ui: &mut Ui, pal: &Palette) {
        let mut wanted = Vec::new();
        let mut query = None;
        let mut refresh = false;
        ui.horizontal(|ui| {
            ui.label(RichText::new("Базы и таблицы").size(12.0).color(pal.faint));
            if ui
                .small_button("↺")
                .on_hover_text("Обновить список")
                .clicked()
            {
                refresh = true;
            }
        });
        ScrollArea::vertical()
            .id_salt("tror_db_tree")
            .auto_shrink(false)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                for schema in &mut self.schemas {
                    let current = schema.name == self.current;
                    let mark = if schema.open { "▼" } else { "▶" };
                    let text = RichText::new(format!("{mark}  {}", schema.name))
                        .color(if current { pal.accent } else { pal.text });
                    if ui.add(Button::new(text).frame(false).truncate()).clicked() {
                        schema.open = !schema.open;
                        if schema.open {
                            self.current = schema.name.clone();
                            wanted.push(schema.name.clone());
                            query = Some((format!("USE {}", quote_name(&schema.name)), false));
                        }
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
                                let full =
                                    format!("{}.{}", quote_name(&schema.name), quote_name(table));
                                let response = ui.add(
                                    Button::new(
                                        RichText::new(format!("      {table}")).color(pal.subtext),
                                    )
                                    .frame(false)
                                    .truncate(),
                                );
                                if response.clicked() {
                                    query = Some((format!("SELECT * FROM {full} LIMIT 200"), true));
                                }
                                response.context_menu(|ui| {
                                    let items = [
                                        (
                                            "Первые 200 строк",
                                            format!("SELECT * FROM {full} LIMIT 200"),
                                        ),
                                        ("Столбцы", format!("SHOW FULL COLUMNS FROM {full}")),
                                        ("Индексы", format!("SHOW INDEX FROM {full}")),
                                        ("SQL создания", format!("SHOW CREATE TABLE {full}")),
                                        ("Число строк", format!("SELECT COUNT(*) FROM {full}")),
                                    ];
                                    for (label, sql) in items {
                                        if ui.button(label).clicked() {
                                            query = Some((sql, true));
                                            ui.close();
                                        }
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
        for db in wanted {
            self.send(Cmd::Tables(db));
        }
        if let Some((sql, show)) = query {
            if show {
                self.sql = sql.clone();
            }
            self.run(sql);
        }
    }

    fn main(&mut self, ui: &mut Ui, pal: &Palette, font_size: f32) {
        let font = FontId::monospace(font_size - 1.0);
        let run_key = ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Enter));
        let output = TextEdit::multiline(&mut self.sql)
            .id_salt("tror_db_sql")
            .font(font.clone())
            .hint_text(
                "SQL-запрос — Ctrl+Enter для выполнения (выделенный фрагмент выполняется отдельно)",
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
        ui.horizontal(|ui| {
            let run = ui
                .add_enabled(
                    !self.busy,
                    Button::new(RichText::new("Выполнить").color(pal.base)).fill(pal.accent),
                )
                .clicked();
            if (run || (run_key && output.response.has_focus())) && !self.busy {
                self.run(selected.unwrap_or_else(|| self.sql.clone()));
            }
            if !self.current.is_empty() {
                ui.label(
                    RichText::new(format!("база: {}", self.current))
                        .size(12.5)
                        .color(pal.subtext),
                );
            }
            if self.result.as_ref().is_some_and(|t| !t.columns.is_empty()) {
                if ui.button("Экспорт CSV…").clicked() {
                    export = Some(true);
                }
                if ui
                    .button("Копировать (TSV)")
                    .on_hover_text("Вся таблица результата в буфер обмена")
                    .clicked()
                {
                    export = Some(false);
                }
            }
        });
        if let (Some(to_file), Some(table)) = (export, &self.result) {
            if to_file {
                if let Some(path) = rfd::FileDialog::new()
                    .set_file_name("result.csv")
                    .save_file()
                {
                    match std::fs::write(&path, csv(table, ';')) {
                        Ok(()) => self.status = format!("Сохранено: {}", path.display()),
                        Err(e) => {
                            self.status = format!("Не сохранено: {e}");
                            self.error = true;
                        }
                    }
                }
            } else {
                ui.ctx().copy_text(csv(table, '\t'));
            }
        }
        ui.add_space(4.0);
        let Some(table) = &self.result else {
            return;
        };
        if table.columns.is_empty() {
            return;
        }
        let char_w = ui.fonts_mut(|f| f.glyph_width(&font, '0'));
        let row_h = (font_size * 1.7).round();
        let pad = 10.0;
        let widths: Vec<f32> = self.widths.iter().map(|w| w * char_w + pad * 2.0).collect();
        let number_w = 6.0 * char_w + pad;
        let total: f32 = number_w + widths.iter().sum::<f32>();
        let (header, _) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), row_h), Sense::hover());
        ui.painter().rect_filled(header, 0.0, pal.crust);
        let clip = ui.painter().with_clip_rect(header);
        let mut x = header.left() + number_w - self.scroll_x;
        for (name, w) in table.columns.iter().zip(&widths) {
            clip.text(
                egui::pos2(x + pad, header.center().y),
                Align2::LEFT_CENTER,
                name,
                font.clone(),
                pal.text,
            );
            x += w;
        }
        let rows = &table.rows;
        let mut copy = None;
        let mut picked = self.picked;
        ui.spacing_mut().item_spacing.y = 0.0;
        let out = ScrollArea::both()
            .id_salt("tror_db_grid")
            .auto_shrink(false)
            .show_rows(ui, row_h, rows.len(), |ui, range| {
                for index in range {
                    let (rect, response) = ui.allocate_exact_size(
                        egui::vec2(total.max(ui.available_width()), row_h),
                        Sense::click(),
                    );
                    let painter = ui.painter();
                    if index % 2 == 1 {
                        painter.rect_filled(rect, 0.0, pal.base);
                    }
                    if response.hovered() {
                        painter.rect_filled(rect, 0.0, pal.current_line);
                    }
                    painter.text(
                        egui::pos2(rect.left() + number_w - pad, rect.center().y),
                        Align2::RIGHT_CENTER,
                        (index + 1).to_string(),
                        font.clone(),
                        pal.faint,
                    );
                    let pointer = response.interact_pointer_pos().or_else(|| {
                        response
                            .secondary_clicked()
                            .then(|| ui.input(|i| i.pointer.interact_pos()))
                            .flatten()
                    });
                    let mut x = rect.left() + number_w;
                    for (col, (value, w)) in rows[index].iter().zip(&widths).enumerate() {
                        let cell = egui::Rect::from_min_size(
                            egui::pos2(x, rect.top()),
                            egui::vec2(*w, row_h),
                        );
                        if let Some(pos) = pointer
                            && cell.contains(pos)
                            && (response.clicked() || response.secondary_clicked())
                        {
                            picked = Some((index, col));
                        }
                        if picked == Some((index, col)) {
                            painter.rect_stroke(
                                cell.shrink(1.0),
                                3.0,
                                Stroke::new(1.0, pal.accent),
                                egui::StrokeKind::Inside,
                            );
                        }
                        let (text, color) = match value {
                            Some(text) => (text.as_str(), pal.text),
                            None => ("NULL", pal.faint),
                        };
                        let shown: String = text
                            .chars()
                            .take(self.widths[col] as usize)
                            .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
                            .collect();
                        painter
                            .with_clip_rect(cell.intersect(painter.clip_rect()))
                            .text(
                                egui::pos2(x + pad, rect.center().y),
                                Align2::LEFT_CENTER,
                                shown,
                                font.clone(),
                                color,
                            );
                        x += w;
                    }
                    response.context_menu(|ui| {
                        if let Some((r, c)) = picked
                            && r == index
                            && ui.button("Копировать значение").clicked()
                        {
                            copy = Some(rows[r][c].clone().unwrap_or_default());
                            ui.close();
                        }
                        if ui.button("Копировать строку").clicked() {
                            copy = Some(
                                rows[index]
                                    .iter()
                                    .map(|c| c.as_deref().unwrap_or("NULL"))
                                    .collect::<Vec<_>>()
                                    .join("\t"),
                            );
                            ui.close();
                        }
                    });
                }
            });
        self.scroll_x = out.state.clamped_offset().x;
        self.picked = picked;
        if let Some((r, c)) = picked
            && ui.memory(|m| m.focused().is_none())
            && ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Copy)))
        {
            copy = Some(rows[r][c].clone().unwrap_or_default());
        }
        if let Some(text) = copy {
            ui.ctx().copy_text(text);
        }
    }

    fn open_editor(&mut self, store: &Store, new: bool) {
        self.editor = Some(if new || store.databases.is_empty() {
            Editor {
                profile: DbProfile {
                    name: format!("База {}", store.databases.len() + 1),
                    ..Default::default()
                },
                index: None,
                show_password: false,
            }
        } else {
            Editor {
                profile: store.databases[self.selected].clone(),
                index: Some(self.selected),
                show_password: false,
            }
        });
    }

    pub fn dialogs(&mut self, ctx: &egui::Context, pal: &Palette, store: &mut Store) {
        let Some(editor) = &mut self.editor else {
            return;
        };
        let mut done = false;
        let mut save = false;
        let mut delete = false;
        let mut switch = None;
        let mut create = false;
        let modal = dialog("tror_db_editor", pal).show(ctx, |ui| {
            ui.set_width(440.0);
            ui.label(
                RichText::new("Подключение к базе данных")
                    .family(crate::theme::medium())
                    .size(16.0),
            );
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                for (i, profile) in store.databases.iter().enumerate() {
                    let mut button = Button::new(profile.name.as_str());
                    if editor.index == Some(i) {
                        button = button.fill(pal.surface);
                    }
                    if ui.add(button).clicked() {
                        switch = Some(i);
                    }
                }
                if ui.button("+ Новое").clicked() {
                    create = true;
                }
            });
            ui.add_space(8.0);
            let p = &mut editor.profile;
            egui::Grid::new("tror_db_editor_grid")
                .num_columns(2)
                .spacing([14.0, 8.0])
                .show(ui, |ui| {
                    let wide = 290.0;
                    ui.label("Название");
                    ui.add(TextEdit::singleline(&mut p.name).desired_width(wide));
                    ui.end_row();
                    ui.label("Тип");
                    ui.label(RichText::new("MySQL / MariaDB").color(pal.subtext));
                    ui.end_row();
                    ui.label("Туннель через SSH");
                    ComboBox::from_id_salt("tror_db_tunnel")
                        .width(wide)
                        .selected_text(if p.tunnel.is_empty() {
                            "нет, подключаться напрямую"
                        } else {
                            p.tunnel.as_str()
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut p.tunnel,
                                String::new(),
                                "нет, подключаться напрямую",
                            );
                            for host in &store.profiles {
                                if host.protocol == remote::Protocol::Sftp {
                                    ui.selectable_value(
                                        &mut p.tunnel,
                                        host.name.clone(),
                                        host.name.as_str(),
                                    );
                                }
                            }
                        });
                    ui.end_row();
                    ui.label(if p.tunnel.is_empty() {
                        "Сервер"
                    } else {
                        "Сервер (со стороны хоста)"
                    });
                    ui.horizontal(|ui| {
                        ui.add(TextEdit::singleline(&mut p.host).desired_width(wide - 78.0));
                        ui.add(egui::DragValue::new(&mut p.port).range(1..=65535));
                    });
                    ui.end_row();
                    ui.label("Пользователь");
                    ui.add(TextEdit::singleline(&mut p.user).desired_width(wide));
                    ui.end_row();
                    ui.label("Пароль");
                    ui.horizontal(|ui| {
                        ui.add(
                            TextEdit::singleline(&mut p.password)
                                .password(!editor.show_password)
                                .desired_width(wide - 96.0),
                        );
                        let label = if editor.show_password {
                            "Скрыть"
                        } else {
                            "Показать"
                        };
                        if ui.button(label).clicked() {
                            editor.show_password = !editor.show_password;
                        }
                    });
                    ui.end_row();
                    ui.label("База (необязательно)");
                    ui.add(TextEdit::singleline(&mut p.database).desired_width(wide));
                    ui.end_row();
                });
            ui.add_space(6.0);
            ui.label(
                RichText::new(
                    "Пароль хранится в файле профилей в замаскированном, но не зашифрованном виде. \
                     Соединение без TLS — для удалённых серверов используйте туннель через SSH.",
                )
                .size(12.0)
                .color(pal.faint),
            );
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if dialog_button(ui, pal, "Сохранить", true) {
                    save = true;
                }
                if dialog_button(ui, pal, "Закрыть", false) {
                    done = true;
                }
                if editor.index.is_some() && dialog_button(ui, pal, "Удалить", false) {
                    delete = true;
                }
            });
        });
        if let Some(i) = switch {
            self.selected = i;
            self.open_editor(store, false);
            return;
        }
        if create {
            self.open_editor(store, true);
            return;
        }
        if save && let Some(editor) = self.editor.take() {
            match editor.index {
                Some(i) if i < store.databases.len() => store.databases[i] = editor.profile,
                _ => {
                    store.databases.push(editor.profile);
                    self.selected = store.databases.len() - 1;
                }
            }
            if let Err(e) = store.save() {
                self.status = format!("Не удалось сохранить профили: {e}");
                self.error = true;
            }
            return;
        }
        if delete
            && let Some(Editor { index: Some(i), .. }) = self.editor.take()
            && i < store.databases.len()
        {
            store.databases.remove(i);
            self.selected = 0;
            let _ = store.save();
            return;
        }
        if done || modal.should_close() {
            self.editor = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_quoted_and_csv_is_escaped() {
        assert_eq!(quote_name("b_user"), "`b_user`");
        assert_eq!(quote_name("we`ird"), "`we``ird`");
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
    #[ignore]
    fn live_mysql_server() {
        let opts = mysql::OptsBuilder::new()
            .ip_or_hostname(Some("127.0.0.1"))
            .tcp_port(3307)
            .user(Some("jozef"))
            .pass(Some("secret"))
            .prefer_socket(false);
        let mut conn = mysql::Conn::new(opts).unwrap();
        let dbs = names(&mut conn, "SHOW DATABASES").unwrap();
        assert!(dbs.iter().any(|d| d == "shop"), "{dbs:?}");
        let table = run_query(&mut conn, "SELECT id, title, price FROM shop.goods").unwrap();
        assert_eq!(table.columns, ["id", "title", "price"]);
        assert_eq!(table.rows.len(), 3);
        assert_eq!(table.rows[1][1].as_deref(), Some("Чайник"));
        assert_eq!(table.rows[2][2], None);
        assert!(run_query(&mut conn, "SELEC nonsense").is_err());
    }
}
