use eframe::egui::{
    self, Align2, Button, ComboBox, FontId, RichText, ScrollArea, Sense, TextEdit, Ui,
};

use super::{Cmd, DbKind, DbPanel, Engine, Source, Table};
use crate::app::{dialog, dialog_button};
use crate::theme::Palette;
use crate::widgets::Lit;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Column {
    pub name: String,
    pub kind: String,
    pub null: bool,
    pub default: Option<String>,
    pub key: bool,
    pub auto: bool,
    pub comment: String,
    pub extra: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Index {
    pub name: String,
    pub columns: Vec<String>,
    pub unique: bool,
    pub primary: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Relation {
    pub name: String,
    pub columns: Vec<String>,
    pub target: String,
    pub target_columns: Vec<String>,
    pub on_delete: String,
    pub on_update: String,
}

#[derive(Clone, Debug, Default)]
pub struct Structure {
    pub source: Source,
    pub rows: Option<(u64, bool)>,
    pub columns: Vec<Column>,
    pub indexes: Vec<Index>,
    pub relations: Vec<Relation>,
    pub create: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Draft {
    pub name: String,
    pub kind: String,
    pub null: bool,
    pub default: String,
    pub comment: String,
}

impl Draft {
    fn of(column: &Column) -> Self {
        Draft {
            name: column.name.clone(),
            kind: column.kind.clone(),
            null: column.null,
            default: column.default.clone().unwrap_or_default(),
            comment: column.comment.clone(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Default)]
enum Tab {
    #[default]
    Columns,
    Indexes,
    Relations,
    Create,
}

struct ColumnEdit {
    original: Option<Column>,
    draft: Draft,
}

#[derive(Default)]
pub struct State {
    pub data: Option<Structure>,
    tab: Tab,
    pub picked: Option<usize>,
    edit: Option<ColumnEdit>,
    drop: Option<(String, Vec<String>)>,
    pub loading: Option<Source>,
    asked: Option<Source>,
}

fn at(table: &Table, row: &[Option<String>], name: &str) -> Option<String> {
    let index = table
        .columns
        .iter()
        .position(|c| c.eq_ignore_ascii_case(name))?;
    row.get(index).cloned().flatten()
}

fn text(table: &Table, row: &[Option<String>], name: &str) -> String {
    at(table, row, name).unwrap_or_default()
}

fn truthy(value: &str) -> bool {
    matches!(value, "1" | "t" | "true" | "YES" | "yes")
}

fn group_relations(rows: Vec<(String, String, String, String, String, String)>) -> Vec<Relation> {
    let mut out: Vec<Relation> = Vec::new();
    for (name, column, target, target_column, on_delete, on_update) in rows {
        match out.iter_mut().find(|r| r.name == name) {
            Some(relation) => {
                relation.columns.push(column);
                relation.target_columns.push(target_column);
            }
            None => out.push(Relation {
                name,
                columns: vec![column],
                target,
                target_columns: vec![target_column],
                on_delete,
                on_update,
            }),
        }
    }
    out
}

fn mysql_default(raw: Option<String>, extra: &str) -> Option<String> {
    let raw = raw?;
    if raw == "NULL" {
        return None;
    }
    let bare = raw.starts_with('\'')
        || raw.contains('(')
        || raw.parse::<f64>().is_ok()
        || extra.contains("DEFAULT_GENERATED")
        || raw.eq_ignore_ascii_case("CURRENT_TIMESTAMP");
    Some(if bare {
        raw
    } else {
        DbKind::MySql.literal(Some(&raw))
    })
}

fn pg_rule(code: &str) -> String {
    match code {
        "r" => "RESTRICT",
        "c" => "CASCADE",
        "n" => "SET NULL",
        "d" => "SET DEFAULT",
        _ => "NO ACTION",
    }
    .into()
}

impl Engine {
    pub(super) fn structure(&mut self, source: &Source) -> Result<Structure, String> {
        let kind = self.kind();
        let full = format!(
            "{}.{}",
            kind.quote(&source.schema),
            kind.quote(&source.table)
        );
        let mut out = Structure {
            source: source.clone(),
            ..Default::default()
        };
        match self {
            Engine::My(_) => {
                let schema = kind.literal(Some(&source.schema));
                let name = kind.literal(Some(&source.table));
                let filter = format!("TABLE_SCHEMA = {schema} AND TABLE_NAME = {name}");
                let t = self.query(&format!(
                    "SELECT COLUMN_NAME, COLUMN_TYPE, IS_NULLABLE, COLUMN_DEFAULT, COLUMN_KEY, EXTRA, COLUMN_COMMENT FROM information_schema.COLUMNS WHERE {filter} ORDER BY ORDINAL_POSITION"
                ))?;
                for row in &t.rows {
                    let extra = text(&t, row, "EXTRA");
                    out.columns.push(Column {
                        name: text(&t, row, "COLUMN_NAME"),
                        kind: text(&t, row, "COLUMN_TYPE"),
                        null: text(&t, row, "IS_NULLABLE") == "YES",
                        default: mysql_default(at(&t, row, "COLUMN_DEFAULT"), &extra),
                        key: text(&t, row, "COLUMN_KEY") == "PRI",
                        auto: extra.to_ascii_lowercase().contains("auto_increment"),
                        comment: text(&t, row, "COLUMN_COMMENT"),
                        extra,
                    });
                }
                let t = self.query(&format!(
                    "SELECT INDEX_NAME, NON_UNIQUE, COLUMN_NAME FROM information_schema.STATISTICS WHERE {filter} ORDER BY INDEX_NAME = 'PRIMARY' DESC, INDEX_NAME, SEQ_IN_INDEX"
                ))?;
                for row in &t.rows {
                    let name = text(&t, row, "INDEX_NAME");
                    let column = text(&t, row, "COLUMN_NAME");
                    match out.indexes.iter_mut().find(|i| i.name == name) {
                        Some(index) => index.columns.push(column),
                        None => out.indexes.push(Index {
                            primary: name == "PRIMARY",
                            unique: text(&t, row, "NON_UNIQUE") == "0",
                            name,
                            columns: vec![column],
                        }),
                    }
                }
                let t = self.query(&format!(
                    "SELECT k.CONSTRAINT_NAME, k.COLUMN_NAME, k.REFERENCED_TABLE_SCHEMA, k.REFERENCED_TABLE_NAME, k.REFERENCED_COLUMN_NAME, r.DELETE_RULE, r.UPDATE_RULE FROM information_schema.KEY_COLUMN_USAGE k JOIN information_schema.REFERENTIAL_CONSTRAINTS r ON r.CONSTRAINT_SCHEMA = k.CONSTRAINT_SCHEMA AND r.CONSTRAINT_NAME = k.CONSTRAINT_NAME AND r.TABLE_NAME = k.TABLE_NAME WHERE k.TABLE_SCHEMA = {schema} AND k.TABLE_NAME = {name} AND k.REFERENCED_TABLE_NAME IS NOT NULL ORDER BY k.CONSTRAINT_NAME, k.ORDINAL_POSITION"
                ))?;
                out.relations = group_relations(
                    t.rows
                        .iter()
                        .map(|row| {
                            let target_schema = text(&t, row, "REFERENCED_TABLE_SCHEMA");
                            let target_table = text(&t, row, "REFERENCED_TABLE_NAME");
                            (
                                text(&t, row, "CONSTRAINT_NAME"),
                                text(&t, row, "COLUMN_NAME"),
                                if target_schema == source.schema {
                                    target_table
                                } else {
                                    format!("{target_schema}.{target_table}")
                                },
                                text(&t, row, "REFERENCED_COLUMN_NAME"),
                                text(&t, row, "DELETE_RULE"),
                                text(&t, row, "UPDATE_RULE"),
                            )
                        })
                        .collect(),
                );
                out.create = self
                    .query(&format!("SHOW CREATE TABLE {full}"))?
                    .rows
                    .first()
                    .and_then(|row| row.get(1).cloned().flatten())
                    .unwrap_or_default();
                out.rows = self
                    .query(&format!(
                        "SELECT TABLE_ROWS FROM information_schema.TABLES WHERE {filter}"
                    ))
                    .ok()
                    .and_then(|t| t.rows.first()?.first()?.as_deref()?.parse().ok())
                    .map(|n| (n, true));
            }
            Engine::Pg(_) => {
                let id = format!("{}::regclass", kind.literal(Some(&full)));
                let t = self.query(&format!(
                    "SELECT a.attname, format_type(a.atttypid, a.atttypmod) AS type, NOT a.attnotnull AS nullable, pg_get_expr(d.adbin, d.adrelid) AS def, col_description(a.attrelid, a.attnum) AS comment, a.attidentity AS ident FROM pg_attribute a LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum WHERE a.attrelid = {id} AND a.attnum > 0 AND NOT a.attisdropped ORDER BY a.attnum"
                ))?;
                let keys = self.primary_key(source);
                for row in &t.rows {
                    let name = text(&t, row, "attname");
                    let default = at(&t, row, "def");
                    let ident = text(&t, row, "ident");
                    out.columns.push(Column {
                        key: keys.contains(&name),
                        auto: !ident.is_empty()
                            || default
                                .as_deref()
                                .is_some_and(|d| d.starts_with("nextval(")),
                        kind: text(&t, row, "type"),
                        null: truthy(&text(&t, row, "nullable")),
                        default,
                        comment: text(&t, row, "comment"),
                        extra: ident,
                        name,
                    });
                }
                let t = self.query(&format!(
                    "SELECT c.relname, ix.indisunique AS uniq, ix.indisprimary AS prim, (SELECT string_agg(a.attname, chr(31) ORDER BY k.n) FROM unnest(ix.indkey) WITH ORDINALITY k(attnum, n) JOIN pg_attribute a ON a.attrelid = ix.indrelid AND a.attnum = k.attnum) AS cols FROM pg_index ix JOIN pg_class c ON c.oid = ix.indexrelid WHERE ix.indrelid = {id} ORDER BY ix.indisprimary DESC, c.relname"
                ))?;
                for row in &t.rows {
                    out.indexes.push(Index {
                        name: text(&t, row, "relname"),
                        columns: text(&t, row, "cols")
                            .split('\u{1f}')
                            .filter(|c| !c.is_empty())
                            .map(str::to_owned)
                            .collect(),
                        unique: truthy(&text(&t, row, "uniq")),
                        primary: truthy(&text(&t, row, "prim")),
                    });
                }
                let names = |field: &str, rel: &str| {
                    format!(
                        "(SELECT string_agg(a.attname, chr(31) ORDER BY k.n) FROM unnest(con.{field}) WITH ORDINALITY k(attnum, n) JOIN pg_attribute a ON a.attrelid = con.{rel} AND a.attnum = k.attnum)"
                    )
                };
                let t = self.query(&format!(
                    "SELECT con.conname, {} AS cols, con.confrelid::regclass::text AS target, {} AS target_cols, con.confdeltype AS del, con.confupdtype AS upd FROM pg_constraint con WHERE con.contype = 'f' AND con.conrelid = {id} ORDER BY 1",
                    names("conkey", "conrelid"),
                    names("confkey", "confrelid")
                ))?;
                for row in &t.rows {
                    let split =
                        |v: String| v.split('\u{1f}').map(str::to_owned).collect::<Vec<_>>();
                    out.relations.push(Relation {
                        name: text(&t, row, "conname"),
                        columns: split(text(&t, row, "cols")),
                        target: text(&t, row, "target"),
                        target_columns: split(text(&t, row, "target_cols")),
                        on_delete: pg_rule(&text(&t, row, "del")),
                        on_update: pg_rule(&text(&t, row, "upd")),
                    });
                }
                let relkind = self.query(&format!(
                    "SELECT relkind, reltuples::bigint FROM pg_class WHERE oid = {id}"
                ))?;
                let first = relkind.rows.first().cloned().unwrap_or_default();
                out.rows = first
                    .get(1)
                    .cloned()
                    .flatten()
                    .and_then(|v| v.parse::<i64>().ok())
                    .filter(|n| *n >= 0)
                    .map(|n| (n as u64, true));
                if matches!(first.first().cloned().flatten().as_deref(), Some("v" | "m")) {
                    let body = self
                        .query(&format!("SELECT pg_get_viewdef({id}, true)"))?
                        .rows
                        .first()
                        .and_then(|r| r.first().cloned().flatten())
                        .unwrap_or_default();
                    out.create = format!("CREATE VIEW {full} AS\n{}", body.trim_end());
                } else {
                    let constraints = self.query(&format!(
                        "SELECT conname, pg_get_constraintdef(oid) FROM pg_constraint WHERE conrelid = {id} ORDER BY contype <> 'p', conname"
                    ))?;
                    let indexes = self.query(&format!(
                        "SELECT pg_get_indexdef(ix.indexrelid) FROM pg_index ix WHERE ix.indrelid = {id} AND NOT EXISTS (SELECT 1 FROM pg_constraint c WHERE c.conindid = ix.indexrelid) ORDER BY 1"
                    ))?;
                    out.create = pg_create(&full, &out.columns, &constraints, &indexes);
                }
            }
            Engine::Lite(_) => {
                let schema = kind.quote(&source.schema);
                let name = kind.quote(&source.table);
                let t = self.query(&format!("PRAGMA {schema}.table_info({name})"))?;
                let keys = t.rows.iter().filter(|r| text(&t, r, "pk") != "0").count();
                for row in &t.rows {
                    let kind_name = text(&t, row, "type");
                    let key = text(&t, row, "pk") != "0";
                    out.columns.push(Column {
                        name: text(&t, row, "name"),
                        auto: key && keys == 1 && kind_name.eq_ignore_ascii_case("INTEGER"),
                        kind: kind_name,
                        null: text(&t, row, "notnull") == "0" && !key,
                        default: at(&t, row, "dflt_value"),
                        key,
                        comment: String::new(),
                        extra: String::new(),
                    });
                }
                let list = self.query(&format!("PRAGMA {schema}.index_list({name})"))?;
                for row in &list.rows {
                    let index = text(&list, row, "name");
                    let info = self.query(&format!(
                        "PRAGMA {schema}.index_info({})",
                        DbKind::Sqlite.quote(&index)
                    ))?;
                    out.indexes.push(Index {
                        columns: info.rows.iter().map(|r| text(&info, r, "name")).collect(),
                        unique: text(&list, row, "unique") == "1",
                        primary: text(&list, row, "origin") == "pk",
                        name: index,
                    });
                }
                let pk: Vec<String> = out
                    .columns
                    .iter()
                    .filter(|c| c.key)
                    .map(|c| c.name.clone())
                    .collect();
                if !pk.is_empty() && !out.indexes.iter().any(|i| i.primary) {
                    out.indexes.insert(
                        0,
                        Index {
                            name: "PRIMARY".into(),
                            columns: pk,
                            unique: true,
                            primary: true,
                        },
                    );
                }
                let t = self.query(&format!("PRAGMA {schema}.foreign_key_list({name})"))?;
                out.relations = group_relations(
                    t.rows
                        .iter()
                        .map(|row| {
                            (
                                format!("#{}", text(&t, row, "id")),
                                text(&t, row, "from"),
                                text(&t, row, "table"),
                                text(&t, row, "to"),
                                text(&t, row, "on_delete"),
                                text(&t, row, "on_update"),
                            )
                        })
                        .collect(),
                );
                for relation in &mut out.relations {
                    relation.name = "—".into();
                }
                let t = self.query(&format!(
                    "SELECT sql FROM {schema}.sqlite_master WHERE tbl_name = {} AND sql IS NOT NULL ORDER BY type NOT IN ('table', 'view'), name",
                    kind.literal(Some(&source.table))
                ))?;
                out.create = t
                    .rows
                    .iter()
                    .filter_map(|r| r.first().cloned().flatten())
                    .map(|s| format!("{s};"))
                    .collect::<Vec<_>>()
                    .join("\n\n");
                out.rows = self
                    .query(&format!("SELECT COUNT(*) FROM {schema}.{name}"))
                    .ok()
                    .and_then(|t| t.rows.first()?.first()?.as_deref()?.parse().ok())
                    .map(|n| (n, false));
            }
        }
        Ok(out)
    }
}

fn pg_create(full: &str, columns: &[Column], constraints: &Table, indexes: &Table) -> String {
    let q = |n: &str| DbKind::Postgres.quote(n);
    let mut lines: Vec<String> = columns
        .iter()
        .map(|c| {
            let mut line = format!("    {} {}", q(&c.name), c.kind);
            match c.extra.as_str() {
                "a" => line.push_str(" GENERATED ALWAYS AS IDENTITY"),
                "d" => line.push_str(" GENERATED BY DEFAULT AS IDENTITY"),
                _ => {}
            }
            if !c.null {
                line.push_str(" NOT NULL");
            }
            if let Some(default) = &c.default {
                line.push_str(&format!(" DEFAULT {default}"));
            }
            line
        })
        .collect();
    for row in &constraints.rows {
        if let (Some(Some(name)), Some(Some(def))) = (row.first(), row.get(1)) {
            lines.push(format!("    CONSTRAINT {} {def}", q(name)));
        }
    }
    let mut out = format!("CREATE TABLE {full} (\n{}\n);", lines.join(",\n"));
    for row in &indexes.rows {
        if let Some(Some(def)) = row.first() {
            out.push_str(&format!("\n\n{def};"));
        }
    }
    for c in columns.iter().filter(|c| !c.comment.is_empty()) {
        out.push_str(&format!(
            "\n\nCOMMENT ON COLUMN {full}.{} IS {};",
            q(&c.name),
            DbKind::Postgres.literal(Some(&c.comment))
        ));
    }
    out
}

fn full_name(kind: DbKind, source: &Source) -> String {
    format!(
        "{}.{}",
        kind.quote(&source.schema),
        kind.quote(&source.table)
    )
}

fn mysql_definition(draft: &Draft, extra: &str) -> String {
    let mut sql = format!(
        "{} {}",
        DbKind::MySql.quote(draft.name.trim()),
        draft.kind.trim()
    );
    sql.push_str(if draft.null { " NULL" } else { " NOT NULL" });
    if !draft.default.trim().is_empty() {
        sql.push_str(&format!(" DEFAULT {}", draft.default.trim()));
    }
    let lower = extra.to_ascii_lowercase();
    if lower.contains("auto_increment") {
        sql.push_str(" AUTO_INCREMENT");
    }
    if let Some(at) = lower.find("on update ") {
        sql.push_str(&format!(" ON UPDATE {}", extra[at + 10..].trim()));
    }
    if !draft.comment.is_empty() {
        sql.push_str(&format!(
            " COMMENT {}",
            DbKind::MySql.literal(Some(&draft.comment))
        ));
    }
    sql
}

fn check(draft: &Draft) -> Result<(), String> {
    if draft.name.trim().is_empty() {
        return Err(tr!("Укажите имя столбца").into());
    }
    if draft.kind.trim().is_empty() {
        return Err(tr!("Укажите тип столбца").into());
    }
    Ok(())
}

pub fn add_column(kind: DbKind, source: &Source, draft: &Draft) -> Result<Vec<String>, String> {
    check(draft)?;
    let table = full_name(kind, source);
    Ok(match kind {
        DbKind::MySql => vec![format!(
            "ALTER TABLE {table} ADD COLUMN {}",
            mysql_definition(draft, "")
        )],
        DbKind::Postgres | DbKind::Sqlite => {
            let mut sql = format!(
                "ALTER TABLE {table} ADD COLUMN {} {}",
                kind.quote(draft.name.trim()),
                draft.kind.trim()
            );
            if !draft.null {
                sql.push_str(" NOT NULL");
            }
            if !draft.default.trim().is_empty() {
                sql.push_str(&format!(" DEFAULT {}", draft.default.trim()));
            }
            let mut out = vec![sql];
            if kind == DbKind::Postgres && !draft.comment.is_empty() {
                out.push(format!(
                    "COMMENT ON COLUMN {table}.{} IS {}",
                    kind.quote(draft.name.trim()),
                    kind.literal(Some(&draft.comment))
                ));
            }
            out
        }
    })
}

pub fn alter_column(
    kind: DbKind,
    source: &Source,
    old: &Column,
    draft: &Draft,
) -> Result<Vec<String>, String> {
    check(draft)?;
    if *draft == Draft::of(old) {
        return Err(tr!("Нет изменений").into());
    }
    let table = full_name(kind, source);
    let name = draft.name.trim();
    let renamed = name != old.name;
    let default = draft.default.trim();
    let old_default = old.default.as_deref().unwrap_or("");
    match kind {
        DbKind::MySql => Ok(vec![format!(
            "ALTER TABLE {table} CHANGE COLUMN {} {}",
            kind.quote(&old.name),
            mysql_definition(draft, &old.extra)
        )]),
        DbKind::Postgres => {
            let mut out = Vec::new();
            let column = kind.quote(name);
            if renamed {
                out.push(format!(
                    "ALTER TABLE {table} RENAME COLUMN {} TO {column}",
                    kind.quote(&old.name)
                ));
            }
            if !draft.kind.trim().eq_ignore_ascii_case(&old.kind) {
                out.push(format!(
                    "ALTER TABLE {table} ALTER COLUMN {column} TYPE {ty} USING {column}::{ty}",
                    ty = draft.kind.trim()
                ));
            }
            if draft.null != old.null {
                out.push(format!(
                    "ALTER TABLE {table} ALTER COLUMN {column} {} NOT NULL",
                    if draft.null { "DROP" } else { "SET" }
                ));
            }
            if default != old_default {
                out.push(if default.is_empty() {
                    format!("ALTER TABLE {table} ALTER COLUMN {column} DROP DEFAULT")
                } else {
                    format!("ALTER TABLE {table} ALTER COLUMN {column} SET DEFAULT {default}")
                });
            }
            if draft.comment != old.comment {
                out.push(format!(
                    "COMMENT ON COLUMN {table}.{column} IS {}",
                    if draft.comment.is_empty() {
                        "NULL".to_owned()
                    } else {
                        kind.literal(Some(&draft.comment))
                    }
                ));
            }
            Ok(out)
        }
        DbKind::Sqlite => {
            if !draft.kind.trim().eq_ignore_ascii_case(&old.kind)
                || draft.null != old.null
                || default != old_default
            {
                return Err(tr!("SQLite меняет у существующего столбца только имя").into());
            }
            Ok(vec![format!(
                "ALTER TABLE {table} RENAME COLUMN {} TO {}",
                kind.quote(&old.name),
                kind.quote(name)
            )])
        }
    }
}

pub fn drop_column(kind: DbKind, source: &Source, name: &str) -> Vec<String> {
    vec![format!(
        "ALTER TABLE {} DROP COLUMN {}",
        full_name(kind, source),
        kind.quote(name)
    )]
}

fn types(kind: DbKind) -> &'static [&'static str] {
    match kind {
        DbKind::MySql => &[
            "int",
            "bigint",
            "tinyint(1)",
            "decimal(10,2)",
            "varchar(255)",
            "text",
            "mediumtext",
            "datetime",
            "date",
            "timestamp",
            "json",
        ],
        DbKind::Postgres => &[
            "integer",
            "bigint",
            "boolean",
            "numeric(10,2)",
            "character varying(255)",
            "text",
            "timestamp without time zone",
            "date",
            "jsonb",
            "uuid",
        ],
        DbKind::Sqlite => &["INTEGER", "REAL", "TEXT", "BLOB", "NUMERIC"],
    }
}

fn plural_rows(n: u64) -> String {
    if crate::i18n::english() {
        return if n == 1 {
            "1 row".into()
        } else {
            format!("{n} rows")
        };
    }
    let word = match (n % 10, n % 100) {
        (1, r) if r != 11 => "строка",
        (2..=4, r) if !(12..=14).contains(&r) => "строки",
        _ => "строк",
    };
    format!("{n} {word}")
}

fn sub_tab(ui: &mut Ui, pal: &Palette, active: bool, label: &str) -> bool {
    let response = ui.add(
        Button::new(RichText::new(label).color(if active { pal.text } else { pal.subtext }))
            .frame(false)
            .min_size(egui::vec2(0.0, 24.0)),
    );
    if active {
        let r = response.rect;
        ui.painter().line_segment(
            [
                egui::pos2(r.left() - 6.0, r.bottom() + 1.0),
                egui::pos2(r.right() + 6.0, r.bottom() + 1.0),
            ],
            egui::Stroke::new(2.0, pal.accent),
        );
    }
    response.clicked()
}

fn key_icon(painter: &egui::Painter, pal: &Palette, x: f32, cy: f32) {
    let stroke = egui::Stroke::new(1.4, pal.accent);
    painter.circle_stroke(egui::pos2(x + 3.5, cy), 3.0, stroke);
    painter.line_segment([egui::pos2(x + 6.5, cy), egui::pos2(x + 12.0, cy)], stroke);
    painter.line_segment(
        [egui::pos2(x + 10.5, cy), egui::pos2(x + 10.5, cy + 3.0)],
        stroke,
    );
}

struct Cell {
    text: String,
    faint: bool,
    accent: bool,
}

fn cell(text: impl Into<String>) -> Cell {
    Cell {
        text: text.into(),
        faint: false,
        accent: false,
    }
}

fn faint(text: impl Into<String>) -> Cell {
    Cell {
        text: text.into(),
        faint: true,
        accent: false,
    }
}

fn table_view(
    ui: &mut Ui,
    pal: &Palette,
    id: &str,
    font: &FontId,
    headers: &[&str],
    rows: &[(bool, Vec<Cell>)],
    picked: Option<usize>,
) -> Option<(usize, bool)> {
    let char_w = ui.fonts_mut(|f| f.glyph_width(font, '0'));
    let pad = 10.0;
    let icon = 22.0;
    let widths: Vec<f32> = (0..headers.len())
        .map(|i| {
            let widest = rows
                .iter()
                .map(|(_, cells)| cells[i].text.chars().count())
                .max()
                .unwrap_or(0)
                .max(headers[i].chars().count())
                .clamp(4, 40);
            widest as f32 * char_w + pad * 2.0
        })
        .collect();
    let total = icon + widths.iter().sum::<f32>();
    let row_h = crate::widgets::RESULT_ROW + 2.0;
    let mut out = None;
    ScrollArea::both()
        .id_salt(id)
        .auto_shrink([false, true])
        .max_height(ui.available_height() - 70.0)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let width = total.max(ui.available_width());
            let (header, _) = ui.allocate_exact_size(egui::vec2(width, row_h), Sense::hover());
            ui.painter().rect_filled(header, 0.0, pal.crust);
            let mut x = header.left() + icon;
            for (name, w) in headers.iter().zip(&widths) {
                ui.painter().text(
                    egui::pos2(x + pad, header.center().y),
                    Align2::LEFT_CENTER,
                    *name,
                    FontId::proportional(12.0),
                    pal.subtext,
                );
                x += w;
            }
            for (index, (key, cells)) in rows.iter().enumerate() {
                let (rect, response) =
                    ui.allocate_exact_size(egui::vec2(width, row_h), Sense::click());
                let label = cells.first().map(|c| c.text.clone()).unwrap_or_default();
                response
                    .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &label));
                let painter = ui.painter();
                if picked == Some(index) {
                    painter.rect_filled(rect, 0.0, pal.selection);
                } else if response.hovered() {
                    painter.rect_filled(rect, 0.0, pal.current_line);
                } else if index % 2 == 1 {
                    painter.rect_filled(rect, 0.0, pal.base);
                }
                if *key {
                    key_icon(painter, pal, rect.left() + 5.0, rect.center().y);
                }
                let mut x = rect.left() + icon;
                for (c, w) in cells.iter().zip(&widths) {
                    let clip = painter.with_clip_rect(egui::Rect::from_min_size(
                        egui::pos2(x, rect.top()),
                        egui::vec2(*w - 4.0, row_h),
                    ));
                    clip.text(
                        egui::pos2(x + pad, rect.center().y),
                        Align2::LEFT_CENTER,
                        &c.text,
                        font.clone(),
                        if c.accent || (*key && x == rect.left() + icon) {
                            pal.accent
                        } else if c.faint {
                            pal.faint
                        } else {
                            pal.text
                        },
                    );
                    x += w;
                }
                if response.double_clicked() {
                    out = Some((index, true));
                } else if response.clicked() {
                    out = Some((index, false));
                }
            }
        });
    out
}

