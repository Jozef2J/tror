use std::collections::BTreeMap;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
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

pub static MAX_ROWS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(5000);

fn max_rows() -> usize {
    MAX_ROWS.load(std::sync::atomic::Ordering::Relaxed)
}
const MAX_CELL: usize = 4000;

#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Debug, Default)]
pub enum DbKind {
    #[default]
    MySql,
    Postgres,
    Sqlite,
}

impl DbKind {
    pub fn label(self) -> &'static str {
        match self {
            DbKind::MySql => "MySQL / MariaDB",
            DbKind::Postgres => "PostgreSQL",
            DbKind::Sqlite => "SQLite",
        }
    }

    fn default_port(self) -> u16 {
        match self {
            DbKind::Postgres => 5432,
            _ => 3306,
        }
    }

    pub fn quote(self, name: &str) -> String {
        match self {
            DbKind::MySql => format!("`{}`", name.replace('`', "``")),
            _ => format!("\"{}\"", name.replace('"', "\"\"")),
        }
    }

    pub fn literal(self, value: Option<&str>) -> String {
        match value {
            None => "NULL".into(),
            Some(text) => {
                let mut escaped = text.replace('\'', "''");
                if self == DbKind::MySql {
                    escaped = escaped.replace('\\', "\\\\");
                }
                format!("'{escaped}'")
            }
        }
    }
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
#[serde(default)]
pub struct DbProfile {
    pub name: String,
    pub kind: DbKind,
    pub host: String,
    pub port: u16,
    pub user: String,
    #[serde(with = "crate::remote::masked")]
    pub password: String,
    pub database: String,
    pub tunnel: String,
    pub file: String,
    pub tls: bool,
    pub tls_any_cert: bool,
}

