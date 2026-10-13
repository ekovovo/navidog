//! Dragged column widths (F-DATA-009): `profiles/<connection>/.column-widths.json`.
//! The leading dot cannot clash with a database folder: those names encode a leading `.` as `%2E`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::object::Scope;
use crate::paths;

const SEP: char = '\u{1f}';

#[derive(Serialize, Deserialize)]
struct File {
    version: u32,
    #[serde(default)]
    tables: BTreeMap<String, BTreeMap<String, f32>>,
}

fn file_path(root: &Path, connection: &str) -> PathBuf {
    root.join("profiles")
        .join(connection)
        .join(".column-widths.json")
}

/// `database \u{1f} schema \u{1f} table`. An empty schema is an empty segment.
fn table_key(scope: &Scope, table: &str) -> String {
    format!(
        "{}{SEP}{}{SEP}{table}",
        scope.database,
        scope.schema.as_deref().unwrap_or("")
    )
}

fn read_file(path: &Path) -> File {
    std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(File {
            version: 1,
            tables: BTreeMap::new(),
        })
}

fn write_file(path: &Path, file: &File) {
    let Ok(bytes) = serde_json::to_vec_pretty(file) else {
        return;
    };
    if let Err(err) = paths::write_atomic(path, &bytes) {
        tracing::error!("failed to save column widths: {err}");
    }
}

pub fn load(scope: &Scope, table: &str) -> BTreeMap<String, f32> {
    load_in(&paths::config_dir(), scope, table)
}

fn load_in(root: &Path, scope: &Scope, table: &str) -> BTreeMap<String, f32> {
    read_file(&file_path(root, &scope.connection))
        .tables
        .remove(&table_key(scope, table))
        .unwrap_or_default()
}

/// Merges `changed` into the widths already stored for this table. Other columns stay.
pub fn store(scope: &Scope, table: &str, changed: &BTreeMap<String, f32>) {
    if changed.is_empty() {
        return;
    }
    store_in(&paths::config_dir(), scope, table, changed);
}

fn store_in(root: &Path, scope: &Scope, table: &str, changed: &BTreeMap<String, f32>) {
    let path = file_path(root, &scope.connection);
    let mut file = read_file(&path);
    file.version = 1;
    file.tables
        .entry(table_key(scope, table))
        .or_default()
        .extend(changed.iter().map(|(k, v)| (k.clone(), *v)));
    write_file(&path, &file);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "navidog-widths-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn two_connections_keep_their_own_dragged_columns() {
        let root = temp();
        let a = Scope::new("c-a", "db", None);
        let b = Scope::new("c-b", "db", Some("public".into()));
        store_in(&root, &a, "actor", &BTreeMap::from([("id".into(), 72.)]));
        store_in(&root, &a, "actor", &BTreeMap::from([("name".into(), 140.)]));
        store_in(&root, &b, "actor", &BTreeMap::from([("id".into(), 90.)]));

        let a_widths = load_in(&root, &a, "actor");
        assert_eq!(a_widths.get("id"), Some(&72.));
        assert_eq!(a_widths.get("name"), Some(&140.));
        assert_eq!(load_in(&root, &b, "actor").get("id"), Some(&90.));
        assert!(load_in(&root, &a, "other").is_empty());

        // A database named like the metadata file cannot take its path
        let dotted = Scope::new("c-a", ".column-widths.json", None);
        store_in(&root, &dotted, "t", &BTreeMap::from([("id".into(), 10.)]));
        let meta = file_path(&root, "c-a");
        let encoded = root
            .join("profiles")
            .join("c-a")
            .join(crate::queries::encode_segment(".column-widths.json"));
        assert!(meta.is_file());
        assert_ne!(meta, encoded);
        assert_eq!(load_in(&root, &dotted, "t").get("id"), Some(&10.));
        assert_eq!(load_in(&root, &a, "actor").get("id"), Some(&72.));

        let _ = std::fs::remove_dir_all(&root);
    }
}