impl DbPanel {
    pub(super) fn open_structure(&mut self, source: Source) {
        if self.structure.loading.as_ref() == Some(&source) {
            return;
        }
        self.structure.loading = Some(source.clone());
        self.structure.picked = None;
        self.send(Cmd::Structure(source));
    }

    pub(super) fn structure_view(&mut self, ui: &mut Ui, pal: &Palette, font_size: f32) {
        if let Some(source) = self.source.clone()
            && self.structure.asked.as_ref() != Some(&source)
        {
            self.structure.asked = Some(source.clone());
            if self.structure.data.as_ref().map(|d| &d.source) != Some(&source) {
                self.open_structure(source);
            }
        }
        let Some(data) = self.structure.data.clone() else {
            let text = if self.structure.loading.is_some() {
                tr!("Загрузка структуры…")
            } else {
                tr!(
                    "Выберите таблицу слева — здесь появится её структура: столбцы, индексы, связи и SQL создания."
                )
            };
            ui.add_space(8.0);
            ui.add(egui::Label::new(RichText::new(text).color(pal.subtext)).wrap());
            return;
        };
        let kind = self.kind;
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(&data.source.table)
                    .size(crate::widgets::TITLE)
                    .color(pal.text),
            );
            let mut meta = Vec::new();
            if let Some((n, approx)) = data.rows {
                meta.push(format!(
                    "{}{}",
                    if approx { "≈ " } else { "" },
                    plural_rows(n)
                ));
            }
            meta.push(kind.label().to_owned());
            ui.label(RichText::new(format!("·  {}", meta.join("  ·  "))).color(pal.subtext));
            if crate::widgets::refresh_button(ui, pal)
                .on_hover_text(tr!("Перечитать структуру"))
                .clicked()
            {
                self.structure.loading = None;
                self.open_structure(data.source.clone());
            }
        });
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 18.0;
            let tabs = [
                (
                    Tab::Columns,
                    format!("{}  {}", tr!("Столбцы"), data.columns.len()),
                ),
                (
                    Tab::Indexes,
                    format!("{}  {}", tr!("Индексы"), data.indexes.len()),
                ),
                (
                    Tab::Relations,
                    format!("{}  {}", tr!("Связи"), data.relations.len()),
                ),
                (Tab::Create, tr!("SQL создания").to_owned()),
            ];
            ui.add_space(6.0);
            for (tab, label) in tabs {
                if sub_tab(ui, pal, self.structure.tab == tab, &label) {
                    self.structure.tab = tab;
                }
            }
        });
        ui.add_space(6.0);
        let font = FontId::monospace(font_size - 1.0);
        match self.structure.tab {
            Tab::Columns => self.columns_tab(ui, pal, &font, &data),
            Tab::Indexes => {
                let rows: Vec<(bool, Vec<Cell>)> = data
                    .indexes
                    .iter()
                    .map(|i| {
                        (
                            i.primary,
                            vec![
                                cell(i.name.clone()),
                                cell(i.columns.join(", ")),
                                faint(if i.primary {
                                    "PRIMARY"
                                } else if i.unique {
                                    "UNIQUE"
                                } else {
                                    "INDEX"
                                }),
                            ],
                        )
                    })
                    .collect();
                if rows.is_empty() {
                    ui.label(RichText::new(tr!("Индексов нет.")).color(pal.subtext));
                } else {
                    table_view(
                        ui,
                        pal,
                        "tror_db_indexes",
                        &font,
                        &[tr!("Имя"), tr!("Столбцы"), tr!("Тип")],
                        &rows,
                        None,
                    );
                }
            }
            Tab::Relations => {
                let rows: Vec<(bool, Vec<Cell>)> = data
                    .relations
                    .iter()
                    .map(|r| {
                        (
                            false,
                            vec![
                                cell(r.name.clone()),
                                cell(r.columns.join(", ")),
                                cell(format!("{} ({})", r.target, r.target_columns.join(", "))),
                                faint(r.on_delete.clone()),
                                faint(r.on_update.clone()),
                            ],
                        )
                    })
                    .collect();
                if rows.is_empty() {
                    ui.label(
                        RichText::new(tr!("Внешних ключей нет: таблица не ссылается на другие."))
                            .color(pal.subtext),
                    );
                } else {
                    table_view(
                        ui,
                        pal,
                        "tror_db_relations",
                        &font,
                        &[
                            tr!("Имя"),
                            tr!("Столбцы"),
                            tr!("Ссылается на"),
                            tr!("При удалении"),
                            tr!("При изменении"),
                        ],
                        &rows,
                        None,
                    );
                }
            }
            Tab::Create => {
                let mut code = data.create.clone();
                ScrollArea::both()
                    .id_salt("tror_db_create")
                    .max_height(ui.available_height() - 44.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.add(
                            TextEdit::multiline(&mut code)
                                .font(font.clone())
                                .code_editor()
                                .desired_width(f32::INFINITY)
                                .interactive(true),
                        );
                    });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui
                        .add(Button::new(tr!("Копировать")).min_size(crate::widgets::ACTION))
                        .clicked()
                    {
                        ui.ctx().copy_text(data.create.clone());
                    }
                    if ui
                        .add(Button::new(tr!("Открыть в запросе")).min_size(crate::widgets::ACTION))
                        .clicked()
                    {
                        self.sql = data.create.clone();
                        self.mode = super::Mode::Query;
                    }
                });
            }
        }
    }

    fn columns_tab(&mut self, ui: &mut Ui, pal: &Palette, font: &FontId, data: &Structure) {
        let comments = kind_has_comments(self.kind);
        let rows: Vec<(bool, Vec<Cell>)> = data
            .columns
            .iter()
            .map(|c| {
                let mut cells = vec![
                    cell(c.name.clone()),
                    cell(c.kind.clone()),
                    faint(if c.null { tr!("да") } else { tr!("нет") }),
                    match &c.default {
                        Some(d) => cell(d.clone()),
                        None if c.null => faint("NULL"),
                        None => faint("—"),
                    },
                ];
                let mut note = c.comment.clone();
                if c.auto {
                    if !note.is_empty() {
                        note.push_str("  ·  ");
                    }
                    note.push_str(tr!("автоинкремент"));
                }
                cells.push(if c.comment.is_empty() {
                    faint(note)
                } else {
                    cell(note)
                });
                (c.key, cells)
            })
            .collect();
        let headers = [
            tr!("Имя"),
            tr!("Тип"),
            "NULL",
            tr!("По умолчанию"),
            if comments {
                tr!("Комментарий")
            } else {
                tr!("Примечание")
            },
        ];
        if let Some((index, open)) = table_view(
            ui,
            pal,
            "tror_db_columns",
            font,
            &headers,
            &rows,
            self.structure.picked,
        ) {
            self.structure.picked = Some(index);
            if open {
                self.structure.edit = Some(ColumnEdit {
                    original: Some(data.columns[index].clone()),
                    draft: Draft::of(&data.columns[index]),
                });
            }
        }
        ui.add_space(8.0);
        let picked = self.structure.picked.and_then(|i| data.columns.get(i));
        ui.horizontal(|ui| {
            let enabled = !self.busy;
            if ui
                .add_enabled(
                    enabled,
                    Button::new(tr!("+ Столбец")).min_size(crate::widgets::ACTION),
                )
                .clicked()
            {
                self.structure.edit = Some(ColumnEdit {
                    original: None,
                    draft: Draft {
                        null: true,
                        kind: types(self.kind)[if self.kind == DbKind::Sqlite { 2 } else { 4 }]
                            .to_owned(),
                        ..Default::default()
                    },
                });
            }
            if ui
                .add_enabled(
                    enabled && picked.is_some(),
                    Button::new(tr!("Изменить…")).min_size(crate::widgets::ACTION),
                )
                .clicked()
                && let Some(column) = picked
            {
                self.structure.edit = Some(ColumnEdit {
                    original: Some(column.clone()),
                    draft: Draft::of(column),
                });
            }
            if ui
                .add_enabled(
                    enabled && picked.is_some() && data.columns.len() > 1,
                    Button::new(tr!("Удалить столбец")).min_size(crate::widgets::ACTION),
                )
                .clicked()
                && let Some(column) = picked
            {
                self.structure.drop = Some((
                    column.name.clone(),
                    drop_column(self.kind, &data.source, &column.name),
                ));
            }
        });
        ui.add_space(4.0);
        ui.add(
            egui::Label::new(
                RichText::new(tr!(
                    "Изменения не применяются сразу: TrorPad покажет SQL (ALTER TABLE) и спросит подтверждение."
                ))
                .small()
                .color(pal.faint),
            )
            .wrap(),
        );
    }

    pub(super) fn structure_dialogs(&mut self, ctx: &egui::Context, pal: &Palette) {
        let Some(data) = self.structure.data.clone() else {
            self.structure.edit = None;
            self.structure.drop = None;
            return;
        };
        let kind = self.kind;
        if let Some((name, statements)) = self.structure.drop.clone() {
            let mut run = false;
            let mut cancel = false;
            let modal = dialog("tror_db_drop_column", pal).show(ctx, |ui| {
                ui.set_width(520.0);
                crate::widgets::title(
                    ui,
                    pal,
                    tf!("Удалить столбец «{name}»?", "Delete column \"{name}\"?"),
                );
                ui.label(
                    RichText::new(tr!("Данные столбца будут потеряны. Будет выполнено:"))
                        .color(pal.subtext)
                        .small(),
                );
                ui.add_space(6.0);
                for statement in &statements {
                    ui.add(
                        egui::Label::new(RichText::new(statement).monospace().size(12.0)).wrap(),
                    );
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui
                        .add(
                            crate::widgets::danger(tr!("Удалить"), pal)
                                .min_size(crate::widgets::ACTION),
                        )
                        .clicked()
                    {
                        run = true;
                    }
                    if dialog_button(ui, pal, tr!("Отмена"), false) {
                        cancel = true;
                    }
                });
            });
            if run {
                self.structure.drop = None;
                self.structure.picked = None;
                self.alter(statements, data.source.clone());
            } else if cancel || modal.should_close() {
                self.structure.drop = None;
            }
        }
        let Some(edit) = &mut self.structure.edit else {
            return;
        };
        let built = match &edit.original {
            Some(old) => alter_column(kind, &data.source, old, &edit.draft),
            None => add_column(kind, &data.source, &edit.draft),
        };
        let mut run = false;
        let mut cancel = false;
        let rename_only = kind == DbKind::Sqlite && edit.original.is_some();
        let modal = dialog("tror_db_column", pal).show(ctx, |ui| {
            use crate::widgets::{form_field, form_row};
            ui.set_width(560.0);
            crate::widgets::title(
                ui,
                pal,
                match &edit.original {
                    Some(old) => tf!("Столбец «{}»", "Column \"{}\"", old.name),
                    None => tf!("Новый столбец в «{}»", "New column in \"{}\"", data.source.table),
                },
            );
            ui.add_space(8.0);
            ui.spacing_mut().item_spacing.y = 8.0;
            let field_w = 360.0;
            form_row(ui, pal, tr!("Имя"), |ui| {
                ui.add(form_field(&mut edit.draft.name, "").desired_width(field_w));
            });
            ui.add_enabled_ui(!rename_only, |ui| {
                form_row(ui, pal, tr!("Тип"), |ui| {
                    ui.add(form_field(&mut edit.draft.kind, "").desired_width(field_w - 34.0));
                    ComboBox::from_id_salt("tror_db_column_type")
                        .width(24.0)
                        .selected_text("")
                        .show_ui(ui, |ui| {
                            for ty in types(kind) {
                                if ui.selectable_label(edit.draft.kind == *ty, *ty).clicked() {
                                    edit.draft.kind = (*ty).to_owned();
                                }
                            }
                        });
                });
                form_row(ui, pal, "NULL", |ui| {
                    ui.checkbox(&mut edit.draft.null, tr!("Может быть пустым (NULL)"));
                });
                form_row(ui, pal, tr!("По умолчанию"), |ui| {
                    ui.add(
                        form_field(&mut edit.draft.default, tr!("например 0, 'текст' или CURRENT_TIMESTAMP"))
                            .desired_width(field_w),
                    );
                });
            });
            if kind_has_comments(kind) {
                form_row(ui, pal, tr!("Комментарий"), |ui| {
                    ui.add(form_field(&mut edit.draft.comment, "").desired_width(field_w));
                });
            }
            if rename_only {
                ui.label(
                    RichText::new(tr!(
                        "SQLite умеет у существующего столбца только менять имя. Для другого типа создайте новый столбец и перенесите данные."
                    ))
                    .small()
                    .color(pal.faint),
                );
            }
            ui.add_space(4.0);
            match &built {
                Ok(statements) => {
                    ui.label(RichText::new(tr!("Будет выполнено:")).small().color(pal.subtext));
                    egui::Frame::new()
                        .fill(pal.crust)
                        .corner_radius(6)
                        .inner_margin(egui::Margin::same(8))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            for statement in statements {
                                ui.add(
                                    egui::Label::new(RichText::new(format!("{statement};")).monospace().size(12.0))
                                        .wrap(),
                                );
                            }
                        });
                }
                Err(e) => {
                    ui.label(RichText::new(e).small().color(pal.faint));
                }
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let ok = built.is_ok();
                if ui
                    .add_enabled(ok, crate::widgets::primary(tr!("Выполнить"), pal).min_size(crate::widgets::ACTION))
                    .lit(ui)
                    .clicked()
                {
                    run = true;
                }
                if dialog_button(ui, pal, tr!("Отмена"), false) {
                    cancel = true;
                }
            });
        });
        if run && let Ok(statements) = built {
            self.structure.edit = None;
            self.alter(statements, data.source.clone());
        } else if cancel || modal.should_close() {
            self.structure.edit = None;
        }
    }

    fn alter(&mut self, statements: Vec<String>, source: Source) {
        self.status = tr!("Изменение структуры…").into();
        self.error = false;
        self.structure.loading = Some(source.clone());
        self.send(Cmd::Alter(statements, source));
    }

    pub(super) fn structure_modal(&self) -> bool {
        self.structure.edit.is_some() || self.structure.drop.is_some()
    }
}

