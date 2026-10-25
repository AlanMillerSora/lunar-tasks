use chrono::{Datelike, Local, TimeZone, Timelike};
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Id,
    Layout, Pos2, Rect, Sense, Stroke, Vec2,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static TOGGLE: AtomicBool = AtomicBool::new(false);
extern "C" fn on_sigusr1(_: libc::c_int) { TOGGLE.store(true, Ordering::SeqCst); }

const PANEL_W: f32 = 640.0;
const PANEL_H: f32 = 880.0;

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/home/sora".into()))
}

fn state_path() -> PathBuf {
    home().join(".local/share/tasks-rs/state.json")
}

fn default_version() -> u32 {
    2
}

fn is_zero(v: &i64) -> bool {
    *v == 0
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Task {
    #[serde(rename = "taskId")]
    id: String,
    title: String,
    #[serde(default, skip_serializing_if = "is_false")]
    done: bool,
    #[serde(rename = "parentId", default, skip_serializing_if = "String::is_empty")]
    parent: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    notes: String,
    #[serde(default)]
    created: i64,
    #[serde(default, skip_serializing_if = "is_false")]
    expanded: bool,
    #[serde(default, skip_serializing_if = "is_zero")]
    due: i64,
    #[serde(rename = "remindAt", default, skip_serializing_if = "is_zero")]
    remind_at: i64,
    #[serde(default, skip_serializing_if = "is_false")]
    notified: bool,
    #[serde(rename = "deletedAt", default, skip_serializing_if = "is_zero")]
    deleted_at: i64,
}

fn is_false(v: &bool) -> bool {
    !*v
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Doc {
    #[serde(default = "default_version")]
    version: u32,
    #[serde(default)]
    items: Vec<Task>,
}

impl Default for Doc {
    fn default() -> Self {
        Doc { version: 2, items: Vec::new() }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Filter {
    All,
    Open,
    Done,
    Trash,
}

struct Theme {
    bg: Color32,
    fg: Color32,
    dim: Color32,
    surface: Color32,
    accent: Color32,
    accent2: Color32,
    danger: Color32,
    font: String,
    icon: String,
}

fn hex(s: &str, fallback: Color32) -> Color32 {
    let h = s.trim().trim_start_matches('#');
    if h.len() < 6 { return fallback; }
    let r = u8::from_str_radix(&h[0..2], 16).ok();
    let g = u8::from_str_radix(&h[2..4], 16).ok();
    let b = u8::from_str_radix(&h[4..6], 16).ok();
    match (r, g, b) {
        (Some(r), Some(g), Some(b)) => Color32::from_rgb(r, g, b),
        _ => fallback,
    }
}

fn load_theme() -> Theme {
    let mut t = Theme {
        bg: Color32::from_rgb(13, 9, 28),
        fg: Color32::from_rgb(206, 205, 209),
        dim: Color32::from_rgb(130, 128, 140),
        surface: Color32::from_rgb(24, 19, 42),
        accent: Color32::from_rgb(249, 145, 197),
        accent2: Color32::from_rgb(193, 102, 213),
        danger: Color32::from_rgb(255, 107, 138),
        font: "JetBrainsMono NFP".into(),
        icon: "JetBrainsMono Nerd Font".into(),
    };
    let path = home().join(".cache/coloreclipse/colors.json");
    if let Ok(raw) = fs::read_to_string(path) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
            let special = &v["special"];
            if let Some(s) = special["background"].as_str() { t.bg = hex(s, t.bg); }
            if let Some(s) = special["foreground"].as_str() { t.fg = hex(s, t.fg); }
            if let Some(s) = v["colors"]["color5"].as_str() { t.accent = hex(s, t.accent); }
            if let Some(s) = v["colors"]["color9"].as_str() { t.accent2 = hex(s, t.accent2); }
            if let Some(s) = v["colors"]["color3"].as_str() {
                let rose = hex(s, t.fg);
                t.dim = mix(rose, t.bg, 0.45);
            }
            t.surface = mix(t.bg, t.fg, 0.055);
        }
    }
    t
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(f(a.r(), b.r()), f(a.g(), b.g()), f(a.b(), b.b()))
}

fn rgba(c: Color32, a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (a.clamp(0.0, 1.0) * 255.0) as u8)
}

fn install_fonts(ctx: &egui::Context) {
    let mut defs = FontDefinitions::default();
    let regular = "/usr/share/fonts/TTF/JetBrainsMonoNerdFont-Regular.ttf";
    let bold = "/usr/share/fonts/TTF/JetBrainsMonoNerdFont-Bold.ttf";
    if let Ok(bytes) = fs::read(regular) {
        defs.font_data.insert("jbmono".into(), Arc::new(FontData::from_owned(bytes)));
        for fam in [FontFamily::Monospace, FontFamily::Proportional] {
            if let Some(list) = defs.families.get_mut(&fam) {
                list.insert(0, "jbmono".into());
            }
        }
    }
    if let Ok(bytes) = fs::read(bold) {
        defs.font_data.insert("jbmono-bold".into(), Arc::new(FontData::from_owned(bytes)));
        defs.families.insert(FontFamily::Name("bold".into()), vec!["jbmono-bold".into()]);
    }
    ctx.set_fonts(defs);

    let mut style = (*ctx.style_of(egui::Theme::Dark)).clone();
    style.text_styles = [
        (egui::TextStyle::Small, FontId::new(12.0, FontFamily::Monospace)),
        (egui::TextStyle::Body, FontId::new(15.0, FontFamily::Monospace)),
        (egui::TextStyle::Button, FontId::new(13.0, FontFamily::Monospace)),
        (egui::TextStyle::Heading, FontId::new(16.0, FontFamily::Monospace)),
        (egui::TextStyle::Monospace, FontId::new(13.0, FontFamily::Monospace)),
    ].into();
    style.visuals = egui::Visuals::dark();
    style.visuals.panel_fill = Color32::TRANSPARENT;
    style.visuals.window_fill = Color32::from_rgb(18, 13, 33);
    style.visuals.override_text_color = Some(Color32::from_rgb(206, 205, 209));
    style.visuals.widgets.noninteractive.bg_stroke = Stroke::NONE;
    style.visuals.widgets.inactive.bg_fill = Color32::TRANSPARENT;
    style.visuals.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
    style.visuals.widgets.inactive.bg_stroke = Stroke::NONE;
    style.visuals.selection.bg_fill = Color32::from_rgba_unmultiplied(249, 145, 197, 60);
    ctx.set_style_of(egui::Theme::Dark, style);
    ctx.set_theme(egui::Theme::Dark);
}

struct App {
    doc: Doc,
    path: PathBuf,
    theme: Theme,
    filter: Filter,
    input: String,
    search: String,
    selected: Option<String>,
    undo: Vec<Vec<Task>>,
    redo: Vec<Vec<Task>>,
    dirty: bool,
    last_save: Instant,
    toast: Option<(String, Instant)>,
    link_from: Option<String>,
    multi: HashSet<String>,
    drag_id: Option<String>,
    drag_y: f32,
    due_edit: Option<(String, String)>,
    edit: Option<(String, String, String)>,
    last_reminder: Instant,
    had_focus: bool,
    hidden: bool,
    first_frame: bool,
    win_h: f32,
    id_seq: u64,
}

impl App {
    fn new() -> Self {
        let path = state_path();
        let doc = fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str::<Doc>(&raw).ok())
            .unwrap_or_default();
        let theme = load_theme();
        let mut app = App {
            doc,
            path,
            theme,
            filter: Filter::All,
            input: String::new(),
            search: String::new(),
            selected: None,
            undo: Vec::new(),
            redo: Vec::new(),
            dirty: false,
            last_save: Instant::now(),
            toast: None,
            link_from: None,
            multi: HashSet::new(),
            drag_id: None,
            drag_y: -1.0,
            due_edit: None,
            edit: None,
            last_reminder: Instant::now(),
            had_focus: false,
            hidden: false,
            first_frame: true,
            win_h: PANEL_H,
            id_seq: 0,
        };
        app.seed_if_empty();
        app
    }

    fn seed_if_empty(&mut self) {
        if self.doc.items.is_empty() {
            let now = now_ms();
            self.doc.items.push(Task {
                id: "welcome".into(),
                title: "Добро пожаловать в Tasks".into(),
                done: false,
                parent: String::new(),
                notes: "Тяни узел на другую задачу — получится связь.".into(),
                created: now,
                expanded: false,
                due: 0,
                remind_at: 0,
                notified: false,
                deleted_at: 0,
            });
            self.dirty = true;
            self.save_now();
        }
    }

    fn save_now(&mut self) {
        if let Some(dir) = self.path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        if let Ok(text) = serde_json::to_string_pretty(&self.doc) {
            if self.path.exists() {
                let _ = fs::copy(&self.path, self.path.with_extension("json.bak"));
            }
            let tmp = self.path.with_extension("json.tmp");
            if fs::write(&tmp, text).is_ok() {
                let _ = fs::rename(&tmp, &self.path);
            }
        }
        self.dirty = false;
        self.last_save = Instant::now();
    }

    fn snapshot(&mut self) {
        self.undo.push(self.doc.items.clone());
        if self.undo.len() > 60 {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    fn undo(&mut self) {
        if let Some(prev) = self.undo.pop() {
            self.redo.push(self.doc.items.clone());
            self.doc.items = prev;
            self.dirty = true;
            self.toast("Undo");
        }
    }

    fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            self.undo.push(self.doc.items.clone());
            self.doc.items = next;
            self.dirty = true;
            self.toast("Redo");
        }
    }

    fn toast(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), Instant::now()));
    }

    fn next_id(&mut self) -> String {
        self.id_seq += 1;
        format!("r{}", now_ms().to_string().replace('.', "") + &self.id_seq.to_string())
    }

    fn active(&self) -> impl Iterator<Item = &Task> {
        self.doc.items.iter().filter(|t| t.deleted_at == 0)
    }

    fn counts(&self) -> (usize, usize, usize) {
        let mut open = 0;
        let mut done = 0;
        let mut trash = 0;
        for t in &self.doc.items {
            if t.deleted_at > 0 { trash += 1; }
            else if t.done { done += 1; }
            else { open += 1; }
        }
        (open, done, trash)
    }

    fn task(&self, id: &str) -> Option<&Task> {
        self.doc.items.iter().find(|t| t.id == id)
    }

    fn task_mut(&mut self, id: &str) -> Option<&mut Task> {
        self.doc.items.iter_mut().find(|t| t.id == id)
    }

    fn toggle(&mut self, id: &str) {
        self.snapshot();
        if let Some(t) = self.task_mut(id) {
            t.done = !t.done;
        }
        self.dirty = true;
    }

    fn add(&mut self, title: &str) {
        let title = title.trim();
        if title.is_empty() { return; }
        self.snapshot();
        let id = self.next_id();
        self.doc.items.insert(0, Task {
            id: id.clone(),
            title: title.to_string(),
            done: false,
            parent: String::new(),
            notes: String::new(),
            created: now_ms(),
            expanded: false,
            due: 0,
            remind_at: 0,
            notified: false,
            deleted_at: 0,
        });
        self.selected = Some(id);
        self.dirty = true;
    }

    fn soft_delete(&mut self, id: &str) {
        self.snapshot();
        let stamp = now_ms();
        let mut ids = vec![id.to_string()];
        let mut i = 0;
        while i < ids.len() {
            let cur = ids[i].clone();
            for t in &self.doc.items {
                if t.parent == cur && !ids.contains(&t.id) {
                    ids.push(t.id.clone());
                }
            }
            i += 1;
        }
        let mut n = 0;
        for t in &mut self.doc.items {
            if ids.contains(&t.id) && t.deleted_at == 0 {
                t.deleted_at = stamp;
                n += 1;
            }
        }
        if self.selected.as_deref() == Some(id) { self.selected = None; }
        self.dirty = true;
        self.toast(format!("Moved {} task{} to trash", n, if n == 1 { "" } else { "s" }));
    }

    fn restore(&mut self, id: &str) {
        let stamp = match self.task(id) {
            Some(t) if t.deleted_at > 0 => t.deleted_at,
            _ => return,
        };
        self.snapshot();
        let mut ids = vec![id.to_string()];
        let mut i = 0;
        while i < ids.len() {
            let cur = ids[i].clone();
            for t in &self.doc.items {
                if t.parent == cur && !ids.contains(&t.id) {
                    ids.push(t.id.clone());
                }
            }
            i += 1;
        }
        for t in &mut self.doc.items {
            if ids.contains(&t.id) && t.deleted_at == stamp {
                t.deleted_at = 0;
            }
        }
        self.dirty = true;
        self.toast("Restored");
    }

    fn purge(&mut self, id: &str) {
        self.snapshot();
        self.doc.items.retain(|t| t.id != id);
        self.dirty = true;
        self.toast("Deleted forever");
    }

    fn set_done_many(&mut self, ids: &[String], done: bool) {
        if ids.is_empty() { return; }
        self.snapshot();
        let mut n = 0;
        for t in &mut self.doc.items {
            if ids.contains(&t.id) && t.deleted_at == 0 && t.done != done {
                t.done = done;
                n += 1;
            }
        }
        if n > 0 {
            self.dirty = true;
            self.toast(format!("{} {} task{}", if done { "Completed" } else { "Reopened" }, n, if n == 1 { "" } else { "s" }));
        }
    }

    fn delete_many(&mut self, ids: &[String]) {
        if ids.is_empty() { return; }
        self.snapshot();
        let stamp = now_ms();
        let mut all: Vec<String> = ids.to_vec();
        let mut i = 0;
        while i < all.len() {
            let cur = all[i].clone();
            for t in &self.doc.items {
                if t.parent == cur && !all.contains(&t.id) { all.push(t.id.clone()); }
            }
            i += 1;
        }
        let mut n = 0;
        for t in &mut self.doc.items {
            if all.contains(&t.id) && t.deleted_at == 0 { t.deleted_at = stamp; n += 1; }
        }
        self.selected = None;
        self.multi.clear();
        if n > 0 {
            self.dirty = true;
            self.toast(format!("Moved {} task{} to trash", n, if n == 1 { "" } else { "s" }));
        }
    }

    fn move_to(&mut self, id: &str, target: usize) {
        let Some(cur) = self.doc.items.iter().position(|t| t.id == id) else { return };
        let target = target.min(self.doc.items.len().saturating_sub(1));
        if cur == target { return; }
        self.snapshot();
        let item = self.doc.items.remove(cur);
        self.doc.items.insert(target, item);
        self.dirty = true;
    }

    fn link(&mut self, child: &str, parent: &str) -> bool {
        if child == parent || child.is_empty() || parent.is_empty() {
            return false;
        }
        if self.is_ancestor(child, parent) {
            self.toast("Can't link into its own branch");
            return false;
        }
        self.snapshot();
        if let Some(t) = self.task_mut(child) {
            t.parent = parent.to_string();
        }
        // keep the graph top-down: move the child just below the parent
        if let Some(ci) = self.doc.items.iter().position(|t| t.id == child) {
            let item = self.doc.items.remove(ci);
            if let Some(pi) = self.doc.items.iter().position(|t| t.id == parent) {
                self.doc.items.insert(pi + 1, item);
            } else {
                self.doc.items.insert(0, item);
            }
        }
        self.dirty = true;
        let ct = self.task(child).map(|t| t.title.clone()).unwrap_or_default();
        let pt = self.task(parent).map(|t| t.title.clone()).unwrap_or_default();
        self.toast(format!("Linked «{}» under «{}»", truncate(&ct, 24), truncate(&pt, 24)));
        true
    }

    fn is_ancestor(&self, ancestor: &str, node: &str) -> bool {
        let mut cur = node.to_string();
        let mut guard = self.doc.items.len() + 2;
        while !cur.is_empty() && guard > 0 {
            guard -= 1;
            match self.task(&cur) {
                Some(t) => {
                    if t.parent == ancestor { return true; }
                    cur = t.parent.clone();
                }
                None => return false,
            }
        }
        false
    }

    fn lane(&self, id: &str) -> usize {
        let mut depth = 0;
        let mut cur = id.to_string();
        let mut guard = self.doc.items.len() + 2;
        while guard > 0 {
            guard -= 1;
            match self.task(&cur) {
                Some(t) if t.deleted_at == 0 && !t.parent.is_empty() => {
                    depth += 1;
                    cur = t.parent.clone();
                }
                _ => break,
            }
        }
        depth.min(3)
    }

    fn visible(&self) -> Vec<&Task> {
        self.doc.items.iter().filter(|t| {
            if self.filter == Filter::Trash {
                if t.deleted_at == 0 { return false; }
            } else {
                if t.deleted_at > 0 { return false; }
                match self.filter {
                    Filter::All => {}
                    Filter::Open => if t.done { return false; },
                    Filter::Done => if !t.done { return false; },
                    Filter::Trash => {}
                }
            }
            if !self.search.is_empty() {
                let hay = format!("{} {}", t.title, t.notes).to_lowercase();
                let q = self.search.to_lowercase();
                if !hay.contains(&q) && !subsequence(&hay, &q) { return false; }
            }
            true
        }).collect()
    }

    fn check_reminders(&mut self) {
        if self.last_reminder.elapsed() < Duration::from_secs(20) {
            return;
        }
        self.last_reminder = Instant::now();
        let now = now_ms();
        let mut fired: Vec<(String, String, i64)> = Vec::new();
        for t in &self.doc.items {
            if t.deleted_at == 0 && !t.done && t.remind_at > 0 && !t.notified && t.remind_at <= now {
                fired.push((t.id.clone(), t.title.clone(), t.due));
            }
        }
        if fired.is_empty() { return; }
        for (id, title, due) in &fired {
            let body = if *due > 0 { format!("{} — {}", title, due_label(*due)) } else { title.clone() };
            let _ = std::process::Command::new("notify-send")
                .args(["-a", "Tasks", "-u", "normal", "Task due", &body])
                .spawn();
            if let Some(t) = self.task_mut(id) { t.notified = true; }
        }
        self.dirty = true;
        self.toast(format!("Reminder: {}", truncate(&fired[0].1, 30)));
    }
}

