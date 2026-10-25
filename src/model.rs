use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::util::now_ms;

fn default_version() -> u32 {
    2
}

fn is_zero(v: &i64) -> bool {
    *v == 0
}

fn is_false(v: &bool) -> bool {
    !*v
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Task {
    #[serde(rename = "taskId")]
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub done: bool,
    #[serde(rename = "parentId", default, skip_serializing_if = "String::is_empty")]
    pub parent: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub notes: String,
    #[serde(default)]
    pub created: i64,
    #[serde(default, skip_serializing_if = "is_false")]
    pub expanded: bool,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub due: i64,
    #[serde(rename = "remindAt", default, skip_serializing_if = "is_zero")]
    pub remind_at: i64,
    #[serde(default, skip_serializing_if = "is_false")]
    pub notified: bool,
    #[serde(rename = "deletedAt", default, skip_serializing_if = "is_zero")]
    pub deleted_at: i64,
}

impl Task {
    pub fn tags(&self) -> Vec<String> {
        parse_tags(&format!("{} {}", self.title, self.notes))
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Doc {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub items: Vec<Task>,
}

impl Default for Doc {
    fn default() -> Self {
        Doc {
            version: 2,
            items: Vec::new(),
        }
    }
}

pub fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/home/sora".into()))
}

pub fn state_path() -> PathBuf {
    if let Ok(p) = std::env::var("TASKS_RS_STATE") {
        return PathBuf::from(p);
    }
    home().join(".local/share/tasks-rs/state.json")
}

pub fn settings_path() -> PathBuf {
    home().join(".local/share/tasks-rs/settings.json")
}

pub fn load_doc(path: &Path) -> Result<Option<Doc>, String> {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("could not read {}: {e}", path.display())),
    };
    let mut doc: Doc = serde_json::from_str(&raw)
        .map_err(|e| format!("could not parse {}: {e}", path.display()))?;
    doc.version = 2;
    prune(&mut doc);
    Ok(Some(doc))
}

/// Remove dangling parents, self-parents and cycles. Never leaves a panic-prone graph.
pub fn prune(doc: &mut Doc) -> usize {
    let ids: HashSet<String> = doc.items.iter().map(|t| t.id.clone()).collect();
    let mut fixed = 0;
    for t in &mut doc.items {
        if !t.parent.is_empty() && (t.parent == t.id || !ids.contains(&t.parent)) {
            t.parent.clear();
            fixed += 1;
        }
    }
    let n = doc.items.len();
    for i in 0..n {
        let mut seen = HashSet::new();
        let mut cur = doc.items[i].parent.clone();
        let mut guard = n + 1;
        while !cur.is_empty() && guard > 0 {
            guard -= 1;
            if cur == doc.items[i].id {
                doc.items[i].parent.clear();
                fixed += 1;
                break;
            }
            if !seen.insert(cur.clone()) {
                doc.items[i].parent.clear();
                fixed += 1;
                break;
            }
            cur = doc
                .items
                .iter()
                .find(|t| t.id == cur)
                .map(|t| t.parent.clone())
                .unwrap_or_default();
        }
    }
    // De-duplicate ids just in case a hand-edited file contains them.
    let mut seen = HashSet::new();
    doc.items
        .retain(|t| !t.id.is_empty() && seen.insert(t.id.clone()));
    fixed
}

pub fn atomic_write(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let bak = path.with_extension("json.bak");
    if path.exists() {
        let _ = fs::copy(path, &bak);
    }
    let tmp = path.with_extension("json.tmp");
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    if let Some(dir) = path.parent() {
        if let Ok(d) = fs::File::open(dir) {
            let _ = d.sync_all();
        }
    }
    Ok(())
}

pub fn save_doc(path: &Path, doc: &Doc) -> Result<(), String> {
    let text = serde_json::to_string_pretty(doc).map_err(|e| e.to_string())?;
    atomic_write(path, &text).map_err(|e| format!("could not save {}: {e}", path.display()))
}

pub fn parse_tags(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for word in text.split_whitespace() {
        let Some(rest) = word.strip_prefix('#') else {
            continue;
        };
        let tag: String = rest
            .trim_matches(|c: char| {
                !(c.is_alphanumeric() || c == '_' || c == '-' || c == '/' || c == '.')
            })
            .trim_end_matches(['.', '/'])
            .trim_start_matches(['.', '/'])
            .to_string();
        if tag.is_empty() {
            continue;
        }
        if !out.iter().any(|t| t.eq_ignore_ascii_case(&tag)) {
            out.push(tag);
        }
    }
    out
}

