//! HyperDrive - persistent file tags (SQLite, portable-first location).
//! Copyright (C) 2026 dragon. SPDX-License-Identifier: GPL-3.0-or-later

use rusqlite::Connection;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

pub const TAG_COLORS: [(&str, &str); 7] = [
    ("Red", "#e85d75"),
    ("Orange", "#f2a35c"),
    ("Yellow", "#e8d44d"),
    ("Green", "#77d17a"),
    ("Blue", "#5a96dc"),
    ("Purple", "#b08bd6"),
    ("(none)", ""),
];

pub struct TagStore {
    conn: Mutex<Connection>,
}

fn db_path() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let c = dir.join("hyperdrive.db");
            if c.exists() {
                return c;
            }
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    let d = PathBuf::from(home).join(".local/share/hyperdrive");
    let _ = std::fs::create_dir_all(&d);
    d.join("hyperdrive.db")
}

impl TagStore {
    pub fn open() -> Self {
        let conn = Connection::open(db_path())
            .unwrap_or_else(|_| Connection::open_in_memory().unwrap());
        let _ = conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS tags(
                path TEXT NOT NULL,
                color TEXT NOT NULL,
                PRIMARY KEY(path)
             );",
        );
        Self { conn: Mutex::new(conn) }
    }

    pub fn set(&self, path: &str, color: &str) {
        if color.is_empty() {
            if let Ok(c) = self.conn.lock() {
                let _ = c.execute("DELETE FROM tags WHERE path=?1", [path]);
            }
            return;
        }
        if let Ok(c) = self.conn.lock() {
            let _ = c.execute(
                "INSERT INTO tags(path,color) VALUES(?1,?2)
                 ON CONFLICT(path) DO UPDATE SET color=excluded.color",
                [path, color],
            );
        }
    }

    pub fn rename(&self, old_path: &str, new_path: &str) {
        if let Ok(c) = self.conn.lock() {
            let _ = c.execute(
                "UPDATE tags SET path=?2 WHERE path=?1",
                [old_path, new_path],
            );
        }
    }

    pub fn count_by_color(&self) -> std::collections::HashMap<String, usize> {
        let mut out = std::collections::HashMap::new();
        if let Ok(c) = self.conn.lock() {
            if let Ok(mut st) = c.prepare("SELECT color, COUNT(*) FROM tags GROUP BY color") {
                if let Ok(rows) = st.query_map([], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, usize>(1)?))
                }) {
                    for row in rows.flatten() {
                        out.insert(row.0, row.1);
                    }
                }
            }
        }
        out
    }

    pub fn all(&self) -> HashMap<PathBuf, String> {
        let mut out = HashMap::new();
        if let Ok(c) = self.conn.lock() {
            if let Ok(mut st) = c.prepare("SELECT path,color FROM tags") {
                if let Ok(rows) = st.query_map([], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                }) {
                    for row in rows.flatten() {
                        out.insert(PathBuf::from(row.0), row.1);
                    }
                }
            }
        }
        out
    }
}