fn subsequence(hay: &str, needle: &str) -> bool {
    let mut chars = hay.chars();
    needle.chars().all(|n| chars.by_ref().any(|h| h == n))
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n { return s.to_string(); }
    let mut out: String = s.chars().take(n.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn age(ms: i64) -> String {
    let d = now_ms() - ms;
    if d < 60_000 { "now".into() }
    else if d < 3_600_000 { format!("{}m", d / 60_000) }
    else if d < 86_400_000 { format!("{}h", d / 3_600_000) }
    else if d < 604_800_000 { format!("{}d", d / 86_400_000) }
    else {
        let dt = Local.timestamp_millis_opt(ms).single();
        dt.map(|t| t.format("%-d %b").to_string()).unwrap_or_else(|| "—".into())
    }
}

fn due_label(ms: i64) -> String {
    if ms <= 0 { return String::new(); }
    let now = now_ms();
    let dt = match Local.timestamp_millis_opt(ms).single() { Some(d) => d, None => return String::new() };
    if ms < now {
        let d = now - ms;
        if d < 3_600_000 { return format!("{}m late", (d / 60_000).max(1)); }
        if d < 86_400_000 { return format!("{}h late", d / 3_600_000); }
        return format!("{}d late", d / 86_400_000);
    }
    let t = format!("{:02}:{:02}", dt.hour(), dt.minute());
    let today = Local::now().date_naive();
    if dt.date_naive() == today { t }
    else if dt.date_naive() == today.succ_opt().unwrap_or(today) { format!("tomorrow {}", t) }
    else if (dt.date_naive() - today).num_days() < 7 {
        format!("{} {}", dt.format("%a"), t)
    } else {
        dt.format("%-d %b").to_string()
    }
}

fn parse_when(text: &str) -> Option<i64> {
    let text = text.trim().to_lowercase();
    if text.is_empty() { return None; }
    let now = Local::now();
    if let Some(rest) = text.strip_prefix('+') {
        let split = rest.find(|c: char| !c.is_ascii_digit() && c != '.')?;
        let n: f64 = rest[..split].parse().ok()?;
        let unit = rest[split..].trim();
        let ms = match unit {
            "m" | "min" | "mins" => 60_000.0,
            "h" | "hr" | "hrs" => 3_600_000.0,
            "d" | "day" | "days" => 86_400_000.0,
            "w" | "week" | "weeks" => 604_800_000.0,
            _ => return None,
        };
        return Some(now_ms() + (n * ms) as i64);
    }
    let (day_offset, text) = if let Some(r) = text.strip_prefix("tomorrow ") {
        (1, r)
    } else if let Some(r) = text.strip_prefix("today ") {
        (0, r)
    } else {
        (0, text.as_str())
    };
    let make = |y: i32, mo: u32, d: u32, h: u32, mi: u32| {
        Local.with_ymd_and_hms(y, mo, d, h, mi, 0).single().map(|t| t.timestamp_millis())
    };
    if let Ok(t) = chrono::NaiveTime::parse_from_str(text, "%H:%M") {
        let mut date = now.date_naive() + chrono::Duration::days(day_offset);
        if day_offset == 0 && t <= now.time() { date += chrono::Duration::days(1); }
        return make(date.year(), date.month(), date.day(), t.hour(), t.minute());
    }
    for fmt in ["%Y-%m-%d %H:%M", "%Y-%m-%d", "%d.%m %H:%M", "%d.%m"] {
        if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(text, fmt) {
            return make(dt.year(), dt.month(), dt.day(), dt.hour(), dt.minute());
        }
    }
    None
}

fn paint_text(
    painter: &egui::Painter,
    pos: Pos2,
    text: &str,
    size: f32,
    color: Color32,
    bold: bool,
    clip: Rect,
) {
    let fam = if bold { FontFamily::Name("bold".into()) } else { FontFamily::Monospace };
    let p = painter.with_clip_rect(clip);
    p.text(pos, Align2::LEFT_CENTER, text, FontId::new(size, fam), color);
}

fn text_width(painter: &egui::Painter, text: &str, size: f32, bold: bool) -> f32 {
    let fam = if bold { FontFamily::Name("bold".into()) } else { FontFamily::Monospace };
    painter.layout_no_wrap(text.to_string(), FontId::new(size, fam), Color32::WHITE).size().x
}

impl App {
    fn hide_window(&mut self, ctx: &egui::Context) {
        self.save_now();
        self.hidden = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
    }

    fn show_window(&mut self, ctx: &egui::Context) {
        self.hidden = false;
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    }

    fn ui_header(&mut self, ui: &mut egui::Ui) {
        let (open, done, _) = self.counts();
        let total = open + done;
        let pct = if total > 0 { (done * 100 + total / 2) / total } else { 0 };
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(Vec2::new(22.0, 26.0), Sense::hover());
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, "✦", FontId::new(20.0, FontFamily::Monospace), self.theme.accent);
            ui.add_space(6.0);
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("TASKS").size(14.0).color(self.theme.fg).strong());
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new(format!("{}%", pct)).size(13.0).color(self.theme.accent));
                });
                ui.label(egui::RichText::new(format!("{} open · {} done", open, done)).size(13.0).color(self.theme.dim));
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.add(egui::Button::new(egui::RichText::new("✕").size(16.0).color(self.theme.dim)).frame(false)).clicked() {
                    self.save_now();
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
                if ui.add(egui::Button::new(egui::RichText::new("+").size(20.0).color(self.theme.dim)).frame(false)).clicked() {
                    // focus the bottom input
                    ui.ctx().memory_mut(|m| m.request_focus(Id::new("task-input")));
                }
            });
        });
    }

    fn ui_progress(&mut self, ui: &mut egui::Ui) {
        let (open, done, _) = self.counts();
        let total = open + done;
        let ratio = if total > 0 { done as f32 / total as f32 } else { 0.0 };
        let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 4.0), Sense::hover());
        let p = ui.painter();
        p.rect_filled(rect, CornerRadius::same(2), rgba(self.theme.fg, 0.07));
        if ratio > 0.001 {
            let fill = Rect::from_min_size(rect.min, Vec2::new(rect.width() * ratio, rect.height()));
            p.rect_filled(fill, CornerRadius::same(2), self.theme.accent);
        }
        ui.add_space(10.0);
    }

    fn ui_filters(&mut self, ui: &mut egui::Ui) {
        let (open, done, trash) = self.counts();
        let items = [
            ("All", (open + done) as usize, Filter::All),
            ("Open", open, Filter::Open),
            ("Done", done, Filter::Done),
            ("Trash", trash, Filter::Trash),
        ];
        ui.horizontal(|ui| {
            for (label, count, f) in items {
                let active = self.filter == f;
                let color = if active { self.theme.accent } else { rgba(self.theme.fg, 0.55) };
                let text = format!("{}  {}", label, count);
                let galley = ui.painter().layout_no_wrap(text, FontId::new(13.0, FontFamily::Monospace), color);
                let (rect, resp) = ui.allocate_exact_size(Vec2::new(galley.size().x + 10.0, 28.0), Sense::click());
                let p = ui.painter();
                p.text(rect.center(), Align2::CENTER_CENTER, galley.job.text.clone(), FontId::new(13.0, FontFamily::Monospace), color);
                if active {
                    let line = Rect::from_min_size(Pos2::new(rect.left(), rect.bottom() - 2.0), Vec2::new(rect.width(), 1.5));
                    p.rect_filled(line, CornerRadius::ZERO, self.theme.accent);
                }
                if resp.clicked() { self.filter = f; }
                ui.add_space(10.0);
            }
        });
        ui.add_space(8.0);
    }

    fn ui_list(&mut self, ui: &mut egui::Ui) {
        let tasks: Vec<Task> = self.visible().into_iter().cloned().collect();
        let width = ui.available_width();
        let heights: Vec<f32> = tasks.iter().map(|t| if t.notes.is_empty() { 54.0 } else { 70.0 }).collect();
        let total_h: f32 = heights.iter().sum::<f32>() + 4.0;
        let (content_rect, _) = ui.allocate_exact_size(Vec2::new(width, total_h.max(60.0)), Sense::hover());
        // positions
        let mut positions: Vec<(String, Rect)> = Vec::new();
        let mut y = content_rect.top() + 2.0;
        for (t, h) in tasks.iter().zip(heights.iter()) {
            let r = Rect::from_min_size(Pos2::new(content_rect.left(), y), Vec2::new(width, *h));
            positions.push((t.id.clone(), r));
            y += *h;
        }
        let pos_of = |id: &str| positions.iter().find(|(i, _)| i == id).map(|(_, r)| *r);

        // graph behind rows
        let p = ui.painter();
        let lane_x = |lane: usize| content_rect.left() + 22.0 + lane as f32 * 17.0;
        let node_y = |r: Rect| r.top() + 24.0;
        // root spine
        let roots: Vec<Rect> = tasks.iter().filter(|t| t.parent.is_empty()).filter_map(|t| pos_of(&t.id)).collect();
        for w in roots.windows(2) {
            let x = lane_x(0);
            p.line_segment([Pos2::new(x, node_y(w[0])), Pos2::new(x, node_y(w[1]))], Stroke::new(1.0, rgba(self.theme.fg, 0.08)));
        }
        let palette = [self.theme.accent, self.theme.accent2, Color32::from_rgb(94, 124, 165), Color32::from_rgb(255, 167, 211)];
        for t in &tasks {
            if t.parent.is_empty() { continue; }
            let Some(pr) = pos_of(&t.parent) else { continue };
            let Some(cr) = pos_of(&t.id) else { continue };
            let lane = self.lane(&t.id);
            let px = lane_x(self.lane(&t.parent));
            let cx = lane_x(lane);
            let py = node_y(pr);
            let cy = node_y(cr);
            let color = palette[lane % palette.len()];
            let stroke = Stroke::new(1.5, rgba(color, if t.done { 0.32 } else { 0.58 }));
            let dir = if cy >= py { 1.0 } else { -1.0 };
            let bend = cy - dir * 10.0;
            p.line_segment([Pos2::new(px, py), Pos2::new(px, bend)], stroke);
            p.line_segment([Pos2::new(px, bend), Pos2::new(cx, bend)], stroke);
            p.line_segment([Pos2::new(cx, bend), Pos2::new(cx, cy)], stroke);
            p.circle_filled(Pos2::new(cx, cy), 2.2, rgba(color, 0.95));
        }

        // rows
        let mut delete_id: Option<String> = None;
        let mut toggle_id: Option<String> = None;
        let mut due_id: Option<String> = None;
        let mut restore_id: Option<String> = None;
        let mut purge_id: Option<String> = None;
        let mut hover_del = false;
        for (t, r) in tasks.iter().zip(positions.iter().map(|(_, r)| *r)) {
            let row = r.shrink2(Vec2::new(0.0, 3.0));
            let resp = ui.interact(row, Id::new(("row", &t.id)), Sense::click());
            if resp.hovered() { hover_del = true; } // not used per row; hover handled below
            if !self.multi.is_empty() {
                let boxr = Rect::from_center_size(Pos2::new(row.left() + 6.0, row.top() + 20.0), Vec2::new(13.0, 13.0));
                let on = self.multi.contains(&t.id);
                ui.painter().rect_filled(boxr, CornerRadius::same(3), if on { rgba(self.theme.accent, 0.16) } else { Color32::TRANSPARENT });
                ui.painter().rect_stroke(boxr, CornerRadius::same(3), Stroke::new(1.0, rgba(self.theme.fg, if on { 0.6 } else { 0.3 })), egui::StrokeKind::Inside);
                if on {
                    ui.painter().text(boxr.center(), Align2::CENTER_CENTER, "✓", FontId::new(10.0, FontFamily::Monospace), self.theme.accent);
                }
            }
            if resp.clicked() {
                if ui.input(|i| i.modifiers.command) {
                    if self.multi.contains(&t.id) { self.multi.remove(&t.id); }
                    else { self.multi.insert(t.id.clone()); }
                } else {
                    self.selected = Some(t.id.clone());
                }
            }
            if resp.double_clicked() { self.edit = Some((t.id.clone(), t.title.clone(), t.notes.clone())); }
            let selected = self.selected.as_deref() == Some(t.id.as_str());
            if selected {
                ui.painter().rect_filled(row, CornerRadius::same(6), rgba(self.theme.accent, 0.05));
            } else if resp.hovered() {
                ui.painter().rect_filled(row, CornerRadius::same(6), rgba(self.theme.fg, 0.025));
            }
            // node
            let lane = self.lane(&t.id);
            let center = Pos2::new(lane_x(lane), node_y(r) - 4.0);
            let node = Rect::from_center_size(center, Vec2::new(26.0, 26.0));
            let nresp = ui.interact(node, Id::new(("node", &t.id)), Sense::click_and_drag());
            let p = ui.painter();
            let done_t = ui.ctx().animate_bool_with_time(Id::new(("done-anim", &t.id)), t.done, 0.2);
            p.circle_filled(center, 7.0, if t.done { self.theme.accent } else { rgba(self.theme.bg, 0.9) });
            p.circle_stroke(center, 7.0, Stroke::new(1.5, if nresp.hovered() { rgba(self.theme.fg, 0.8) } else { rgba(self.theme.fg, 0.45) }));
            if t.done || done_t > 0.02 {
                p.text(center, Align2::CENTER_CENTER, "✓", FontId::new(10.0, FontFamily::Monospace), rgba(self.theme.bg, done_t));
            }
            if nresp.clicked() { toggle_id = Some(t.id.clone()); }
            if nresp.drag_started() { self.link_from = Some(t.id.clone()); }
            // title + strike
            // clear the deepest lane (3 * 17) so nodes never sit on the title
            let title_x = content_rect.left() + 88.0;
            let title_clip = Rect::from_min_max(Pos2::new(title_x, row.top()), Pos2::new(row.right() - 120.0, row.top() + 30.0));
            let title_color = mix(self.theme.fg, self.theme.dim, if t.done { 0.55 } else { 0.0 });
            paint_text(p, Pos2::new(title_x, row.top() + 18.0), &t.title, 15.0, title_color, false, title_clip);
            let tw = text_width(p, &t.title, 15.0, false).min(title_clip.width());
            if done_t > 0.01 {
                let line = Rect::from_min_size(
                    Pos2::new(title_x, row.top() + 18.0),
                    Vec2::new(tw * done_t, 1.5),
                );
                p.rect_filled(line, CornerRadius::ZERO, rgba(self.theme.fg, 0.55));
            }
            // notes
            if !t.notes.is_empty() {
                let nc = Rect::from_min_max(Pos2::new(title_x, row.top() + 30.0), Pos2::new(row.right() - 20.0, row.bottom()));
                paint_text(p, Pos2::new(title_x, row.top() + 42.0), &t.notes.replace('\n', " "), 13.0, rgba(self.theme.fg, 0.45), false, nc);
            }
            // right side: age, due, buttons
            let mut rx = row.right() - 12.0;
            if self.filter == Filter::Trash {
                // restore / purge
                let purge_rect = Rect::from_center_size(Pos2::new(rx - 8.0, row.top() + 20.0), Vec2::new(22.0, 22.0));
                let rp = ui.interact(purge_rect, Id::new(("purge", &t.id)), Sense::click());
                p.text(purge_rect.center(), Align2::CENTER_CENTER, "✕", FontId::new(13.0, FontFamily::Monospace), self.theme.danger);
                if rp.clicked() { purge_id = Some(t.id.clone()); }
                rx -= 28.0;
                let rest_rect = Rect::from_center_size(Pos2::new(rx - 8.0, row.top() + 20.0), Vec2::new(22.0, 22.0));
                let rr = ui.interact(rest_rect, Id::new(("restore", &t.id)), Sense::click());
                p.text(rest_rect.center(), Align2::CENTER_CENTER, "↺", FontId::new(14.0, FontFamily::Monospace), self.theme.accent);
                if rr.clicked() { restore_id = Some(t.id.clone()); }
                rx -= 30.0;
            } else if resp.hovered() || selected {
                let handle = Rect::from_center_size(Pos2::new(rx - 8.0, row.top() + 20.0), Vec2::new(18.0, 22.0));
                let hr = ui.interact(handle, Id::new(("drag", &t.id)), Sense::click_and_drag());
                p.text(handle.center(), Align2::CENTER_CENTER, "⠿", FontId::new(11.0, FontFamily::Monospace), rgba(self.theme.fg, if hr.hovered() { 0.7 } else { 0.3 }));
                if hr.drag_started() { self.drag_id = Some(t.id.clone()); self.drag_y = hr.interact_pointer_pos().map(|p| p.y).unwrap_or(row.top()); }
                if hr.dragged() { self.drag_y = ui.ctx().input(|i| i.pointer.interact_pos()).map(|p| p.y).unwrap_or(self.drag_y); }
                if hr.drag_stopped() {
                    if let Some(did) = self.drag_id.clone() {
                        let target = positions.iter().position(|(_, rr)| self.drag_y < rr.center().y).unwrap_or(positions.len().saturating_sub(1));
                        self.move_to(&did, target);
                    }
                    self.drag_id = None; self.drag_y = -1.0;
                }
                rx -= 28.0;
                let del = Rect::from_center_size(Pos2::new(rx - 8.0, row.top() + 20.0), Vec2::new(22.0, 22.0));
                let dr = ui.interact(del, Id::new(("del", &t.id)), Sense::click());
                p.text(del.center(), Align2::CENTER_CENTER, "✕", FontId::new(12.0, FontFamily::Monospace), rgba(self.theme.danger, if dr.hovered() { 1.0 } else { 0.7 }));
                if dr.clicked() { delete_id = Some(t.id.clone()); }
                rx -= 28.0;
            }
            if !t.title.is_empty() && self.filter != Filter::Trash && (t.due > 0 || resp.hovered()) {
                let due_color = if t.due > 0 && t.due < now_ms() { self.theme.danger } else { rgba(self.theme.fg, 0.5) };
                let label = if t.due > 0 { due_label(t.due) } else { "⏰".into() };
                let w = text_width(p, &label, 12.0, false) + 10.0;
                let drect = Rect::from_center_size(Pos2::new(rx - w / 2.0, row.top() + 18.0), Vec2::new(w, 20.0));
                let dr = ui.interact(drect, Id::new(("due", &t.id)), Sense::click());
                p.rect_filled(drect, CornerRadius::same(4), rgba(if t.due > 0 { due_color } else { self.theme.fg }, if dr.hovered() { 0.12 } else { 0.05 }));
                p.text(drect.center(), Align2::CENTER_CENTER, label, FontId::new(12.0, FontFamily::Monospace), due_color);
                if dr.clicked() { due_id = Some(t.id.clone()); }
                rx -= w + 8.0;
            }
            let age_text = if t.deleted_at > 0 { age(t.deleted_at) } else { age(t.created) };
            p.text(Pos2::new(rx, row.top() + 18.0), Align2::RIGHT_CENTER, age_text, FontId::new(12.0, FontFamily::Monospace), rgba(self.theme.fg, 0.4));
        }

        if self.drag_id.is_some() && self.drag_y > 0.0 {
            let p = ui.painter();
            let y = self.drag_y.clamp(content_rect.top(), content_rect.bottom());
            p.rect_filled(Rect::from_min_size(Pos2::new(content_rect.left() + 88.0, y - 1.0), Vec2::new(width - 100.0, 2.0)), CornerRadius::same(1), rgba(self.theme.accent, 0.8));
        }

        // link drag line
        if let Some(from) = self.link_from.clone() {
            if let Some(r) = pos_of(&from) {
                if let Some(ptr) = ui.ctx().input(|i| i.pointer.interact_pos()) {
                    let start = Pos2::new(lane_x(self.lane(&from)), node_y(r));
                    let p = ui.painter();
                    let mut a = start;
                    let steps = 18;
                    for k in 1..=steps {
                        let t = k as f32 / steps as f32;
                        let b = start + (ptr - start) * t;
                        if k % 2 == 0 { p.line_segment([a, b], Stroke::new(1.5, self.theme.accent2)); }
                        a = b;
                    }
                    p.circle_filled(ptr, 3.0, self.theme.accent2);
                }
            }
        }
        // handle drops
        if let Some(id) = toggle_id { self.toggle(&id); }
        if let Some(id) = delete_id { self.soft_delete(&id); }
        if let Some(id) = restore_id { self.restore(&id); }
        if let Some(id) = purge_id { self.purge(&id); }
        if let Some(id) = due_id {
            let cur = self.task(&id).map(|t| t.due).unwrap_or(0);
            self.due_edit = Some((id, if cur > 0 { Local.timestamp_millis_opt(cur).single().map(|d| d.format("%Y-%m-%d %H:%M").to_string()).unwrap_or_default() } else { String::new() }));
        }
        if let Some(from) = self.link_from.clone() {
            if ui.ctx().input(|i| i.pointer.any_released()) {
                if let Some(ptr) = ui.ctx().input(|i| i.pointer.interact_pos()) {
                    if let Some((to, _)) = positions.iter().find(|(_, r)| r.contains(ptr)) {
                        if to != &from { self.link(&from, to); }
                    }
                }
                self.link_from = None;
            }
        }
        // empty state
        if tasks.is_empty() {
            let p = ui.painter();
            let c = content_rect.center();
            p.text(c, Align2::CENTER_CENTER, if self.filter == Filter::Trash { "Trash is empty" } else { "No tasks" }, FontId::new(14.0, FontFamily::Monospace), rgba(self.theme.fg, 0.6));
        }
    }

    fn ui_batch(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(format!("{} selected", self.multi.len())).size(13.0).color(self.theme.accent).strong());
            ui.add_space(8.0);
            if ui.button(egui::RichText::new("✓ Done").size(13.0).color(self.theme.fg)).clicked() {
                let ids: Vec<String> = self.multi.iter().cloned().collect(); self.set_done_many(&ids, true); self.multi.clear();
            }
            if ui.button(egui::RichText::new("↺ Reopen").size(13.0).color(self.theme.fg)).clicked() {
                let ids: Vec<String> = self.multi.iter().cloned().collect(); self.set_done_many(&ids, false); self.multi.clear();
            }
            if ui.button(egui::RichText::new("✕ Trash").size(13.0).color(self.theme.danger)).clicked() {
                let ids: Vec<String> = self.multi.iter().cloned().collect(); self.delete_many(&ids);
            }
            if ui.button(egui::RichText::new("Clear").size(13.0).color(self.theme.dim)).clicked() { self.multi.clear(); }
        });
        ui.add_space(4.0);
        ui.label(egui::RichText::new("ctrl+click to add · esc to clear").size(12.0).color(rgba(self.theme.fg, 0.4)));
    }

    fn ui_input(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        let (line, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), Sense::hover());
        ui.painter().rect_filled(line, CornerRadius::ZERO, rgba(self.theme.fg, 0.05));
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("❯").size(18.0).color(self.theme.accent).strong());
            let edit = egui::TextEdit::singleline(&mut self.input)
                .id(Id::new("task-input"))
                .hint_text("add task…    / to filter")
                .font(FontId::new(15.0, FontFamily::Monospace))
                .text_color(self.theme.fg)
                .frame(egui::Frame::NONE)
                .desired_width(f32::INFINITY);
            let resp = ui.add_sized([ui.available_width() - 4.0, 28.0], edit);
            let underline = Rect::from_min_size(Pos2::new(resp.rect.left(), resp.rect.bottom() + 2.0), Vec2::new(resp.rect.width(), 1.5));
            ui.painter().rect_filled(underline, CornerRadius::ZERO, rgba(self.theme.accent, if resp.has_focus() { 0.6 } else { 0.25 }));
            if self.first_frame {
                resp.request_focus();
                self.first_frame = false;
            }
            let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
            if enter && resp.has_focus() {
                if let Some(q) = self.input.strip_prefix('/') {
                    self.search = q.trim().to_string();
                    self.input.clear();
                } else if !self.input.trim().is_empty() {
                    let text = self.input.clone();
                    self.add(&text);
                    self.input.clear();
                }
            }
            if self.input.starts_with('/') {
                self.search = self.input.trim_start_matches('/').trim().to_string();
            }
        });
        ui.horizontal(|ui| {
            let hint = if self.search.is_empty() {
                "↵ add   ·   / filter   ·   ctrl+z undo   ·   esc hide".to_string()
            } else {
                format!("filter: {}   ·   esc to clear", self.search)
            };
            ui.label(egui::RichText::new(hint).size(12.0).color(rgba(self.theme.fg, 0.4)));
        });
    }

    fn ui_status_toast(&mut self, ui: &mut egui::Ui) {
        if let Some((msg, at)) = self.toast.clone() {
            if at.elapsed() > Duration::from_millis(2600) {
                self.toast = None;
            } else {
                let p = ui.painter();
                let rect = Rect::from_min_size(Pos2::new(ui.min_rect().left() + 20.0, ui.min_rect().bottom() - 54.0), Vec2::new(ui.available_width() - 40.0, 26.0));
                p.rect_filled(rect, CornerRadius::same(5), rgba(self.theme.surface, 0.96));
                p.text(Pos2::new(rect.left() + 10.0, rect.center().y), Align2::LEFT_CENTER, &msg, FontId::new(12.0, FontFamily::Monospace), rgba(self.theme.fg, 0.85));
            }
        }
    }

    fn ui_edit_window(&mut self, ctx: &egui::Context) {
        let Some((id, title, notes)) = self.edit.clone() else { return };
        let mut open = true;
        let mut title = title;
        let mut notes = notes;
        let mut save = false;
        let mut cancel = false;
        egui::Window::new("Edit task")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(egui::RichText::new("title").size(12.0).color(self.theme.dim));
                ui.add_sized([ui.available_width(), 30.0],
                    egui::TextEdit::singleline(&mut title).font(FontId::new(15.0, FontFamily::Monospace)));
                ui.add_space(4.0);
                ui.label(egui::RichText::new("notes").size(12.0).color(self.theme.dim));
                ui.add_sized([ui.available_width(), 80.0],
                    egui::TextEdit::multiline(&mut notes).font(FontId::new(13.0, FontFamily::Monospace)));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button(egui::RichText::new("Save").size(12.0).color(self.theme.accent)).clicked() { save = true; }
                    if ui.button(egui::RichText::new("Cancel").size(12.0).color(self.theme.dim)).clicked() { cancel = true; }
                });
            });
        if !open || cancel { self.edit = None; return; }
        if save {
            let clean = title.trim().to_string();
            if !clean.is_empty() {
                self.snapshot();
                if let Some(t) = self.task_mut(&id) {
                    t.title = clean;
                    t.notes = notes;
                }
                self.dirty = true;
            }
            self.edit = None;
        } else if let Some((_, t, n)) = self.edit.as_mut() {
            *t = title;
            *n = notes;
        }
    }

    fn ui_due_window(&mut self, ctx: &egui::Context) {
        let Some((id, draft)) = self.due_edit.clone() else { return };
        let mut open = true;
        let mut draft = draft;
        let mut apply: Option<Option<i64>> = None;
        egui::Window::new("Due date")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(egui::RichText::new(draft_task_title(&self.doc, &id)).size(12.0).color(self.theme.dim));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    for (label, val) in [("Today 18:00", "today 18:00"), ("Tomorrow 9:00", "tomorrow 09:00"), ("+1h", "+1h"), ("+3d", "+3d")] {
                        if ui.add(egui::Button::new(egui::RichText::new(label).size(12.0).color(self.theme.fg)).frame(true).fill(rgba(self.theme.surface, 0.9))).clicked() {
                            draft = val.to_string();
                        }
                    }
                });
                ui.add_space(6.0);
                let resp = ui.add_sized([ui.available_width(), 30.0], egui::TextEdit::singleline(&mut draft).hint_text("2026-10-27 18:00 · +2h"));
                ui.add_space(2.0);
                if draft.trim().is_empty() {
                    ui.label(egui::RichText::new("no due date").size(12.0).color(self.theme.dim));
                } else if let Some(ms) = parse_when(&draft) {
                    ui.label(egui::RichText::new(due_label(ms)).size(12.0).color(self.theme.accent));
                } else {
                    ui.label(egui::RichText::new("couldn't parse").size(12.0).color(self.theme.danger));
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button(egui::RichText::new("Clear").size(12.0).color(self.theme.danger)).clicked() {
                        apply = Some(None);
                    }
                    if ui.button(egui::RichText::new("Cancel").size(12.0).color(self.theme.dim)).clicked() {
                        self.due_edit = None;
                    }
                    if ui.button(egui::RichText::new("Save").size(12.0).color(self.theme.accent)).clicked() || (resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))) {
                        match parse_when(&draft) {
                            Some(ms) => apply = Some(Some(ms)),
                            None => {}
                        }
                    }
                });
            });
        if !open { self.due_edit = None; }
        if let Some(value) = apply {
            self.snapshot();
            let remind = match value {
                Some(ms) => (ms - 600_000).max(now_ms() + 5_000),
                None => 0,
            };
            if let Some(t) = self.task_mut(&id) {
                t.due = value.unwrap_or(0);
                t.remind_at = if value.is_some() { remind } else { 0 };
                t.notified = false;
            }
            self.dirty = true;
            self.due_edit = None;
        } else if self.due_edit.is_some() {
            // keep the edited text while the modal is open
            if let Some((_, d)) = self.due_edit.as_mut() { *d = draft; }
        }
    }
}