pub fn task_matches(t: &Task, query: &str, tag: Option<&str>) -> bool {
    if let Some(want) = tag {
        if !t.tags().iter().any(|t| t.eq_ignore_ascii_case(want)) {
            return false;
        }
    }
    let q = query.trim();
    if q.is_empty() {
        return true;
    }
    if let Some(want) = q.strip_prefix('#') {
        let want = want.trim();
        return t
            .tags()
            .iter()
            .any(|t| t.to_lowercase().contains(&want.to_lowercase()));
    }
    let hay = format!("{} {} {}", t.title, t.notes, t.tags().join(" ")).to_lowercase();
    let q = q.to_lowercase();
    hay.contains(&q) || subsequence(&hay, &q)
}

pub fn subsequence(hay: &str, needle: &str) -> bool {
    let mut chars = hay.chars();
    needle.chars().all(|n| chars.by_ref().any(|h| h == n))
}

pub fn new_task(title: &str, parent: &str) -> Task {
    let title = title.trim();
    Task {
        id: format!("r{}{}", now_ms(), rand_suffix()),
        title: title.to_string(),
        done: false,
        parent: parent.to_string(),
        notes: String::new(),
        created: now_ms(),
        expanded: false,
        due: 0,
        remind_at: 0,
        notified: false,
        deleted_at: 0,
    }
}

fn rand_suffix() -> u32 {
    use std::sync::atomic::{AtomicU32, Ordering};
    static SEQ: AtomicU32 = AtomicU32::new(1);
    let t = now_ms() as u32;
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    t.rotate_left(7) ^ n.wrapping_mul(2_654_435_761)
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Settings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<f32>,
}

pub fn load_settings() -> Settings {
    fs::read_to_string(settings_path())
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

pub fn save_settings(s: &Settings) {
    if let Ok(text) = serde_json::to_string_pretty(s) {
        let _ = atomic_write(&settings_path(), &text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(title: &str, notes: &str) -> Task {
        let mut t = new_task(title, "");
        t.notes = notes.into();
        t
    }

    #[test]
    fn parses_hashtags() {
        let t = task("ship #Work and #work", "notes #idea, #deep-work.");
        let tags = t.tags();
        assert_eq!(tags, vec!["Work", "idea", "deep-work"]);
        let t = task("#paren #tag! #a/b", "");
        assert_eq!(t.tags(), vec!["paren", "tag", "a/b"]);
        assert!(parse_tags("no tags here").is_empty());
    }

    #[test]
    fn tag_and_text_search() {
        let t = task("write report #work", "");
        assert!(task_matches(&t, "#work", None));
        assert!(!task_matches(&t, "#home", None));
        assert!(task_matches(&t, "", Some("work")));
        assert!(task_matches(&t, "wrt", None)); // fuzzy subsequence
        assert!(!task_matches(&t, "zzz", None));
    }

    #[test]
    fn prunes_dangling_and_cycles() {
        let mut doc = Doc::default();
        let mut a = new_task("a", "");
        a.id = "a".into();
        let mut b = new_task("b", "missing");
        b.id = "b".into();
        let mut c = new_task("c", "a");
        c.id = "c".into();
        let mut d = new_task("d", "e");
        d.id = "d".into();
        let mut e = new_task("e", "d");
        e.id = "e".into();
        doc.items = vec![a, b, c, d, e];
        prune(&mut doc);
        assert_eq!(doc.items[1].parent, "");
        assert_eq!(doc.items[2].parent, "a");
        assert_eq!(doc.items[3].parent, "");
        // Breaking one edge is enough: there is no cycle left.
        assert!(doc.items[4].parent != "d" || doc.items[3].parent != "e");
        assert_eq!(prune(&mut doc), 0);
    }

    #[test]
    fn atomic_save_creates_backup() {
        let dir = std::env::temp_dir().join(format!("tasks-rs-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join("state.json");
        let mut doc = Doc::default();
        doc.items.push(new_task("one", ""));
        save_doc(&path, &doc).unwrap();
        assert!(path.exists());
        assert!(!path.with_extension("json.bak").exists());
        doc.items.push(new_task("two", ""));
        save_doc(&path, &doc).unwrap();
        assert!(path.with_extension("json.bak").exists());
        let back = load_doc(&path).unwrap().unwrap();
        assert_eq!(back.items.len(), 2);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn unreadable_state_is_an_error_not_default() {
        let dir = std::env::temp_dir().join(format!("tasks-rs-bad-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.json");
        fs::write(&path, "{ definitely not json").unwrap();
        assert!(load_doc(&path).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "{ definitely not json");
        let _ = fs::remove_dir_all(&dir);
    }
}
