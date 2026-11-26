use crate::trace;
use std::time::{Duration, Instant};

use mysql::prelude::Queryable;
use crate::remote::{DbKind, DbProfile};

pub static MAX_ROWS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(5000);

pub(super) fn max_rows() -> usize {
    MAX_ROWS.load(std::sync::atomic::Ordering::Relaxed)
}
pub(super) const MAX_CELL: usize = 4000;

impl DbKind {
    pub fn label(self) -> &'static str {
        match self {
            DbKind::MySql => "MySQL / MariaDB",
            DbKind::Postgres => "PostgreSQL",
            DbKind::Sqlite => "SQLite",
        }
    }

    pub(super) fn default_port(self) -> u16 {
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


pub struct Table {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Option<String>>>,
    pub affected: u64,
    pub info: String,
    pub elapsed: Duration,
    pub truncated: bool,
}

impl Table {
    pub(super) fn empty() -> Self {
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

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Source {
    pub schema: String,
    pub table: String,
}

pub(super) fn clip(text: String) -> String {
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

pub(super) fn cell(value: mysql::Value) -> Option<String> {
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

pub(super) fn lite_cell(value: rusqlite::types::ValueRef<'_>) -> Option<String> {
    use rusqlite::types::ValueRef;
    Some(match value {
        ValueRef::Null => return None,
        ValueRef::Integer(v) => v.to_string(),
        ValueRef::Real(v) => v.to_string(),
        ValueRef::Text(bytes) => clip(String::from_utf8_lossy(bytes).into_owned()),
        ValueRef::Blob(bytes) => tf!("[BLOB, {} Б]", "[BLOB, {} B]", bytes.len()),
    })
}

pub(super) fn explain(error: mysql::Error) -> String {
    match error {
        mysql::Error::MySqlError(e) => format!("ERROR {} ({}): {}", e.code, e.state, e.message),
        other => other.to_string(),
    }
}

pub(super) fn explain_pg(error: postgres::Error) -> String {
    match error.as_db_error() {
        Some(db) => format!("ERROR {}: {}", db.code().code(), db.message()),
        None => error.to_string(),
    }
}

#[cfg(test)]
pub fn quote_name(name: &str) -> String {
    DbKind::MySql.quote(name)
}

pub(super) enum Engine {
    My(Box<mysql::Conn>),
    Pg(Box<postgres::Client>),
    Lite(Box<rusqlite::Connection>),
}

impl Engine {
    pub(super) fn alive(&mut self) -> bool {
        match self {
            Engine::My(conn) => conn.ping().is_ok(),
            Engine::Pg(client) => !client.is_closed() && client.simple_query("SELECT 1").is_ok(),
            Engine::Lite(_) => true,
        }
    }

    pub(super) fn kind(&self) -> DbKind {
        match self {
            Engine::My(_) => DbKind::MySql,
            Engine::Pg(_) => DbKind::Postgres,
            Engine::Lite(_) => DbKind::Sqlite,
        }
    }

    pub(super) fn query(&mut self, sql: &str) -> Result<Table, String> {
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

    pub(super) fn first_column(&mut self, sql: &str) -> Result<Vec<String>, String> {
        Ok(self
            .query(sql)?
            .rows
            .into_iter()
            .filter_map(|row| row.into_iter().next().flatten())
            .collect())
    }

    pub(super) fn databases(&mut self) -> Result<Vec<String>, String> {
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

    pub(super) fn tables(&mut self, schema: &str) -> Result<Vec<String>, String> {
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

    pub(super) fn primary_key(&mut self, source: &Source) -> Vec<String> {
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

    pub(super) fn import(&mut self, statements: &[String]) -> Result<u64, String> {
        let begin = match self {
            Engine::My(_) => "START TRANSACTION",
            _ => "BEGIN",
        };
        self.query(begin)?;
        let mut affected = 0;
        for (index, statement) in statements.iter().enumerate() {
            match self.query(statement) {
                Ok(table) => affected += table.affected,
                Err(e) => {
                    let _ = self.query("ROLLBACK");
                    let head: String = statement.chars().take(200).collect();
                    trace::line(format!("[DB] ROLLBACK: {} #{}: {head}", e, index + 1), true);
                    return Err(tf!(
                        "Запрос №{} не выполнен, всё отменено: {e}",
                        "Statement #{} failed, everything was rolled back: {e}",
                        index + 1
                    ));
                }
            }
        }
        self.query("COMMIT")?;
        Ok(affected)
    }

    pub(super) fn batch(&mut self, statements: &[String]) -> Result<u64, String> {
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

pub(super) fn open_engine(profile: &DbProfile, host: String, port: u16) -> Result<(Engine, String), String> {
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
                return Err(tr!("не выбран файл базы").into());
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