fn draft_task_title(doc: &Doc, id: &str) -> String {
    doc.items.iter().find(|t| t.id == id).map(|t| t.title.clone()).unwrap_or_default()
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.check_reminders();
        let ctx = ui.ctx().clone();
        let focused = ctx.input(|i| i.viewport().focused.unwrap_or(true));
        if focused { self.had_focus = true; }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            if !self.multi.is_empty() {
                self.multi.clear();
            } else if !self.search.is_empty() || !self.input.is_empty() {
                self.search.clear();
                self.input.clear();
            } else {
                self.save_now();
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::A)) {
            let ids: Vec<String> = self.visible().iter().map(|t| t.id.clone()).collect();
            self.multi = ids.into_iter().collect();
        }
        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Z)) {
            if ctx.input(|i| i.modifiers.shift) { self.redo(); } else { self.undo(); }
        }
        if self.dirty && self.last_save.elapsed() > Duration::from_millis(300) {
            self.save_now();
        }
        ctx.request_repaint_after(Duration::from_millis(300));

        let bg = self.theme.bg;
        let border = rgba(self.theme.fg, 0.07);
        let frame = egui::Frame::NONE
            .fill(bg)
            .stroke(Stroke::new(1.0, border))
            .corner_radius(CornerRadius::same(12))
            .inner_margin(egui::Margin::symmetric(22, 18));
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            self.ui_header(ui);
            self.ui_progress(ui);
            self.ui_filters(ui);
            let list_h = (ui.available_height() - 92.0).max(80.0);
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .max_height(list_h)
                .show(ui, |ui| {
                    self.ui_list(ui);
                });
            if self.multi.is_empty() { self.ui_input(ui); } else { self.ui_batch(ui); }
            self.ui_status_toast(ui);
            if let Some(id) = self.selected.clone() {
                if ui.input(|i| i.key_pressed(egui::Key::Delete)) {
                    self.soft_delete(&id);
                }
            }
        });
        self.ui_edit_window(&ctx);
        self.ui_due_window(&ctx);
        // keep the link interaction alive while the pointer moves
        if self.link_from.is_some() { ctx.request_repaint(); }
        if self.toast.is_some() { ctx.request_repaint_after(Duration::from_millis(200)); }
    }
}

