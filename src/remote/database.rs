use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Debug, Default)]
pub enum DbKind {
    #[default]
    MySql,
    Postgres,
    Sqlite,
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
