//! Saved queries (30-query-editor.md §7): `.sql` files under the connection's profile folder,
//! `profiles/<connection>/<database>[/<schema>]/<name>.sql`. Listing them needs no driver.

use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::object::Scope;
use crate::paths;

/// Percent-encodes the characters a folder or file name cannot hold (`/`, `\`, `:`, `%`) and a
/// leading `.`, so a database named `..` or `a/b` stays inside its folder.
pub fn encode_segment(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for (i, ch) in name.chars().enumerate() {
        match ch {
            '/' | '\\' | ':' | '%' => out.push_str(&format!("%{:02X}", ch as u32)),
            '.' if i == 0 => out.push_str("%2E"),
            _ => out.push(ch),
        }
    }
    out
}

pub fn decode_segment(name: &str) -> String {
    let bytes = name.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(v) = name
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The folder holding the queries of a database (or PostgreSQL schema).
pub fn dir(scope: &Scope) -> PathBuf {
    let mut dir = paths::profile_dir(&scope.connection).join(encode_segment(&scope.database));
    if let Some(schema) = &scope.schema {
        dir = dir.join(encode_segment(schema));
    }
    dir
}

pub fn path(scope: &Scope, name: &str) -> PathBuf {
    dir(scope).join(format!("{}.sql", encode_segment(name)))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueryFile {
    pub name: String,
    pub modified: Option<SystemTime>,
    pub size: u64,
}

/// The queries saved in a scope, sorted by name.
pub fn list(scope: &Scope) -> Vec<QueryFile> {
    let Ok(entries) = std::fs::read_dir(dir(scope)) else {
        return Vec::new();
    };
    let mut out: Vec<QueryFile> = entries
        .flatten()
        .filter_map(|e| {
            let file = e.file_name().to_string_lossy().into_owned();
            let stem = file.strip_suffix(".sql")?;
            let meta = e.metadata().ok().filter(|m| m.is_file())?;
            Some(QueryFile {
                name: decode_segment(stem),
                modified: meta.modified().ok(),
                size: meta.len(),
            })
        })
        .collect();
    out.sort_by_key(|q| q.name.to_lowercase());
    out
}

pub fn read(scope: &Scope, name: &str) -> io::Result<String> {
    std::fs::read_to_string(path(scope, name))
}

pub fn write(scope: &Scope, name: &str, text: &str) -> io::Result<()> {
    paths::write_atomic(&path(scope, name), text.as_bytes())
}

pub fn delete(scope: &Scope, name: &str) -> io::Result<()> {
    std::fs::remove_file(path(scope, name))
}

pub fn rename(scope: &Scope, old: &str, new: &str) -> io::Result<()> {
    std::fs::rename(path(scope, old), path(scope, new))
}

/// Copies a query to the first free `name_copyN` and returns that name.
pub fn duplicate(scope: &Scope, name: &str) -> io::Result<String> {
    let text = read(scope, name)?;
    let taken: Vec<String> = list(scope).into_iter().map(|q| q.name).collect();
    let copy = (1..)
        .map(|n| format!("{name}_copy{n}"))
        .find(|c| !taken.iter().any(|t| t.eq_ignore_ascii_case(c)))
        .expect("an unused copy name");
    write(scope, &copy, &text)?;
    Ok(copy)
}

/// A usable query name for `scope`: not empty, no `/` or `:`, no leading `.`, and not already
/// taken ignoring case. `current` is the query being renamed or saved over, which may keep its name.
pub fn name_ok(scope: &Scope, name: &str, current: Option<&str>) -> bool {
    let name = name.trim();
    if name.is_empty() || name.contains(['/', ':']) || name.starts_with('.') {
        return false;
    }
    if current.is_some_and(|c| c.eq_ignore_ascii_case(name)) {
        return true;
    }
    !list(scope)
        .iter()
        .any(|q| q.name.eq_ignore_ascii_case(name))
}

/// Modification time of a file, to notice edits made outside the app.
pub fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).ok()?.modified().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments_round_trip_and_stay_inside() {
        for name in ["a/b", "..", ".hidden", "50%:off", "普通"] {
            let enc = encode_segment(name);
            assert!(!enc.contains(['/', ':']) && !enc.starts_with('.'), "{enc}");
            assert_eq!(decode_segment(&enc), name);
        }
        assert_eq!(encode_segment("a.b"), "a.b");
    }

    #[test]
    fn saves_lists_renames_duplicates_and_deletes() {
        let home = std::env::temp_dir().join(format!("navidog-queries-{}", std::process::id()));
        // SAFETY: tests in this crate that touch the config folder use their own NAVIDOG_HOME
        unsafe { std::env::set_var("NAVIDOG_HOME", &home) };
        let scope = Scope::new("c1", "db/1", Some("public".into()));
        write(&scope, "b", "SELECT 2").unwrap();
        write(&scope, "A", "SELECT 1").unwrap();
        assert!(path(&scope, "A").starts_with(home.join("profiles/c1/db%2F1/public")));
        let names: Vec<_> = list(&scope).into_iter().map(|q| q.name).collect();
        assert_eq!(names, ["A", "b"]);
        assert!(!name_ok(&scope, "a", None));
        assert!(name_ok(&scope, "a", Some("A")));
        assert!(!name_ok(&scope, ".x", None) && !name_ok(&scope, "x/y", None));
        rename(&scope, "b", "c").unwrap();
        assert_eq!(duplicate(&scope, "c").unwrap(), "c_copy1");
        assert_eq!(duplicate(&scope, "c").unwrap(), "c_copy2");
        assert_eq!(read(&scope, "c_copy2").unwrap(), "SELECT 2");
        delete(&scope, "c").unwrap();
        let names: Vec<_> = list(&scope).into_iter().map(|q| q.name).collect();
        assert_eq!(names, ["A", "c_copy1", "c_copy2"]);
        let _ = std::fs::remove_dir_all(&home);
    }
}