fn initial_height() -> f32 {
    let doc = fs::read_to_string(state_path())
        .ok()
        .and_then(|raw| serde_json::from_str::<Doc>(&raw).ok())
        .unwrap_or_default();
    let content: f32 = doc.items.iter().filter(|t| t.deleted_at == 0)
        .map(|t| if t.notes.is_empty() { 54.0 } else { 70.0 }).sum();
    (223.0 + content).clamp(380.0, 940.0)
}

fn remind_only() {
    let path = state_path();
    let Ok(raw) = fs::read_to_string(&path) else { return };
    let Ok(mut doc) = serde_json::from_str::<Doc>(&raw) else { return };
    let now = now_ms();
    let mut fired = 0;
    for t in &mut doc.items {
        if t.deleted_at == 0 && !t.done && t.remind_at > 0 && !t.notified && t.remind_at <= now {
            let body = if t.due > 0 { format!("{} — {}", t.title, due_label(t.due)) } else { t.title.clone() };
            let _ = std::process::Command::new("notify-send")
                .args(["-a", "Tasks", "-u", "normal", "Task due", &body])
                .spawn();
            t.notified = true;
            fired += 1;
        }
    }
    if fired > 0 {
        if let Ok(text) = serde_json::to_string_pretty(&doc) {
            let tmp = path.with_extension("json.tmp");
            if fs::write(&tmp, text).is_ok() { let _ = fs::rename(&tmp, &path); }
        }
    }
}

fn main() -> eframe::Result<()> {
    unsafe { libc::signal(libc::SIGUSR1, on_sigusr1 as libc::sighandler_t); }
    let start_hidden = std::env::args().any(|a| a == "--hidden");
    if std::env::args().any(|a| a == "--remind") {
        remind_only();
        return Ok(());
    }
    let height = initial_height();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([PANEL_W, height])
            .with_min_inner_size([460.0, 420.0])
            .with_decorations(false)
            .with_transparent(true)
            .with_always_on_top()
            .with_resizable(true)
            .with_app_id("tasks-rs")
            .with_title("Tasks"),
        ..Default::default()
    };
    eframe::run_native(
        "Tasks",
        options,
        Box::new(|cc| {
            install_fonts(&cc.egui_ctx);
            let mut app = App::new();
            if start_hidden {
                app.hidden = true;
                cc.egui_ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            }
            Ok(Box::new(app))
        }),
    )
}