impl Default for DbProfile {
    fn default() -> Self {
        Self {
            name: String::new(),
            kind: DbKind::MySql,
            host: "127.0.0.1".into(),
            port: 3306,
            user: String::new(),
            password: String::new(),
            database: String::new(),
            tunnel: String::new(),
            file: String::new(),
            tls: false,
            tls_any_cert: false,
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

impl Table {
    fn empty() -> Self {
        Table {
            columns: Vec::new(),
            rows: Vec::new(),
            affected: 0,
            info: String::new(),
            elapsed: Duration::ZERO,
            truncated: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Source {
    pub schema: String,
    pub table: String,
}

enum Cmd {
    Query(String, Option<Source>),
    Databases,
    Tables(String),
    Batch(Vec<String>, String, Option<Source>),
}

enum Event {
    Connected(String),
    Databases(Vec<String>),
    Tables(String, Vec<String>),
    Result(Table, Option<Source>, Vec<String>),
    Error(String),
    Closed(String),
}

struct Conn {
    tx: Sender<Cmd>,
    rx: Receiver<Event>,
}

fn clip(text: String) -> String {
    if text.len() > MAX_CELL {
        let cut = (0..=MAX_CELL)
            .rev()
            .find(|i| text.is_char_boundary(*i))
            .unwrap_or(0);
        format!("{}…", &text[..cut])
    } else {
        text
    }
}

fn cell(value: mysql::Value) -> Option<String> {
    use mysql::Value;
    Some(match value {
        Value::NULL => return None,
        Value::Bytes(bytes) => clip(String::from_utf8_lossy(&bytes).into_owned()),
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

fn lite_cell(value: rusqlite::types::ValueRef<'_>) -> Option<String> {
    use rusqlite::types::ValueRef;
    Some(match value {
        ValueRef::Null => return None,
        ValueRef::Integer(v) => v.to_string(),
        ValueRef::Real(v) => v.to_string(),
        ValueRef::Text(bytes) => clip(String::from_utf8_lossy(bytes).into_owned()),
        ValueRef::Blob(bytes) => format!("[BLOB, {} Б]", bytes.len()),
    })
}

fn explain(error: mysql::Error) -> String {
    match error {
        mysql::Error::MySqlError(e) => format!("ERROR {} ({}): {}", e.code, e.state, e.message),
        other => other.to_string(),
    }
}

fn explain_pg(error: postgres::Error) -> String {
    match error.as_db_error() {
        Some(db) => format!("ERROR {}: {}", db.code().code(), db.message()),
        None => error.to_string(),
    }
}

#[cfg(test)]
pub fn quote_name(name: &str) -> String {
    DbKind::MySql.quote(name)
}

enum Engine {
    My(Box<mysql::Conn>),
    Pg(Box<postgres::Client>),
    Lite(Box<rusqlite::Connection>),
}

impl Engine {
    fn alive(&mut self) -> bool {
        match self {
            Engine::My(conn) => conn.ping().is_ok(),
            Engine::Pg(client) => !client.is_closed() && client.simple_query("SELECT 1").is_ok(),
            Engine::Lite(_) => true,
        }
    }

    fn kind(&self) -> DbKind {
        match self {
            Engine::My(_) => DbKind::MySql,
            Engine::Pg(_) => DbKind::Postgres,
            Engine::Lite(_) => DbKind::Sqlite,
        }
    }

    fn query(&mut self, sql: &str) -> Result<Table, String> {
        let started = Instant::now();
        let mut table = Table::empty();
        match self {
            Engine::My(conn) => {
                let mut result = conn.query_iter(sql).map_err(explain)?;
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
                        if rows.len() >= max_rows() {
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
            }
            Engine::Pg(client) => {
                let messages = client.simple_query(sql).map_err(explain_pg)?;
                let mut columns: Vec<String> = Vec::new();
                let mut rows: Vec<Vec<Option<String>>> = Vec::new();
                let mut truncated = false;
                for message in messages {
                    match message {
                        postgres::SimpleQueryMessage::RowDescription(desc) => {
                            columns = desc.iter().map(|c| c.name().to_owned()).collect();
                            rows.clear();
                            truncated = false;
                        }
                        postgres::SimpleQueryMessage::Row(row) => {
                            if columns.is_empty() {
                                columns = row.columns().iter().map(|c| c.name().to_owned()).collect();
                            }
                            if rows.len() >= max_rows() {
                                truncated = true;
                                continue;
                            }
                            rows.push(
                                (0..row.len())
                                    .map(|i| row.get(i).map(|v| clip(v.to_owned())))
                                    .collect(),
                            );
                        }
                        postgres::SimpleQueryMessage::CommandComplete(count) => {
                            if columns.is_empty() {
                                table.affected += count;
                            } else {
                                table.columns = std::mem::take(&mut columns);
                                table.rows = std::mem::take(&mut rows);
                                table.truncated = truncated;
                            }
                        }
                        _ => {}
                    }
                }
            }
            Engine::Lite(conn) => {
                use rusqlite::fallible_iterator::FallibleIterator;
                let mut batch = rusqlite::Batch::new(conn, sql);
                while let Some(mut stmt) = batch.next().map_err(|e| e.to_string())? {
                    let count = stmt.column_count();
                    if count == 0 {
                        table.affected += stmt.raw_execute().map_err(|e| e.to_string())? as u64;
                        continue;
                    }
                    let columns: Vec<String> =
                        stmt.column_names().into_iter().map(str::to_owned).collect();
                    let mut rows = Vec::new();
                    let mut truncated = false;
                    let mut query = stmt.raw_query();
                    while let Some(row) = query.next().map_err(|e| e.to_string())? {
                        if rows.len() >= max_rows() {
                            truncated = true;
                            continue;
                        }
                        rows.push(
                            (0..count)
                                .map(|i| row.get_ref(i).ok().and_then(lite_cell))
                                .collect(),
                        );
                    }
                    table.columns = columns;
                    table.rows = rows;
                    table.truncated = truncated;
                }
                table.affected = table.affected.max(conn.changes());
            }
        }
        table.elapsed = started.elapsed();
        Ok(table)
    }

    fn first_column(&mut self, sql: &str) -> Result<Vec<String>, String> {
        Ok(self
            .query(sql)?
            .rows
            .into_iter()
            .filter_map(|row| row.into_iter().next().flatten())
            .collect())
    }

    fn databases(&mut self) -> Result<Vec<String>, String> {
        match self {
            Engine::My(_) => self.first_column("SHOW DATABASES"),
            Engine::Pg(_) => self.first_column(
                "SELECT schema_name FROM information_schema.schemata WHERE schema_name NOT LIKE 'pg\\_%' AND schema_name <> 'information_schema' ORDER BY 1",
            ),
            Engine::Lite(_) => {
                let table = self.query("PRAGMA database_list")?;
                Ok(table
                    .rows
                    .into_iter()
                    .filter_map(|row| row.get(1).cloned().flatten())
                    .collect())
            }
        }
    }

    fn tables(&mut self, schema: &str) -> Result<Vec<String>, String> {
        let kind = self.kind();
        match self {
            Engine::My(_) => self.first_column(&format!("SHOW TABLES FROM {}", kind.quote(schema))),
            Engine::Pg(_) => self.first_column(&format!(
                "SELECT table_name FROM information_schema.tables WHERE table_schema = {} ORDER BY 1",
                kind.literal(Some(schema))
            )),
            Engine::Lite(_) => self.first_column(&format!(
                "SELECT name FROM {}.sqlite_master WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\' ORDER BY 1",
                kind.quote(schema)
            )),
        }
    }

    fn primary_key(&mut self, source: &Source) -> Vec<String> {
        let kind = self.kind();
        let result = match self {
            Engine::My(_) => self
                .query(&format!(
                    "SHOW KEYS FROM {}.{} WHERE Key_name = 'PRIMARY'",
                    kind.quote(&source.schema),
                    kind.quote(&source.table)
                ))
                .map(|t| {
                    let column = t.columns.iter().position(|c| c == "Column_name").unwrap_or(4);
                    t.rows
                        .into_iter()
                        .filter_map(|r| r.get(column).cloned().flatten())
                        .collect()
                }),
            Engine::Pg(_) => self.first_column(&format!(
                "SELECT a.attname FROM pg_index i JOIN pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = ANY(i.indkey) WHERE i.indisprimary AND i.indrelid = {}::regclass ORDER BY array_position(i.indkey, a.attnum)",
                kind.literal(Some(&format!(
                    "{}.{}",
                    kind.quote(&source.schema),
                    kind.quote(&source.table)
                )))
            )),
            Engine::Lite(_) => self
                .query(&format!(
                    "PRAGMA {}.table_info({})",
                    kind.quote(&source.schema),
                    kind.quote(&source.table)
                ))
                .map(|t| {
                    let mut keys: Vec<(i64, String)> = t
                        .rows
                        .into_iter()
                        .filter_map(|r| {
                            let pk: i64 = r.get(5)?.as_deref()?.parse().ok()?;
                            (pk > 0).then(|| Some((pk, r.get(1)?.clone()?)))?
                        })
                        .collect();
                    keys.sort();
                    keys.into_iter().map(|(_, name)| name).collect()
                }),
        };
        result.unwrap_or_default()
    }

    fn batch(&mut self, statements: &[String]) -> Result<u64, String> {
        let begin = match self {
            Engine::My(_) => "START TRANSACTION",
            _ => "BEGIN",
        };
        self.query(begin)?;
        let mut affected = 0;
        for statement in statements {
            trace::client(format!("[DB] > {statement}"));
            match self.query(statement) {
                Ok(table) => affected += table.affected,
                Err(e) => {
                    let _ = self.query("ROLLBACK");
                    trace::line("[DB] ROLLBACK", true);
                    return Err(e);
                }
            }
        }
        self.query("COMMIT")?;
        Ok(affected)
    }
}

fn open_engine(profile: &DbProfile, host: String, port: u16) -> Result<(Engine, String), String> {
    match profile.kind {
        DbKind::MySql => {
            let database = profile.database.trim();
            let opts = mysql::OptsBuilder::new()
                .ip_or_hostname(Some(host))
                .tcp_port(port)
                .user(Some(profile.user.trim()))
                .pass(Some(profile.password.as_str()))
                .db_name((!database.is_empty()).then_some(database))
                .prefer_socket(false)
                .tcp_connect_timeout(Some(Duration::from_secs(12)));
            let loose = profile.tls_any_cert || !profile.tunnel.is_empty();
            let opts = if profile.tls {
                opts.ssl_opts(Some(
                    mysql::SslOpts::default()
                        .with_danger_accept_invalid_certs(profile.tls_any_cert)
                        .with_danger_skip_domain_validation(loose),
                ))
            } else {
                opts
            };
            let conn = mysql::Conn::new(opts).map_err(explain)?;
            let (a, b, c) = conn.server_version();
            Ok((Engine::My(Box::new(conn)), format!("MySQL {a}.{b}.{c}")))
        }
        DbKind::Postgres => {
            let mut config = postgres::Config::new();
            config
                .host(&host)
                .port(port)
                .user(profile.user.trim())
                .password(profile.password.as_str())
                .connect_timeout(Duration::from_secs(12));
            let database = profile.database.trim();
            if !database.is_empty() {
                config.dbname(database);
            }
            let mut client = if profile.tls {
                config.ssl_mode(postgres::config::SslMode::Require);
                let tls = native_tls::TlsConnector::builder()
                    .danger_accept_invalid_certs(profile.tls_any_cert)
                    .danger_accept_invalid_hostnames(
                        profile.tls_any_cert || !profile.tunnel.is_empty(),
                    )
                    .build()
                    .map_err(|e| e.to_string())?;
                config
                    .connect(postgres_native_tls::MakeTlsConnector::new(tls))
                    .map_err(explain_pg)?
            } else {
                config.connect(postgres::NoTls).map_err(explain_pg)?
            };
            let version = client
                .simple_query("SHOW server_version")
                .ok()
                .and_then(|messages| {
                    messages.into_iter().find_map(|m| match m {
                        postgres::SimpleQueryMessage::Row(row) => row.get(0).map(str::to_owned),
                        _ => None,
                    })
                })
                .unwrap_or_default();
            Ok((Engine::Pg(Box::new(client)), format!("PostgreSQL {version}")))
        }
        DbKind::Sqlite => {
            let path = profile.file.trim();
            if path.is_empty() {
                return Err("не выбран файл базы".into());
            }
            let conn = rusqlite::Connection::open(path).map_err(|e| e.to_string())?;
            let _ = conn.busy_timeout(Duration::from_secs(5));
            Ok((
                Engine::Lite(Box::new(conn)),
                format!("SQLite {}", rusqlite::version()),
            ))
        }
    }
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
            .map_err(|e| format!("туннель: {e}"))?;
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
                send(Event::Error(format!("Соединение потеряно, переподключиться не удалось: {e}")));
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
    profile: DbProfile,
    index: Option<usize>,
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
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct History {
    entries: Vec<(String, String)>,
    #[serde(skip)]
    path: Option<std::path::PathBuf>,
}

const HISTORY_MAX: usize = 300;

impl History {
    fn load(path: Option<std::path::PathBuf>) -> Self {
        let mut history: History = path
            .as_deref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        history.path = path;
        history
    }

    fn push(&mut self, profile: &str, sql: &str) {
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

    fn recent(&self, profile: &str) -> impl Iterator<Item = &str> {
        self.entries
            .iter()
            .rev()
            .filter(move |(p, _)| p == profile)
            .map(|(_, s)| s.as_str())
    }
}

fn json(table: &Table) -> String {
    let rows: Vec<serde_json::Value> = table
        .rows
        .iter()
        .map(|row| {
            serde_json::Value::Object(
                table
                    .columns
                    .iter()
                    .zip(row)
                    .map(|(name, cell)| {
                        let value = match cell {
                            Some(text) => serde_json::Value::String(text.clone()),
                            None => serde_json::Value::Null,
                        };
                        (name.clone(), value)
                    })
                    .collect(),
            )
        })
        .collect();
    serde_json::to_string_pretty(&rows).unwrap_or_default()
}

fn inserts(table: &Table, kind: DbKind, name: &str) -> String {
    let columns: Vec<String> = table.columns.iter().map(|c| kind.quote(c)).collect();
    let mut out = String::new();
    for row in &table.rows {
        let values: Vec<String> = row.iter().map(|c| kind.literal(c.as_deref())).collect();
        out.push_str(&format!(
            "INSERT INTO {} ({}) VALUES ({});\n",
            kind.quote(name),
            columns.join(", "),
            values.join(", ")
        ));
    }
    out
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

pub fn source_of(sql: &str, current: &str) -> Option<Source> {
    let text = sql.trim().trim_end_matches(';').trim();
    let lower = text.to_lowercase();
    let rest = lower.strip_prefix("select * from ")?;
    let offset = text.len() - rest.len();
    let original = &text[offset..];
    let name: String = original
        .chars()
        .take_while(|c| !c.is_whitespace())
        .collect();
    let tail = original[name.len()..].trim().to_lowercase();
    if !(tail.is_empty()
        || tail.starts_with("where ")
        || tail.starts_with("order by ")
        || tail.starts_with("limit "))
    {
        return None;
    }
    let clean = |part: &str| part.trim_matches(['`', '"', '[', ']']).to_owned();
    let mut parts = split_name(&name);
    let table = clean(&parts.pop()?);
    let schema = parts.pop().map(|p| clean(&p)).unwrap_or_else(|| current.to_owned());
    (!table.is_empty() && !schema.is_empty()).then_some(Source { schema, table })
}

fn split_name(name: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    for c in name.chars() {
        match (quote, c) {
            (None, '`' | '"') => {
                quote = Some(c);
                current.push(c);
            }
            (Some(q), c) if c == q => {
                quote = None;
                current.push(c);
            }
            (None, '.') => parts.push(std::mem::take(&mut current)),
            _ => current.push(c),
        }
    }
    parts.push(current);
    parts
}

impl DbPanel {
    pub fn modal_open(&self) -> bool {
        self.editor.is_some() || self.edits.confirm.is_some()
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

    fn connect(&mut self, store: &Store, ctx: &egui::Context) {
        let Some(profile) = store.databases.get(self.selected).cloned() else {
            return;
        };
        let mut ssh = None;
        if !profile.tunnel.is_empty() && profile.kind != DbKind::Sqlite {
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
        self.edits = Edits::default();
        self.status = "Подключение…".into();
        self.error = false;
        self.busy = true;
    }

    fn disconnect(&mut self) {
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
                    self.status = format!("Подключено: {version}");
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
                                format!(" (показаны первые {})", max_rows())
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
            crate::widgets::wrapped_row(ui, 8.0, |ui| {
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
        crate::widgets::wrapped_row(ui, 8.0, |ui| {
            let mut edit = false;
            let combo_w = (ui.available_width() - 150.0).clamp(120.0, 240.0);
            ui.add_enabled_ui(!active, |ui| {
                ComboBox::from_id_salt("tror_db_profile")
                    .width(combo_w)
                    .truncate()
                    .selected_text(store.databases[self.selected].name.clone())
                    .show_ui(ui, |ui| {
                        for (i, profile) in store.databases.iter().enumerate() {
                            let text = format!("{}  ·  {}", profile.name, profile.kind.label());
                            ui.selectable_value(&mut self.selected, i, text);
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
            let label = if active { "Отключить" } else { "Подключить" };
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
        let first = ("Первые 200 строк", format!("SELECT * FROM {full} LIMIT 200"));
        let count = ("Число строк", format!("SELECT COUNT(*) FROM {full}"));
        match kind {
            DbKind::MySql => vec![
                first,
                ("Столбцы", format!("SHOW FULL COLUMNS FROM {full}")),
                ("Индексы", format!("SHOW INDEX FROM {full}")),
                ("SQL создания", format!("SHOW CREATE TABLE {full}")),
                count,
            ],
            DbKind::Postgres => vec![
                first,
                (
                    "Столбцы",
                    format!(
                        "SELECT column_name, data_type, is_nullable, column_default FROM information_schema.columns WHERE table_schema = {} AND table_name = {} ORDER BY ordinal_position",
                        kind.literal(Some(schema)),
                        kind.literal(Some(table))
                    ),
                ),
                (
                    "Индексы",
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
                    "Столбцы",
                    format!("PRAGMA {}.table_info({})", kind.quote(schema), kind.quote(table)),
                ),
                (
                    "Индексы",
                    format!("PRAGMA {}.index_list({})", kind.quote(schema), kind.quote(table)),
                ),
                (
                    "SQL создания",
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
        let kind = self.kind;
        ui.horizontal(|ui| {
            let title = match kind {
                DbKind::MySql => "Базы и таблицы",
                DbKind::Postgres => "Схемы и таблицы",
                DbKind::Sqlite => "Таблицы",
            };
            ui.label(RichText::new(title).small().color(pal.faint));
            if ui
                .small_button("↺")
                .on_hover_text("Обновить список")
                .clicked()
            {
                refresh = true;
            }
        });
        let mut toggle = None;
        let mut run: Option<(String, String, bool)> = None;
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
                                    run = Some((schema.name.clone(), queries[0].1.clone(), true));
                                }
                                response.context_menu(|ui| {
                                    for (label, sql) in &queries {
                                        if ui.button(*label).clicked() {
                                            run = Some((schema.name.clone(), sql.clone(), true));
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
        if let Some((schema, sql, show)) = run {
            self.current = schema;
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
        let mut apply = false;
        let mut discard = false;
        let mut add_row = false;
        ui.horizontal_wrapped(|ui| {
            let run = ui
                .add_enabled(
                    !self.busy,
                    crate::widgets::primary("Выполнить", pal),
                )
                .clicked();
            if (run || (run_key && output.response.has_focus())) && !self.busy {
                self.run(selected.unwrap_or_else(|| self.sql.clone()));
            }
            let history = self.history.get_or_insert_with(|| {
                History::load(eframe::storage_dir("Tror").map(|d| d.join("db_history.json")))
            });
            let recent: Vec<String> = history.recent(&self.profile).take(40).map(str::to_owned).collect();
            ui.add_enabled_ui(!recent.is_empty(), |ui| {
                ui.menu_button("История", |ui| {
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
            if !self.current.is_empty() {
                let what = match self.kind {
                    DbKind::MySql => "база",
                    DbKind::Postgres => "схема",
                    DbKind::Sqlite => "база",
                };
                ui.label(
                    RichText::new(format!("{what}: {}", self.current))
                        .small()
                        .color(pal.subtext),
                );
            }
            if self.result.as_ref().is_some_and(|t| !t.columns.is_empty()) {
                if ui.button("Экспорт…").on_hover_text("В файл CSV, JSON или SQL — формат выбирается в окне сохранения").clicked() {
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
            if self.editable() {
                if ui
                    .button("+ Строка")
                    .on_hover_text("Добавить строку в таблицу")
                    .clicked()
                {
                    add_row = true;
                }
                let count = self.edits.count();
                if count > 0 {
                    if ui
                        .add(
                            crate::widgets::primary(format!("Применить изменения ({count})"), pal),
                        )
                        .clicked()
                    {
                        apply = true;
                    }
                    if ui.button("Отменить").clicked() {
                        discard = true;
                    }
                }
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
                    .add_filter("CSV, разделитель «;»", &["csv"])
                    .add_filter("JSON", &["json"])
                    .add_filter("SQL — команды INSERT", &["sql"])
                    .save_file()
                {
                    let text = match path.extension().and_then(|e| e.to_str()) {
                        Some(e) if e.eq_ignore_ascii_case("json") => json(table),
                        Some(e) if e.eq_ignore_ascii_case("sql") => inserts(table, self.kind, &name),
                        _ => csv(table, ';'),
                    };
                    match std::fs::write(&path, text) {
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
        if let Some(table) = &self.result
            && !table.columns.is_empty()
            && self.source.is_some()
            && self.keys.is_empty()
        {
            ui.label(
                RichText::new("У таблицы нет первичного ключа — правка в сетке недоступна, меняйте данные запросами.")
                    .small()
                    .color(pal.faint),
            );
        } else if self.editable() {
            ui.label(
                RichText::new("Двойной щелчок по ячейке — правка, Enter — принять, Esc — отмена. Правый щелчок — NULL и удаление строки.")
                    .small()
                    .color(pal.faint),
            );
        }
        ui.add_space(4.0);
        self.grid(ui, pal, &font, font_size);
    }

    fn editable(&self) -> bool {
        self.source.is_some()
            && !self.keys.is_empty()
            && self
                .result
                .as_ref()
                .is_some_and(|t| !t.columns.is_empty() && !t.truncated)
    }

    fn statements(&self) -> Vec<String> {
        let (Some(table), Some(source)) = (&self.result, &self.source) else {
            return Vec::new();
        };
        let kind = self.kind;
        let full = format!("{}.{}", kind.quote(&source.schema), kind.quote(&source.table));
        let key_index: Vec<usize> = self
            .keys
            .iter()
            .filter_map(|k| table.columns.iter().position(|c| c == k))
            .collect();
        let condition = |row: &[Option<String>]| {
            key_index
                .iter()
                .map(|&i| match &row[i] {
                    Some(value) => format!("{} = {}", kind.quote(&table.columns[i]), kind.literal(Some(value))),
                    None => format!("{} IS NULL", kind.quote(&table.columns[i])),
                })
                .collect::<Vec<_>>()
                .join(" AND ")
        };
        let mut out = Vec::new();
        let mut by_row: BTreeMap<usize, Vec<(usize, &Option<String>)>> = BTreeMap::new();
        for ((row, col), value) in &self.edits.cells {
            if !self.edits.deleted.contains(row) {
                by_row.entry(*row).or_default().push((*col, value));
            }
        }
        for (row, changes) in by_row {
            let Some(original) = table.rows.get(row) else {
                continue;
            };
            let set: Vec<String> = changes
                .iter()
                .map(|(col, value)| {
                    format!(
                        "{} = {}",
                        kind.quote(&table.columns[*col]),
                        kind.literal(value.as_deref())
                    )
                })
                .collect();
            out.push(format!(
                "UPDATE {full} SET {} WHERE {}",
                set.join(", "),
                condition(original)
            ));
        }
        for row in &self.edits.deleted {
            if let Some(original) = table.rows.get(*row) {
                out.push(format!("DELETE FROM {full} WHERE {}", condition(original)));
            }
        }
        for added in &self.edits.added {
            let pairs: Vec<(usize, &String)> = added
                .iter()
                .enumerate()
                .filter_map(|(i, v)| v.as_ref().map(|v| (i, v)))
                .collect();
            if pairs.is_empty() {
                out.push(match kind {
                    DbKind::MySql => format!("INSERT INTO {full} () VALUES ()"),
                    _ => format!("INSERT INTO {full} DEFAULT VALUES"),
                });
                continue;
            }
            let columns: Vec<String> = pairs.iter().map(|(i, _)| kind.quote(&table.columns[*i])).collect();
            let values: Vec<String> = pairs.iter().map(|(_, v)| kind.literal(Some(v))).collect();
            out.push(format!(
                "INSERT INTO {full} ({}) VALUES ({})",
                columns.join(", "),
                values.join(", ")
            ));
        }
        out
    }

    fn grid(&mut self, ui: &mut Ui, pal: &Palette, font: &FontId, font_size: f32) {
        let editable = self.editable();
        let Some(table) = &self.result else {
            return;
        };
        if table.columns.is_empty() {
            return;
        }
        let char_w = ui.fonts_mut(|f| f.glyph_width(font, '0'));
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
            let key = self.keys.contains(name);
            clip.text(
                egui::pos2(x + pad, header.center().y),
                Align2::LEFT_CENTER,
                if key { format!("🔑{name}") } else { name.clone() },
                font.clone(),
                if key { pal.accent } else { pal.text },
            );
            x += w;
        }
        let base_rows = table.rows.len();
        let total_rows = base_rows + self.edits.added.len();
        let mut copy = None;
        let mut picked = self.picked;
        let mut start_edit = None;
        let mut set_null = None;
        let mut toggle_delete = None;
        let mut commit = None;
        let mut cancel_edit = false;
        let changed_color = pal.match_bg;
        ui.spacing_mut().item_spacing.y = 0.0;
        let out = ScrollArea::both()
            .id_salt("tror_db_grid")
            .auto_shrink(false)
            .show_rows(ui, row_h, total_rows, |ui, range| {
                for index in range {
                    let added = index >= base_rows;
                    let row_values: Vec<Option<String>> = if added {
                        self.edits.added[index - base_rows].clone()
                    } else {
                        table.rows[index].clone()
                    };
                    let (rect, response) = ui.allocate_exact_size(
                        egui::vec2(total.max(ui.available_width()), row_h),
                        Sense::click(),
                    );
                    let painter = ui.painter().clone();
                    let deleted = !added && self.edits.deleted.contains(&index);
                    if index % 2 == 1 {
                        painter.rect_filled(rect, 0.0, pal.base);
                    }
                    if added {
                        painter.rect_filled(rect, 0.0, pal.selection.gamma_multiply(0.45));
                    }
                    if response.hovered() {
                        painter.rect_filled(rect, 0.0, pal.current_line);
                    }
                    painter.text(
                        egui::pos2(rect.left() + number_w - pad, rect.center().y),
                        Align2::RIGHT_CENTER,
                        if added { "+".to_owned() } else { (index + 1).to_string() },
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
                    for (col, w) in widths.iter().enumerate() {
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
                        if let Some(pos) = pointer
                            && cell.contains(pos)
                            && response.double_clicked()
                            && editable
                            && !deleted
                        {
                            start_edit = Some((index, col));
                        }
                        let edited = if added {
                            None
                        } else {
                            self.edits.cells.get(&(index, col))
                        };
                        if edited.is_some() {
                            painter.rect_filled(cell.shrink(1.0), 2.0, changed_color);
                        }
                        let value = match edited {
                            Some(v) => v.clone(),
                            None => row_values.get(col).cloned().flatten(),
                        };
                        let editing_here = self
                            .edits
                            .editing
                            .as_ref()
                            .is_some_and(|(r, c, _)| *r == index && *c == col);
                        if editing_here {
                            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(cell.shrink(2.0)));
                            if let Some((_, _, text)) = &mut self.edits.editing {
                                let field = child.add(
                                    TextEdit::singleline(text)
                                        .font(font.clone())
                                        .desired_width(cell.width() - 6.0)
                                        .margin(egui::Margin::symmetric(4, 2)),
                                );
                                field.request_focus();
                                if child.input(|i| i.key_pressed(Key::Enter)) {
                                    commit = Some((index, col, text.clone()));
                                } else if child.input(|i| i.key_pressed(Key::Escape)) {
                                    cancel_edit = true;
                                } else if field.lost_focus() {
                                    commit = Some((index, col, text.clone()));
                                }
                            }
                        } else {
                            if picked == Some((index, col)) {
                                painter.rect_stroke(
                                    cell.shrink(1.0),
                                    3.0,
                                    Stroke::new(1.0, pal.accent),
                                    egui::StrokeKind::Inside,
                                );
                            }
                            let (text, color) = match &value {
                                Some(text) => (text.as_str(), if deleted { pal.faint } else { pal.text }),
                                None => ("NULL", pal.faint),
                            };
                            let shown: String = text
                                .chars()
                                .take(self.widths[col] as usize)
                                .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
                                .collect();
                            let galley = painter
                                .with_clip_rect(cell.intersect(painter.clip_rect()))
                                .text(
                                    egui::pos2(x + pad, rect.center().y),
                                    Align2::LEFT_CENTER,
                                    shown,
                                    font.clone(),
                                    color,
                                );
                            if deleted {
                                painter.line_segment(
                                    [galley.left_center(), galley.right_center()],
                                    Stroke::new(1.0, pal.warn),
                                );
                            }
                        }
                        x += w;
                    }
                    response.context_menu(|ui| {
                        if let Some((r, c)) = picked
                            && r == index
                        {
                            if ui.button("Копировать значение").clicked() {
                                let value = if added {
                                    self.edits.added[r - base_rows][c].clone()
                                } else {
                                    self.edits
                                        .cells
                                        .get(&(r, c))
                                        .cloned()
                                        .unwrap_or_else(|| table.rows[r][c].clone())
                                };
                                copy = Some(value.unwrap_or_default());
                                ui.close();
                            }
                            if editable && !deleted {
                                if ui.button("Изменить значение").clicked() {
                                    start_edit = Some((r, c));
                                    ui.close();
                                }
                                if ui.button("Сделать NULL").clicked() {
                                    set_null = Some((r, c));
                                    ui.close();
                                }
                            }
                        }
                        if ui.button("Копировать строку").clicked() {
                            copy = Some(
                                row_values
                                    .iter()
                                    .map(|c| c.as_deref().unwrap_or("NULL"))
                                    .collect::<Vec<_>>()
                                    .join("\t"),
                            );
                            ui.close();
                        }
                        if editable {
                            ui.separator();
                            let label = if added {
                                "Убрать новую строку"
                            } else if deleted {
                                "Не удалять строку"
                            } else {
                                "Удалить строку"
                            };
                            if ui.button(label).clicked() {
                                toggle_delete = Some(index);
                                ui.close();
                            }
                        }
                    });
                }
            });
        self.scroll_x = out.state.clamped_offset().x;
        self.picked = picked;
        if let Some((row, col)) = start_edit {
            let current = if row >= base_rows {
                self.edits.added[row - base_rows][col].clone()
            } else {
                self.edits
                    .cells
                    .get(&(row, col))
                    .cloned()
                    .unwrap_or_else(|| table.rows[row][col].clone())
            };
            let long = current.as_ref().is_some_and(|v| v.chars().count() > MAX_CELL);
            if long {
                self.status = "Значение длиннее 4000 символов показано не целиком — измените его запросом".into();
                self.error = true;
            } else if current.as_deref().is_some_and(|v| v.starts_with("[BLOB")) {
                self.status = "Двоичные данные в сетке не правятся".into();
                self.error = true;
            } else {
                self.edits.editing = Some((row, col, current.unwrap_or_default()));
            }
        }
        if cancel_edit {
            self.edits.editing = None;
        }
        if let Some((row, col, text)) = commit {
            self.edits.editing = None;
            self.set_cell(row, col, Some(text));
        }
        if let Some((row, col)) = set_null {
            self.set_cell(row, col, None);
        }
        if let Some(row) = toggle_delete {
            if row >= base_rows {
                self.edits.added.remove(row - base_rows);
            } else if let Some(i) = self.edits.deleted.iter().position(|r| *r == row) {
                self.edits.deleted.remove(i);
            } else {
                self.edits.deleted.push(row);
            }
        }
        if let Some((r, c)) = picked
            && self.edits.editing.is_none()
            && ui.memory(|m| m.focused().is_none())
            && ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Copy)))
            && let Some(table) = &self.result
            && r < table.rows.len()
        {
            copy = Some(table.rows[r][c].clone().unwrap_or_default());
        }
        if let Some(text) = copy {
            ui.ctx().copy_text(text);
        }
    }

    fn set_cell(&mut self, row: usize, col: usize, value: Option<String>) {
        let Some(table) = &self.result else {
            return;
        };
        if row >= table.rows.len() {
            if let Some(added) = self.edits.added.get_mut(row - table.rows.len()) {
                added[col] = value;
            }
            return;
        }
        if table.rows[row][col] == value {
            self.edits.cells.remove(&(row, col));
        } else {
            self.edits.cells.insert((row, col), value);
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
                probe: None,
                result: None,
            }
        } else {
            Editor {
                profile: store.databases[self.selected].clone(),
                index: Some(self.selected),
                show_password: false,
                probe: None,
                result: None,
            }
        });
    }

    pub fn dialogs(&mut self, ctx: &egui::Context, pal: &Palette, store: &mut Store) {
        self.confirm_dialog(ctx, pal);
        let Some(editor) = &mut self.editor else {
            return;
        };
        let mut done = false;
        let mut save = false;
        let mut delete = false;
        let mut switch = None;
        let mut create = false;
        let mut test = false;
        if let Some(rx) = &editor.probe
            && let Ok(result) = rx.try_recv()
        {
            crate::trace::popup(
                match &result {
                    Ok(text) => format!("Подключение работает: {text}"),
                    Err(e) => format!("Не удалось подключиться: {e}"),
                },
                result.is_ok(),
            );
            editor.result = Some(result);
            editor.probe = None;
        }
        let modal = dialog("tror_db_editor", pal).show(ctx, |ui| {
            ui.set_width(460.0);
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
                    let wide = 300.0;
                    ui.label("Название");
                    ui.add(TextEdit::singleline(&mut p.name).desired_width(wide));
                    ui.end_row();
                    ui.label("Тип");
                    ui.horizontal(|ui| {
                        for kind in [DbKind::MySql, DbKind::Postgres, DbKind::Sqlite] {
                            if ui.selectable_label(p.kind == kind, kind.label()).clicked() && p.kind != kind {
                                if p.port == p.kind.default_port() {
                                    p.port = kind.default_port();
                                }
                                p.kind = kind;
                            }
                        }
                    });
                    ui.end_row();
                    if p.kind == DbKind::Sqlite {
                        ui.label("Файл базы");
                        ui.horizontal(|ui| {
                            ui.add(TextEdit::singleline(&mut p.file).desired_width(wide - 78.0));
                            if ui.button("Обзор…").clicked()
                                && let Some(file) = rfd::FileDialog::new()
                                    .add_filter("SQLite", &["db", "sqlite", "sqlite3", "db3"])
                                    .add_filter("Все файлы", &["*"])
                                    .pick_file()
                            {
                                p.file = file.display().to_string();
                            }
                        });
                        ui.end_row();
                        return;
                    }
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
                    ui.label("Шифрование TLS");
                    ui.vertical(|ui| {
                        ui.checkbox(&mut p.tls, "Подключаться по TLS");
                        if p.tls {
                            ui.checkbox(
                                &mut p.tls_any_cert,
                                "Принимать любой сертификат (самоподписанный)",
                            );
                        }
                    });
                    ui.end_row();
                    ui.label(if p.tunnel.is_empty() {
                        "Сервер"
                    } else {
                        "Сервер (со стороны хоста)"
                    });
                    ui.horizontal(|ui| {
                        ui.add(TextEdit::singleline(&mut p.host).desired_width(wide - 70.0));
                        crate::widgets::port_field(ui, &mut p.port, 60.0);
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
            ui.add_space(4.0);
            if editor.probe.is_some() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new("проверка…").color(pal.subtext));
                });
            } else if ui.add(Button::new("Проверить подключение").fill(pal.surface)).clicked() {
                test = true;
            }
            if let Some(result) = &editor.result {
                let (text, color) = match result {
                    Ok(text) => (format!("Работает: {text}"), egui::Color32::from_rgb(0x3f, 0xc2, 0x6e)),
                    Err(e) => (format!("Ошибка: {e}"), pal.warn),
                };
                ui.label(RichText::new(text).small().color(color));
            }
            ui.add_space(6.0);
            if p.kind == DbKind::Sqlite {
                ui.label(
                    RichText::new("Файл SQLite открывается напрямую с этого компьютера. Если файла нет, он будет создан.")
                        .small()
                        .color(pal.subtext),
                );
            } else {
                let sftp_hosts = store
                    .profiles
                    .iter()
                    .filter(|h| h.protocol == remote::Protocol::Sftp)
                    .count();
                let default_port = p.kind.default_port();
                let tunnel_note = if sftp_hosts == 0 {
                    "Туннель через SSH нужен, когда база принимает подключения только с самого сервера — \
                     так устроен почти любой хостинг. В списке туннелей появляются ваши SFTP-профили из панели \
                     «Удалённые хосты» (F8); сейчас там нет ни одного SFTP-профиля, поэтому выбрать нечего."
                        .to_owned()
                } else if p.tunnel.is_empty() {
                    "Туннель через SSH нужен, когда база принимает подключения только с самого сервера — \
                     так устроен почти любой хостинг. Выберите в списке SFTP-профиль этого сервера: программа \
                     зайдёт на него по SSH и уже оттуда подключится к базе."
                        .to_owned()
                } else {
                    format!(
                        "Программа зайдёт на выбранный хост по SSH и подключится к базе оттуда. В поле «Сервер» \
                         укажите адрес базы так, как его видит сам хост — обычно 127.0.0.1 или localhost, порт {default_port}."
                    )
                };
                ui.label(RichText::new(tunnel_note).small().color(pal.subtext));
                ui.add_space(4.0);
                ui.label(
                    RichText::new(
                        "Пароль хранится в файле профилей в замаскированном, но не зашифрованном виде. \
                         Без TLS и без туннеля пароль и данные идут по сети открыто.",
                    )
                    .small()
                    .color(pal.faint),
                );
            }
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
        if test {
            let profile = editor.profile.clone();
            let ssh = (!profile.tunnel.is_empty() && profile.kind != DbKind::Sqlite)
                .then(|| store.profiles.iter().find(|p| p.name == profile.tunnel))
                .flatten()
                .map(|host| {
                    let key = format!("{}:{}", host.host.trim(), host.port);
                    (host.clone(), store.known_hosts.get(&key).cloned())
                });
            let (tx, rx) = channel();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let _ = tx.send(test_profile(profile, ssh));
                ctx.request_repaint();
            });
            editor.probe = Some(rx);
            editor.result = None;
        }
        if editor.probe.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
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

    fn confirm_dialog(&mut self, ctx: &egui::Context, pal: &Palette) {
        let Some(statements) = &self.edits.confirm else {
            return;
        };
        let mut run = false;
        let mut cancel = false;
        let modal = dialog("tror_db_apply", pal).show(ctx, |ui| {
            ui.set_width(560.0);
            ui.label(RichText::new("Применить изменения").strong());
            ui.label(
                RichText::new("Будут выполнены запросы, в одной транзакции:")
                    .color(pal.subtext)
                    .small(),
            );
            ui.add_space(6.0);
            ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                for statement in statements {
                    ui.add(
                        egui::Label::new(RichText::new(statement).monospace().size(12.0)).wrap(),
                    );
                    ui.add_space(3.0);
                }
            });
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if dialog_button(ui, pal, "Выполнить", true) {
                    run = true;
                }
                if dialog_button(ui, pal, "Отмена", false) {
                    cancel = true;
                }
            });
        });
        if run && let Some(statements) = self.edits.confirm.take() {
            let refresh = self.last_sql.clone();
            let source = self.source.clone();
            self.send(Cmd::Batch(statements, refresh, source));
        } else if cancel || modal.should_close() {
            self.edits.confirm = None;
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