fn kind_has_comments(kind: DbKind) -> bool {
    kind != DbKind::Sqlite
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lite(sql: &str) -> Engine {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(sql).unwrap();
        Engine::Lite(Box::new(conn))
    }

    fn source(table: &str) -> Source {
        Source {
            schema: "main".into(),
            table: table.into(),
        }
    }

    #[test]
    fn reads_sqlite_structure_and_alters_it() {
        let mut engine = lite(
            "CREATE TABLE users (id INTEGER PRIMARY KEY, email TEXT NOT NULL);
             CREATE TABLE goods (id INTEGER PRIMARY KEY, title TEXT, price INTEGER NOT NULL DEFAULT 0, owner INTEGER REFERENCES users(id) ON DELETE CASCADE);
             CREATE INDEX goods_title ON goods(title);
             INSERT INTO goods (title, price) VALUES ('a', 1), ('b', 2), ('c', 3);",
        );
        let s = engine.structure(&source("goods")).unwrap();
        assert_eq!(s.rows, Some((3, false)));
        let names: Vec<_> = s.columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["id", "title", "price", "owner"]);
        assert!(s.columns[0].key && s.columns[0].auto && !s.columns[0].null);
        assert!(s.columns[1].null);
        assert_eq!(s.columns[2].default.as_deref(), Some("0"));
        assert_eq!(s.indexes.len(), 2);
        assert!(s.indexes[0].primary);
        assert_eq!(s.indexes[1].columns, ["title"]);
        assert_eq!(s.relations.len(), 1);
        assert_eq!(s.relations[0].target, "users");
        assert_eq!(s.relations[0].on_delete, "CASCADE");
        assert!(s.create.starts_with("CREATE TABLE goods"));
        assert!(s.create.contains("CREATE INDEX goods_title"));

        let add = add_column(
            DbKind::Sqlite,
            &s.source,
            &Draft {
                name: "note".into(),
                kind: "TEXT".into(),
                null: true,
                default: "'x'".into(),
                comment: String::new(),
            },
        )
        .unwrap();
        assert_eq!(
            add,
            [r#"ALTER TABLE "main"."goods" ADD COLUMN "note" TEXT DEFAULT 'x'"#]
        );
        engine.batch(&add).unwrap();
        let mut draft = Draft::of(&s.columns[1]);
        draft.name = "name".into();
        let rename = alter_column(DbKind::Sqlite, &s.source, &s.columns[1], &draft).unwrap();
        engine.batch(&rename).unwrap();
        draft.kind = "INTEGER".into();
        assert!(alter_column(DbKind::Sqlite, &s.source, &s.columns[1], &draft).is_err());
        engine
            .batch(&drop_column(DbKind::Sqlite, &s.source, "price"))
            .unwrap();
        let after = engine.structure(&source("goods")).unwrap();
        let names: Vec<_> = after.columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["id", "name", "owner", "note"]);
        assert_eq!(after.columns[3].default.as_deref(), Some("'x'"));
    }

    #[test]
    fn builds_mysql_and_postgres_statements() {
        let src = Source {
            schema: "shop".into(),
            table: "goods".into(),
        };
        let old = Column {
            name: "id".into(),
            kind: "int(11)".into(),
            null: false,
            default: None,
            key: true,
            auto: true,
            comment: String::new(),
            extra: "auto_increment".into(),
        };
        let mut draft = Draft::of(&old);
        assert!(alter_column(DbKind::MySql, &src, &old, &draft).is_err());
        draft.comment = "it's key".into();
        assert_eq!(
            alter_column(DbKind::MySql, &src, &old, &draft).unwrap(),
            [
                "ALTER TABLE `shop`.`goods` CHANGE COLUMN `id` `id` int(11) NOT NULL AUTO_INCREMENT COMMENT 'it''s key'"
            ]
        );
        let stamp = Column {
            name: "changed".into(),
            kind: "timestamp".into(),
            null: true,
            default: mysql_default(
                Some("CURRENT_TIMESTAMP".into()),
                "DEFAULT_GENERATED on update CURRENT_TIMESTAMP",
            ),
            extra: "DEFAULT_GENERATED on update CURRENT_TIMESTAMP".into(),
            ..Default::default()
        };
        let mut draft = Draft::of(&stamp);
        draft.name = "updated".into();
        assert_eq!(
            alter_column(DbKind::MySql, &src, &stamp, &draft).unwrap(),
            [
                "ALTER TABLE `shop`.`goods` CHANGE COLUMN `changed` `updated` timestamp NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP"
            ]
        );
        assert_eq!(mysql_default(Some("abc".into()), ""), Some("'abc'".into()));
        assert_eq!(
            mysql_default(Some("'abc'".into()), ""),
            Some("'abc'".into())
        );
        assert_eq!(mysql_default(Some("NULL".into()), ""), None);
        assert_eq!(mysql_default(Some("1.5".into()), ""), Some("1.5".into()));

        let pg = Source {
            schema: "public".into(),
            table: "goods".into(),
        };
        let price = Column {
            name: "price".into(),
            kind: "integer".into(),
            null: true,
            default: Some("0".into()),
            ..Default::default()
        };
        let draft = Draft {
            name: "cost".into(),
            kind: "numeric(10,2)".into(),
            null: false,
            default: String::new(),
            comment: "в рублях".into(),
        };
        assert_eq!(
            alter_column(DbKind::Postgres, &pg, &price, &draft).unwrap(),
            [
                r#"ALTER TABLE "public"."goods" RENAME COLUMN "price" TO "cost""#,
                r#"ALTER TABLE "public"."goods" ALTER COLUMN "cost" TYPE numeric(10,2) USING "cost"::numeric(10,2)"#,
                r#"ALTER TABLE "public"."goods" ALTER COLUMN "cost" SET NOT NULL"#,
                r#"ALTER TABLE "public"."goods" ALTER COLUMN "cost" DROP DEFAULT"#,
                r#"COMMENT ON COLUMN "public"."goods"."cost" IS 'в рублях'"#,
            ]
        );
        assert_eq!(
            add_column(DbKind::Postgres, &pg, &draft).unwrap(),
            [
                r#"ALTER TABLE "public"."goods" ADD COLUMN "cost" numeric(10,2) NOT NULL"#,
                r#"COMMENT ON COLUMN "public"."goods"."cost" IS 'в рублях'"#,
            ]
        );
        assert!(add_column(DbKind::MySql, &src, &Draft::default()).is_err());
        assert_eq!(plural_rows(3), "3 строки");
        assert_eq!(plural_rows(11), "11 строк");
        assert_eq!(plural_rows(21), "21 строка");
    }

    fn live(profile: super::super::DbProfile, port: u16, setup: &[&str], src: &Source) -> Engine {
        let (mut engine, _) =
            super::super::open_engine(&profile, "127.0.0.1".into(), port).unwrap();
        for sql in setup {
            engine.query(sql).unwrap();
        }
        let s = engine.structure(src).unwrap();
        assert_eq!(s.columns.len(), 4, "{s:?}");
        engine
    }

    fn exercise(engine: &mut Engine, kind: DbKind, src: &Source) {
        let s = engine.structure(src).unwrap();
        let names: Vec<_> = s.columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["id", "title", "price", "owner"]);
        assert!(s.columns[0].key && s.columns[0].auto, "{:?}", s.columns[0]);
        assert!(s.columns[1].null && !s.columns[2].null);
        assert_eq!(s.columns[2].default.as_deref(), Some("0"));
        assert_eq!(s.columns[1].comment, "Название");
        assert!(
            s.indexes.iter().any(|i| i.primary && i.columns == ["id"]),
            "{:?}",
            s.indexes
        );
        assert!(
            s.indexes
                .iter()
                .any(|i| !i.primary && i.columns == ["title"]),
            "{:?}",
            s.indexes
        );
        assert_eq!(s.relations.len(), 1, "{:?}", s.relations);
        assert_eq!(s.relations[0].target, "users");
        assert_eq!(s.relations[0].on_delete, "CASCADE");
        assert!(s.create.contains("CREATE TABLE"), "{}", s.create);
        assert!(s.create.contains("goods_title"), "{}", s.create);
        let add = add_column(
            kind,
            src,
            &Draft {
                name: "note".into(),
                kind: "varchar(20)".into(),
                null: false,
                default: "'x'".into(),
                comment: "заметка".into(),
            },
        )
        .unwrap();
        engine.batch(&add).unwrap();
        let mut draft = Draft::of(&s.columns[1]);
        draft.name = "name".into();
        draft.null = false;
        draft.default = "'—'".into();
        draft.comment = "Имя".into();
        engine
            .batch(&alter_column(kind, src, &s.columns[1], &draft).unwrap())
            .unwrap();
        let id = Draft::of(&s.columns[0]);
        let mut renamed = id.clone();
        renamed.comment = "ключ".into();
        engine
            .batch(&alter_column(kind, src, &s.columns[0], &renamed).unwrap())
            .unwrap();
        engine.batch(&drop_column(kind, src, "price")).unwrap();
        let after = engine.structure(src).unwrap();
        let names: Vec<_> = after.columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["id", "name", "owner", "note"]);
        assert!(
            after.columns[0].auto && after.columns[0].comment == "ключ",
            "{:?}",
            after.columns[0]
        );
        assert!(!after.columns[1].null);
        assert_eq!(
            after.columns[1]
                .default
                .as_deref()
                .map(|d| d.split("::").next().unwrap()),
            Some("'—'")
        );
        assert_eq!(after.columns[1].comment, "Имя");
        assert_eq!(after.columns[3].comment, "заметка");
        let fresh = Draft::of(&after.columns[1]);
        assert!(alter_column(kind, src, &after.columns[1], &fresh).is_err());
    }

    #[test]
    #[ignore]
    fn live_mysql_structure() {
        let src = Source {
            schema: "shop".into(),
            table: "goods".into(),
        };
        let mut engine = live(
            super::super::DbProfile {
                user: "jozef".into(),
                password: "secret".into(),
                ..Default::default()
            },
            3307,
            &[
                "DROP TABLE IF EXISTS shop.goods",
                "DROP TABLE IF EXISTS shop.users",
                "CREATE TABLE shop.users (id INT PRIMARY KEY AUTO_INCREMENT)",
                "CREATE TABLE shop.goods (id INT PRIMARY KEY AUTO_INCREMENT, title VARCHAR(100) NULL COMMENT 'Название', price INT NOT NULL DEFAULT 0, owner INT, KEY goods_title (title), CONSTRAINT goods_owner FOREIGN KEY (owner) REFERENCES users(id) ON DELETE CASCADE)",
            ],
            &src,
        );
        exercise(&mut engine, DbKind::MySql, &src);
    }

    #[test]
    #[ignore]
    fn live_postgres_structure() {
        let src = Source {
            schema: "public".into(),
            table: "goods".into(),
        };
        let mut engine = live(
            super::super::DbProfile {
                kind: DbKind::Postgres,
                user: "jozef".into(),
                password: "secret".into(),
                database: "shop".into(),
                ..Default::default()
            },
            5432,
            &[
                "DROP TABLE IF EXISTS goods; DROP TABLE IF EXISTS users",
                "CREATE TABLE users (id serial PRIMARY KEY)",
                "CREATE TABLE goods (id serial PRIMARY KEY, title varchar(100), price integer NOT NULL DEFAULT 0, owner integer REFERENCES users(id) ON DELETE CASCADE)",
                "CREATE INDEX goods_title ON goods(title); COMMENT ON COLUMN goods.title IS 'Название'",
            ],
            &src,
        );
        exercise(&mut engine, DbKind::Postgres, &src);
    }
}
