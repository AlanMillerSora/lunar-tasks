use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use chrono::{Datelike, Local, TimeZone, Timelike};

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontFamily, FontId, Id, Layout, Pos2, Rect, Sense,
    Stroke, Vec2,
};

use crate::calendar;
use crate::hypr;
use crate::ipc;
use crate::model::{self, Doc, Settings, Task};
use crate::theme::{self, Theme, mix, rgba};
use crate::util::{age, due_label, now_ms, one_line, parse_quick, parse_when, truncate};

const TITLE_X: f32 = 104.0;
const LANE_X: f32 = 26.0;
const LANE_STEP: f32 = 15.0;
const NODE_Y: f32 = 21.0;
const DEFAULT_W: f32 = 660.0;
const DEFAULT_H: f32 = 780.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Filter {
    All,
    Open,
    Done,
    Trash,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SmartView {
    Inbox,
    Today,
    Upcoming,
    Overdue,
    NoDate,
}

impl SmartView {
    fn label(self) -> &'static str {
        match self {
            SmartView::Inbox => "Inbox",
            SmartView::Today => "Today",
            SmartView::Upcoming => "Upcoming",
            SmartView::Overdue => "Overdue",
            SmartView::NoDate => "No date",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SortMode {
    Manual,
    Due,
    Priority,
}

impl SortMode {
    fn label(self) -> &'static str {
        match self {
            SortMode::Manual => "Manual order",
            SortMode::Due => "Due date",
            SortMode::Priority => "Priority",
        }
    }
}

fn sort_mode_from_setting(s: Option<&str>) -> SortMode {
    match s.unwrap_or("manual") {
        "due" => SortMode::Due,
        "priority" => SortMode::Priority,
        _ => SortMode::Manual,
    }
}

struct LinkFlash {
    child: String,
    parent: String,
    at: Instant,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum InlineField {
    Title,
    Notes,
}

struct InlineEdit {
    id: String,
    title: String,
    notes: String,
    field: InlineField,
    focus: bool,
}

#[derive(Clone)]
enum PaletteAction {
    NewTask,
    SetView(SmartView),
    SetFilter(Filter),
    ToggleCompact,
    ToggleTags,
    ToggleFocus,
    SetSort(SortMode),
    EditTitle,
    EditNotes,
    SetDue,
    Stats,
    Help,
    ExportMarkdown,
    ExportJson,
    ImportJson,
    RestoreBackup,
    OpenDataFolder,
    ToggleReduceMotion,
    Hide,
    Quit,
    GoTo(String),
}

struct PaletteItem {
    label: String,
    hint: &'static str,
    action: PaletteAction,
}

#[derive(Clone)]
struct DueEdit {
    id: String,
    draft: String,
    month: (i32, u32),
    repeat: String,
    lead: i64,
}

pub struct App {
    doc: Doc,
    path: PathBuf,
    load_error: Option<String>,
    read_only: bool,
    theme: Theme,
    settings: Settings,
    filter: Filter,
    view: SmartView,
    sort: SortMode,
    collapsed_sections: HashSet<String>,
    tag_filter: Option<String>,
    input: String,
    input_parent: Option<String>,
    search: String,
    selected: Option<String>,
    scroll_to: Option<String>,
    undo: Vec<Vec<Task>>,
    redo: Vec<Vec<Task>>,
    dirty: bool,
    last_save: Instant,
    toast: Option<(String, Instant)>,
    link_from: Option<String>,
    link_drag: Option<String>,
    link_flash: Option<LinkFlash>,
    multi: HashSet<String>,
    drag_id: Option<String>,
    drag_y: f32,
    due_edit: Option<DueEdit>,
    due_anchor: Option<Pos2>,
    due_request: Option<String>,
    edit: Option<(String, String, String)>,
    inline: Option<InlineEdit>,
    edit_focus: bool,
    due_focus: bool,
    tag_rail_open: bool,
    kbd_nav: bool,
    done_flash: HashMap<String, Instant>,
    undo_snapshot: Option<(Vec<Task>, Instant)>,
    palette_open: bool,
    palette_query: String,
    palette_idx: usize,
    palette_focus: bool,
    focus_mode: bool,
    stats_open: bool,
    help_open: bool,
    motion_applied: bool,
    hidden: bool,
    first_frame: bool,
    focus_input: bool,
    quitting: bool,
    pending_pos: Option<(f32, f32)>,
    pos_applied: bool,
    opened_at: Instant,
    last_size: Vec2,
    last_geom_save: Instant,
    last_mtime: Option<SystemTime>,
    last_external_check: Instant,
    last_daemon_check: Instant,
    daemon_present: bool,
    last_reminder: Instant,
    start_hidden: bool,
}

pub fn run_gui(start_hidden: bool) -> eframe::Result<()> {
    let Some(listener) = ipc::claim_gui_socket() else {
        return Ok(());
    };
    let mut settings = model::load_settings();
    let w = settings.width.unwrap_or(DEFAULT_W).clamp(480.0, 900.0);
    let h = settings.height.unwrap_or(DEFAULT_H).clamp(420.0, 1300.0);
    let pos = settings.x.zip(settings.y).or_else(|| hypr::top_right(w));
    settings.width = Some(w);
    settings.height = Some(h);
    let pending_pos = pos;
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([w, h])
        .with_min_inner_size([480.0, 420.0])
        .with_visible(!start_hidden);
    if let Some((x, y)) = pos {
        viewport = viewport.with_position([x, y]);
    }
    let options = eframe::NativeOptions {
        viewport: viewport
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
        Box::new(move |cc| {
            theme::install_fonts(&cc.egui_ctx);
            ipc::start_gui_listener(listener, cc.egui_ctx.clone());
            let app = App::new(settings, start_hidden, pending_pos, w, h);
            Ok(Box::new(app))
        }),
    )
}

impl App {
    fn new(
        settings: Settings,
        start_hidden: bool,
        pending_pos: Option<(f32, f32)>,
        w: f32,
        h: f32,
    ) -> Self {
        let path = model::state_path();
        let mut load_error = None;
        let mut read_only = false;
        let doc = match model::load_doc(&path) {
            Ok(Some(doc)) => doc,
            Ok(None) => Doc::default(),
            Err(e) => {
                load_error = Some(e);
                read_only = true;
                Doc::default()
            }
        };
        let last_mtime = fs::metadata(&path).and_then(|m| m.modified()).ok();
        let tag_rail_open =
            !model::tag_counts(&doc).is_empty() && settings.width.unwrap_or(DEFAULT_W) >= 760.0;
        App {
            doc,
            path,
            load_error,
            read_only,
            theme: theme::load_theme(),
            sort: sort_mode_from_setting(settings.sort.as_deref()),
            settings,
            filter: Filter::All,
            view: SmartView::Inbox,
            collapsed_sections: HashSet::new(),
            tag_filter: None,
            input: String::new(),
            input_parent: None,
            search: String::new(),
            selected: None,
            scroll_to: None,
            undo: Vec::new(),
            redo: Vec::new(),
            dirty: false,
            last_save: Instant::now(),
            toast: None,
            link_from: None,
            link_drag: None,
            link_flash: None,
            multi: HashSet::new(),
            drag_id: None,
            drag_y: -1.0,
            due_edit: None,
            due_anchor: None,
            due_request: None,
            edit: None,
            inline: None,
            edit_focus: false,
            due_focus: false,
            tag_rail_open,
            kbd_nav: false,
            done_flash: HashMap::new(),
            undo_snapshot: None,
            palette_open: false,
            palette_query: String::new(),
            palette_idx: 0,
            palette_focus: false,
            focus_mode: false,
            stats_open: false,
            help_open: false,
            motion_applied: false,
            hidden: start_hidden,
            first_frame: true,
            focus_input: !start_hidden,
            quitting: false,
            pending_pos,
            pos_applied: false,
            opened_at: Instant::now(),
            last_size: Vec2::new(w, h),
            last_geom_save: Instant::now(),
            last_mtime,
            last_external_check: Instant::now(),
            last_daemon_check: Instant::now(),
            daemon_present: ipc::daemon_alive(),
            last_reminder: Instant::now(),
            start_hidden,
        }
    }

    fn save_now(&mut self) {
        if self.read_only {
            return;
        }
        // Reminders are marked by the daemon. Preserve those flags when the GUI
        // happens to hold an older copy of the document.
        match model::load_doc(&self.path) {
            Ok(Some(disk)) => {
                let notified: HashMap<&str, bool> = disk
                    .items
                    .iter()
                    .map(|t| (t.id.as_str(), t.notified))
                    .collect();
                for t in &mut self.doc.items {
                    if notified.get(t.id.as_str()).copied().unwrap_or(false) {
                        t.notified = true;
                    }
                }
            }
            Ok(None) => {}
            Err(e) => {
                self.save_error_toast(format!("Not overwriting unreadable state: {e}"));
                self.last_save = Instant::now();
                return;
            }
        }
        match model::save_doc(&self.path, &self.doc) {
            Ok(()) => {
                self.dirty = false;
                self.last_save = Instant::now();
                self.last_mtime = fs::metadata(&self.path).and_then(|m| m.modified()).ok();
                self.load_error = None;
            }
            Err(e) => {
                self.dirty = true;
                self.last_save = Instant::now();
                self.save_error_toast(e);
            }
        }
    }

    fn save_error_toast(&mut self, e: String) {
        self.toast = Some((truncate(&e, 90), Instant::now()));
    }

    fn save_if_dirty(&mut self) {
        if self.dirty {
            self.save_now();
        }
    }

    fn reload_from_disk(&mut self, quiet: bool) {
        match model::load_doc(&self.path) {
            Ok(Some(doc)) => {
                self.doc = doc;
                self.read_only = false;
                self.load_error = None;
                self.last_mtime = fs::metadata(&self.path).and_then(|m| m.modified()).ok();
                if !quiet {
                    self.toast("Reloaded from disk");
                }
            }
            Ok(None) => {
                self.doc = Doc::default();
                self.read_only = false;
                self.load_error = None;
            }
            Err(e) => {
                self.read_only = true;
                self.load_error = Some(e);
            }
        }
    }

    fn snapshot(&mut self) {
        self.undo.push(self.doc.items.clone());
        if self.undo.len() > 80 {
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
        self.undo_snapshot = None;
        self.toast = Some((msg.into(), Instant::now()));
    }

    fn toast_undo(&mut self, msg: impl Into<String>, items: Vec<Task>) {
        self.toast = Some((msg.into(), Instant::now()));
        self.undo_snapshot = Some((items, Instant::now()));
    }

    fn counts(&self) -> (usize, usize, usize) {
        let mut open = 0;
        let mut done = 0;
        let mut trash = 0;
        for t in &self.doc.items {
            if t.deleted_at > 0 {
                trash += 1;
            } else if t.done {
                done += 1;
            } else {
                open += 1;
            }
        }
        (open, done, trash)
    }

    fn overdue_count(&self) -> usize {
        let now = now_ms();
        self.doc
            .items
            .iter()
            .filter(|t| t.deleted_at == 0 && !t.done && t.due > 0 && t.due < now)
            .count()
    }

    fn task(&self, id: &str) -> Option<&Task> {
        self.doc.items.iter().find(|t| t.id == id)
    }

    fn task_mut(&mut self, id: &str) -> Option<&mut Task> {
        self.doc.items.iter_mut().find(|t| t.id == id)
    }

    fn add(&mut self, title: &str, parent: &str) {
        let title = title.trim();
        if title.is_empty() {
            return;
        }
        self.snapshot();
        let task = model::new_task(title, parent);
        let id = task.id.clone();
        if !parent.is_empty() {
            if let Some(pi) = self.doc.items.iter().position(|t| t.id == parent) {
                self.doc.items.insert(pi + 1, task);
                if let Some(p) = self.task_mut(parent) {
                    p.expanded = true;
                }
            } else {
                self.doc.items.insert(0, task);
            }
        } else {
            self.doc.items.insert(0, task);
        }
        self.selected = Some(id.clone());
        self.filter = Filter::All;
        self.dirty = true;
        self.toast(if parent.is_empty() {
            "Task added".to_string()
        } else {
            "Subtask added".to_string()
        });
        self.scroll_to = Some(id);
    }

    fn add_parsed(&mut self, title: &str, parent: &str, priority: u8, due: Option<i64>) {
        self.add(title, parent);
        let Some(id) = self.selected.clone() else {
            return;
        };
        if priority > 0 {
            if let Some(t) = self.task_mut(&id) {
                t.priority = priority;
            }
        }
        if let Some(due) = due {
            let lead = 600_000;
            if let Some(t) = self.task_mut(&id) {
                t.due = due;
                t.remind_lead = lead;
                t.remind_at = (due - lead).max(now_ms() + 5_000);
                t.notified = false;
            }
            self.toast(format!("Task added  ·  due {}", due_label(due)));
        }
        self.dirty = true;
    }

    fn toggle(&mut self, id: &str) {
        self.snapshot();
        let now_done = self.task(id).map(|t| !t.done).unwrap_or(false);
        if let Some(t) = self.task_mut(id) {
            t.done = !t.done;
            if t.done {
                t.expanded = false;
                t.completed_at = now_ms();
            } else {
                t.completed_at = 0;
            }
        }
        if now_done {
            if !self.settings.reduce_motion.unwrap_or(false) {
                self.done_flash.insert(id.to_string(), Instant::now());
            }
            self.spawn_repeat(id);
        }
        self.dirty = true;
    }

    fn spawn_repeat(&mut self, id: &str) {
        let Some(src) = self.task(id).cloned() else {
            return;
        };
        if src.repeat.is_empty() || src.due <= 0 {
            return;
        }
        let Some(next) = model::next_due(src.due, &src.repeat) else {
            return;
        };
        let lead = model::task_lead(&src);
        let mut copy = model::new_task(&src.title, &src.parent);
        copy.notes = src.notes;
        copy.priority = src.priority;
        copy.repeat = src.repeat.clone();
        copy.remind_lead = src.remind_lead;
        copy.due = next;
        copy.remind_at = (next - lead).max(now_ms() + 5_000);
        copy.expanded = false;
        let at = self
            .doc
            .items
            .iter()
            .position(|t| t.id == id)
            .map(|p| p + 1)
            .unwrap_or(0);
        self.doc.items.insert(at, copy);
        if let Some(t) = self.task_mut(id) {
            t.repeat.clear();
        }
        self.dirty = true;
        self.toast(format!("Next {} occurrence added", src.repeat));
    }

    fn duplicate(&mut self, id: &str) {
        let Some(src) = self.task(id).cloned() else {
            return;
        };
        self.snapshot();
        let mut copy = model::new_task(&format!("{} copy", src.title), &src.parent);
        copy.notes = src.notes;
        copy.due = src.due;
        copy.remind_at = src.remind_at;
        copy.expanded = src.expanded;
        copy.priority = src.priority;
        copy.repeat = src.repeat;
        copy.remind_lead = src.remind_lead;
        let new_id = copy.id.clone();
        let at = self
            .doc
            .items
            .iter()
            .position(|t| t.id == id)
            .map(|p| p + 1)
            .unwrap_or(0);
        self.doc.items.insert(at, copy);
        self.selected = Some(new_id.clone());
        self.scroll_to = Some(new_id);
        self.dirty = true;
        self.toast("Duplicated");
    }

    fn duplicate_with_subtasks(&mut self, id: &str) {
        let ids = self.subtree_ids(id);
        if ids.is_empty() {
            return;
        }
        self.snapshot();
        let source_index = self.doc.items.iter().position(|t| t.id == id).unwrap_or(0);
        let mut mapping: HashMap<String, String> = HashMap::new();
        let mut copies: Vec<Task> = Vec::new();
        for (i, src_id) in ids.iter().enumerate() {
            let Some(src) = self.task(src_id).cloned() else {
                continue;
            };
            let title = if i == 0 {
                format!("{} copy", src.title)
            } else {
                src.title.clone()
            };
            let mut copy = model::new_task(&title, "");
            let lead = model::task_lead(&src);
            copy.parent = mapping
                .get(&src.parent)
                .cloned()
                .unwrap_or_else(|| src.parent.clone());
            copy.notes = src.notes;
            copy.expanded = src.expanded;
            copy.due = src.due;
            copy.priority = src.priority;
            copy.repeat = src.repeat;
            copy.remind_lead = src.remind_lead;
            copy.remind_at = if src.due > 0 {
                (src.due - lead).max(now_ms() + 5_000)
            } else {
                0
            };
            mapping.insert(src.id.clone(), copy.id.clone());
            copies.push(copy);
        }
        let Some(new_root) = copies.first().map(|t| t.id.clone()) else {
            return;
        };
        for (i, copy) in copies.into_iter().enumerate() {
            self.doc.items.insert(source_index + 1 + i, copy);
        }
        self.selected = Some(new_root.clone());
        self.scroll_to = Some(new_root);
        self.dirty = true;
        self.toast("Duplicated with subtasks");
    }

    fn set_priority(&mut self, id: &str, priority: u8) {
        self.snapshot();
        if let Some(t) = self.task_mut(id) {
            t.priority = priority;
        }
        self.dirty = true;
    }

    fn start_inline(&mut self, id: &str, field: InlineField) {
        let Some(t) = self.task(id) else { return };
        self.inline = Some(InlineEdit {
            id: id.to_string(),
            title: t.title.clone(),
            notes: t.notes.clone(),
            field,
            focus: true,
        });
        self.scroll_to = Some(id.to_string());
    }

    fn soft_delete(&mut self, id: &str) {
        let before = self.doc.items.clone();
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
        if self.selected.as_deref() == Some(id) {
            self.selected = None;
        }
        self.dirty = true;
        self.toast_undo(
            format!("Moved {} task{} to trash", n, if n == 1 { "" } else { "s" }),
            before,
        );
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

    fn subtree_ids(&self, id: &str) -> Vec<String> {
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
        ids
    }

    fn lineage_ids(&self, id: &str) -> HashSet<String> {
        let mut set: HashSet<String> = HashSet::new();
        let mut cur = id.to_string();
        let mut guard = self.doc.items.len() + 2;
        while guard > 0 && !cur.is_empty() {
            guard -= 1;
            set.insert(cur.clone());
            match self.task(&cur) {
                Some(t) => cur = t.parent.clone(),
                None => break,
            }
        }
        for child in self.subtree_ids(id) {
            set.insert(child);
        }
        set
    }

    fn purge(&mut self, id: &str) {
        let before = self.doc.items.clone();
        let ids = self.subtree_ids(id);
        self.snapshot();
        self.doc.items.retain(|t| !ids.contains(&t.id));
        self.dirty = true;
        self.toast_undo("Deleted forever", before);
    }

    fn restore_many(&mut self, ids: &[String]) {
        if ids.is_empty() {
            return;
        }
        self.snapshot();
        for t in &mut self.doc.items {
            if ids.contains(&t.id) {
                t.deleted_at = 0;
            }
        }
        self.multi.clear();
        self.dirty = true;
        self.toast(format!("Restored {} task(s)", ids.len()));
    }

    fn purge_many(&mut self, ids: &[String]) {
        if ids.is_empty() {
            return;
        }
        let mut all = Vec::new();
        for id in ids {
            for child in self.subtree_ids(id) {
                if !all.contains(&child) {
                    all.push(child);
                }
            }
        }
        let before = self.doc.items.clone();
        self.snapshot();
        self.doc.items.retain(|t| !all.contains(&t.id));
        for t in &mut self.doc.items {
            if all.contains(&t.parent) {
                t.parent.clear();
            }
        }
        self.multi.clear();
        self.dirty = true;
        self.toast_undo(format!("Deleted {} task(s) forever", ids.len()), before);
    }

    fn set_done_many(&mut self, ids: &[String], done: bool) {
        if ids.is_empty() {
            return;
        }
        let before = self.doc.items.clone();
        self.snapshot();
        let mut n = 0;
        let mut newly_done = Vec::new();
        let stamp = now_ms();
        for t in &mut self.doc.items {
            if ids.contains(&t.id) && t.deleted_at == 0 && t.done != done {
                t.done = done;
                t.completed_at = if done { stamp } else { 0 };
                if done {
                    t.expanded = false;
                    newly_done.push(t.id.clone());
                }
                n += 1;
            }
        }
        let recur = newly_done.clone();
        if !self.settings.reduce_motion.unwrap_or(false) {
            for id in newly_done {
                self.done_flash.insert(id, Instant::now());
            }
        }
        for id in recur {
            self.spawn_repeat(&id);
        }
        if n > 0 {
            self.dirty = true;
            self.toast_undo(
                format!(
                    "{} {} task{}",
                    if done { "Completed" } else { "Reopened" },
                    n,
                    if n == 1 { "" } else { "s" }
                ),
                before,
            );
        }
    }

    fn delete_many(&mut self, ids: &[String]) {
        if ids.is_empty() {
            return;
        }
        let before = self.doc.items.clone();
        self.snapshot();
        let stamp = now_ms();
        let mut all: Vec<String> = ids.to_vec();
        let mut i = 0;
        while i < all.len() {
            let cur = all[i].clone();
            for t in &self.doc.items {
                if t.parent == cur && !all.contains(&t.id) {
                    all.push(t.id.clone());
                }
            }
            i += 1;
        }
        let mut n = 0;
        for t in &mut self.doc.items {
            if all.contains(&t.id) && t.deleted_at == 0 {
                t.deleted_at = stamp;
                n += 1;
            }
        }
        self.selected = None;
        self.multi.clear();
        if n > 0 {
            self.dirty = true;
            self.toast_undo(
                format!("Moved {} task{} to trash", n, if n == 1 { "" } else { "s" }),
                before,
            );
        }
    }

    fn move_relative(&mut self, id: &str, target: &str, after: bool) {
        if id == target {
            return;
        }
        let Some(cur) = self.doc.items.iter().position(|t| t.id == id) else {
            return;
        };
        if !self.doc.items.iter().any(|t| t.id == target) {
            return;
        }
        self.snapshot();
        let item = self.doc.items.remove(cur);
        let Some(mut at) = self.doc.items.iter().position(|t| t.id == target) else {
            self.doc.items.insert(0, item);
            self.dirty = true;
            return;
        };
        if after {
            at += 1;
        }
        let at = at.min(self.doc.items.len());
        self.doc.items.insert(at, item);
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
        self.dirty = true;
        if !self.settings.reduce_motion.unwrap_or(false) {
            self.link_flash = Some(LinkFlash {
                child: child.to_string(),
                parent: parent.to_string(),
                at: Instant::now(),
            });
        }
        let ct = self
            .task(child)
            .map(|t| t.title.clone())
            .unwrap_or_default();
        let pt = self
            .task(parent)
            .map(|t| t.title.clone())
            .unwrap_or_default();
        self.toast(format!(
            "Linked «{}» under «{}»",
            truncate(&ct, 22),
            truncate(&pt, 22)
        ));
        true
    }

    fn unlink(&mut self, id: &str) {
        let Some(t) = self.task_mut(id) else { return };
        if t.parent.is_empty() {
            return;
        }
        self.snapshot();
        if let Some(t) = self.task_mut(id) {
            t.parent.clear();
        }
        self.dirty = true;
        self.toast("Unlinked");
    }

    fn is_ancestor(&self, ancestor: &str, node: &str) -> bool {
        let mut cur = node.to_string();
        let mut guard = self.doc.items.len() + 2;
        while !cur.is_empty() && guard > 0 {
            guard -= 1;
            match self.task(&cur) {
                Some(t) => {
                    if t.parent == ancestor {
                        return true;
                    }
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

    fn today_start(&self) -> i64 {
        let today = Local::now().date_naive();
        Local
            .with_ymd_and_hms(today.year(), today.month(), today.day(), 0, 0, 0)
            .single()
            .map(|d| d.timestamp_millis())
            .unwrap_or_else(now_ms)
    }

    fn view_matches(&self, t: &Task) -> bool {
        if t.deleted_at > 0 {
            return false;
        }
        let start = self.today_start();
        let tomorrow = start + 86_400_000;
        let week_end = start + 8 * 86_400_000;
        match self.view {
            SmartView::Inbox => true,
            SmartView::Today => t.due > 0 && t.due < tomorrow,
            SmartView::Upcoming => t.due >= tomorrow && t.due < week_end,
            SmartView::Overdue => t.due > 0 && t.due < now_ms(),
            SmartView::NoDate => t.due == 0,
        }
    }

    fn due_bucket(&self, t: &Task) -> Option<&'static str> {
        let start = self.today_start();
        let tomorrow = start + 86_400_000;
        match self.view {
            SmartView::Inbox => None,
            SmartView::Today => {
                if t.due < now_ms() {
                    Some("Overdue")
                } else {
                    Some("Today")
                }
            }
            SmartView::Upcoming => {
                if t.due < tomorrow {
                    Some("Tomorrow")
                } else {
                    Some("This week")
                }
            }
            SmartView::Overdue => Some("Overdue"),
            SmartView::NoDate => Some("No due date"),
        }
    }

    fn view_count(&self, view: SmartView) -> usize {
        self.doc
            .items
            .iter()
            .filter(|t| !t.done && !t.deleted_at.gt(&0) && self.view_matches_for(t, view))
            .count()
    }

    fn section_count(&self, section: &str) -> usize {
        self.doc
            .items
            .iter()
            .filter(|t| {
                !t.deleted_at.gt(&0) && self.view_matches(t) && self.due_bucket(t) == Some(section)
            })
            .count()
    }

    fn view_matches_for(&self, t: &Task, view: SmartView) -> bool {
        let start = self.today_start();
        let tomorrow = start + 86_400_000;
        let week_end = start + 8 * 86_400_000;
        match view {
            SmartView::Inbox => true,
            SmartView::Today => t.due > 0 && t.due < tomorrow,
            SmartView::Upcoming => t.due >= tomorrow && t.due < week_end,
            SmartView::Overdue => t.due > 0 && t.due < now_ms(),
            SmartView::NoDate => t.due == 0,
        }
    }

    fn visible(&self) -> Vec<&Task> {
        let mut out: Vec<&Task> = self
            .doc
            .items
            .iter()
            .filter(|t| {
                if self.filter == Filter::Trash {
                    if t.deleted_at == 0 {
                        return false;
                    }
                } else {
                    if t.deleted_at > 0 {
                        return false;
                    }
                    if self.view != SmartView::Inbox && !self.view_matches(t) {
                        return false;
                    }
                    if self.view != SmartView::Inbox {
                        if let Some(section) = self.due_bucket(t) {
                            if self.collapsed_sections.contains(section) {
                                return false;
                            }
                        }
                    }
                    match self.filter {
                        Filter::All => {}
                        Filter::Open => {
                            if t.done {
                                return false;
                            }
                        }
                        Filter::Done => {
                            if !t.done {
                                return false;
                            }
                        }
                        Filter::Trash => {}
                    }
                }
                model::task_matches(t, &self.search, self.tag_filter.as_deref())
            })
            .collect();

        if self.filter == Filter::Trash {
            return out;
        }
        if self.view != SmartView::Inbox {
            out.sort_by(|a, b| {
                let ra = self.section_rank(self.due_bucket(a));
                let rb = self.section_rank(self.due_bucket(b));
                ra.cmp(&rb).then_with(|| self.compare_tasks(a, b))
            });
        } else {
            out.sort_by(|a, b| self.compare_tasks(a, b));
        }
        out
    }

    fn section_rank(&self, section: Option<&str>) -> u8 {
        match section {
            Some("Overdue") => 0,
            Some("Today") => 1,
            Some("Tomorrow") => 0,
            Some("This week") => 1,
            _ => 0,
        }
    }

    fn compare_tasks(&self, a: &Task, b: &Task) -> std::cmp::Ordering {
        match self.sort {
            SortMode::Manual => std::cmp::Ordering::Equal,
            SortMode::Due => a.due.cmp(&b.due).then_with(|| a.title.cmp(&b.title)),
            SortMode::Priority => {
                let pa = if a.priority == 0 { 9 } else { a.priority };
                let pb = if b.priority == 0 { 9 } else { b.priority };
                pa.cmp(&pb)
                    .then_with(|| a.due.cmp(&b.due))
                    .then_with(|| a.title.cmp(&b.title))
            }
        }
    }

    fn jump_to(&mut self, id: &str) {
        if let Some(t) = self.task(id) {
            if t.deleted_at > 0 {
                self.filter = Filter::Trash;
            } else {
                self.filter = Filter::All;
            }
            self.search.clear();
            self.tag_filter = None;
            self.view = SmartView::Inbox;
            self.collapsed_sections.clear();
            self.selected = Some(id.to_string());
            self.scroll_to = Some(id.to_string());
        }
    }

    fn set_tag_filter(&mut self, tag: &str) {
        let same = self
            .tag_filter
            .as_deref()
            .map(|t| t.eq_ignore_ascii_case(tag))
            .unwrap_or(false);
        self.tag_filter = if same { None } else { Some(tag.to_string()) };
        if self.filter == Filter::Trash {
            self.filter = Filter::All;
        }
    }

    fn hide_window(&mut self, ctx: &egui::Context) {
        self.save_geometry(ctx);
        self.save_if_dirty();
        // Wayland compositors do not reliably honour Visible(false). Closing
        // the window process is deterministic and the tray daemon stays alive,
        // so Show/toggle simply spawns the GUI again.
        self.hidden = true;
        self.quitting = true;
        ipc::remove_gui_socket();
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    fn show_window(&mut self, ctx: &egui::Context) {
        self.hidden = false;
        self.focus_input = true;
        self.first_frame = true;
        self.pending_pos = self
            .settings
            .x
            .zip(self.settings.y)
            .or_else(|| hypr::top_right(self.last_size.x.max(480.0)));
        self.pos_applied = false;
        self.opened_at = Instant::now();
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        let _ = hypr::focus_client();
        ctx.request_repaint();
    }

    fn quit(&mut self, ctx: &egui::Context) {
        self.save_geometry(ctx);
        self.save_if_dirty();
        self.quitting = true;
        ipc::remove_gui_socket();
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    fn save_geometry(&mut self, ctx: &egui::Context) {
        let mut changed = false;
        if let Some(geo) = hypr::client_geometry() {
            if geo.w >= 420.0 && geo.h >= 300.0 {
                if self
                    .settings
                    .width
                    .map(|w| (w - geo.w).abs() > 1.0)
                    .unwrap_or(true)
                {
                    self.settings.width = Some(geo.w);
                    changed = true;
                }
                if self
                    .settings
                    .height
                    .map(|h| (h - geo.h).abs() > 1.0)
                    .unwrap_or(true)
                {
                    self.settings.height = Some(geo.h);
                    changed = true;
                }
            }
            if geo.x > -20_000.0 && geo.y > -20_000.0 {
                if self
                    .settings
                    .x
                    .map(|x| (x - geo.x).abs() > 1.0)
                    .unwrap_or(true)
                {
                    self.settings.x = Some(geo.x);
                    changed = true;
                }
                if self
                    .settings
                    .y
                    .map(|y| (y - geo.y).abs() > 1.0)
                    .unwrap_or(true)
                {
                    self.settings.y = Some(geo.y);
                    changed = true;
                }
            }
        } else if let Some(rect) = ctx.input(|i| i.viewport().outer_rect) {
            if self
                .settings
                .x
                .map(|x| (x - rect.min.x).abs() > 1.0)
                .unwrap_or(true)
            {
                self.settings.x = Some(rect.min.x);
                changed = true;
            }
            if self
                .settings
                .y
                .map(|y| (y - rect.min.y).abs() > 1.0)
                .unwrap_or(true)
            {
                self.settings.y = Some(rect.min.y);
                changed = true;
            }
            if self
                .settings
                .width
                .map(|w| (w - rect.width()).abs() > 1.0)
                .unwrap_or(true)
            {
                self.settings.width = Some(rect.width());
                changed = true;
            }
            if self
                .settings
                .height
                .map(|h| (h - rect.height()).abs() > 1.0)
                .unwrap_or(true)
            {
                self.settings.height = Some(rect.height());
                changed = true;
            }
        } else {
            let size = ctx.content_rect().size();
            if (size.x - self.last_size.x).abs() > 1.0 || (size.y - self.last_size.y).abs() > 1.0 {
                self.settings.width = Some(size.x);
                self.settings.height = Some(size.y);
                changed = true;
            }
        }
        if changed {
            model::save_settings(&self.settings);
            self.last_geom_save = Instant::now();
        }
    }

    fn apply_position(&mut self, ctx: &egui::Context) {
        let Some((x, y)) = self.pending_pos else {
            return;
        };
        if hypr::move_client(x, y) {
            self.settings.x = Some(x);
            self.settings.y = Some(y);
        } else {
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(Pos2::new(x, y)));
        }
        self.pos_applied = true;
    }

    fn check_reminders(&mut self) {
        if self.last_reminder.elapsed() < Duration::from_secs(20) {
            return;
        }
        self.last_reminder = Instant::now();
        let now = now_ms();
        let mut fired: Vec<(String, String, i64)> = Vec::new();
        for t in &self.doc.items {
            if t.deleted_at == 0 && !t.done && t.remind_at > 0 && !t.notified && t.remind_at <= now
            {
                fired.push((t.id.clone(), t.title.clone(), t.due));
            }
        }
        if fired.is_empty() {
            return;
        }
        for (id, title, due) in &fired {
            let body = if *due > 0 {
                format!("{} — {}", title, due_label(*due))
            } else {
                title.clone()
            };
            crate::util::notify("Task due", &body);
            if let Some(t) = self.task_mut(id) {
                t.notified = true;
            }
        }
        self.dirty = true;
        self.toast(format!("Reminder: {}", truncate(&fired[0].1, 30)));
    }

    fn handle_commands(&mut self, ctx: &egui::Context) {
        for cmd in ipc::take_gui_cmds() {
            if cmd == "show" {
                self.show_window(ctx);
            } else if cmd == "hide" {
                self.hide_window(ctx);
            } else if cmd == "toggle" {
                if self.hidden {
                    self.show_window(ctx);
                } else {
                    self.hide_window(ctx);
                }
            } else if cmd == "add-prompt" {
                self.show_window(ctx);
                self.multi.clear();
                self.input_parent = None;
                self.focus_input = true;
            } else if cmd == "quit" {
                self.quit(ctx);
            } else if cmd == "reload" {
                if self.dirty {
                    self.save_now();
                }
                self.reload_from_disk(true);
            } else if let Some(text) = cmd.strip_prefix("add ") {
                self.add(text, "");
                self.show_window(ctx);
                self.save_now();
            } else if let Some(name) = cmd.strip_prefix("filter ") {
                self.filter = match name.trim() {
                    "open" => Filter::Open,
                    "done" => Filter::Done,
                    "trash" => Filter::Trash,
                    _ => Filter::All,
                };
                self.multi.clear();
            } else if let Some(query) = cmd.strip_prefix("search ") {
                self.search = query.trim().to_string();
                if self.search.is_empty() {
                    self.search.clear();
                }
            }
        }
    }

    fn maybe_external_reload(&mut self) {
        if self.hidden || self.dirty || self.read_only {
            return;
        }
        if self.last_external_check.elapsed() < Duration::from_secs(4) {
            return;
        }
        self.last_external_check = Instant::now();
        let Ok(meta) = fs::metadata(&self.path) else {
            return;
        };
        let Ok(mtime) = meta.modified() else { return };
        if self.last_mtime.map(|m| mtime > m).unwrap_or(false) {
            self.reload_from_disk(true);
        }
    }

    fn export_markdown(&mut self) {
        let dir = self
            .path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        let out = dir.join("export.md");
        let text = crate::stats::doc_to_markdown(&self.doc);
        match fs::write(&out, text) {
            Ok(()) => self.toast(format!("Exported {}", out.display())),
            Err(e) => self.toast(format!("Export failed: {e}")),
        }
    }

    fn export_json(&mut self) {
        let dir = self
            .path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        let out = dir.join("export.json");
        match serde_json::to_string_pretty(&self.doc) {
            Ok(text) => match fs::write(&out, text) {
                Ok(()) => self.toast(format!("Exported {}", out.display())),
                Err(e) => self.toast(format!("Export failed: {e}")),
            },
            Err(e) => self.toast(format!("Export failed: {e}")),
        }
    }

    fn import_export(&mut self) {
        let dir = self
            .path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        let src = dir.join("export.json");
        if !src.exists() {
            self.toast("No export.json found");
            return;
        }
        match model::load_doc(&src) {
            Ok(Some(doc)) => {
                let _ = fs::copy(&self.path, self.path.with_extension("before-import.json"));
                self.snapshot();
                self.doc = doc;
                self.filter = Filter::All;
                self.view = SmartView::Inbox;
                self.multi.clear();
                self.selected = None;
                self.dirty = true;
                self.save_now();
                self.toast("Imported export.json");
            }
            Ok(None) => self.toast("export.json is empty"),
            Err(e) => self.toast(format!("Import failed: {e}")),
        }
    }

    fn restore_backup(&mut self) {
        let bak = self.path.with_extension("json.bak");
        if !bak.exists() {
            self.toast("No state.json.bak found");
            return;
        }
        match model::load_doc(&bak) {
            Ok(Some(doc)) => {
                let _ = fs::copy(&self.path, self.path.with_extension("before-restore.json"));
                self.snapshot();
                self.doc = doc;
                self.filter = Filter::All;
                self.view = SmartView::Inbox;
                self.multi.clear();
                self.selected = None;
                self.dirty = true;
                self.save_now();
                self.toast("Restored from state.json.bak");
            }
            Ok(None) => self.toast("Backup is empty"),
            Err(e) => self.toast(format!("Backup unreadable: {e}")),
        }
    }

    fn open_data_folder(&mut self) {
        let dir = self
            .path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        let _ = std::process::Command::new("xdg-open").arg(&dir).spawn();
    }

    fn palette_items(&self) -> Vec<PaletteItem> {
        let mut out = vec![
            PaletteItem {
                label: "New task".into(),
                hint: "capture",
                action: PaletteAction::NewTask,
            },
            PaletteItem {
                label: "Toggle compact rows".into(),
                hint: "layout",
                action: PaletteAction::ToggleCompact,
            },
            PaletteItem {
                label: "Toggle focus mode".into(),
                hint: "single task",
                action: PaletteAction::ToggleFocus,
            },
            PaletteItem {
                label: "Toggle tag rail".into(),
                hint: "navigation",
                action: PaletteAction::ToggleTags,
            },
            PaletteItem {
                label: "Statistics & activity".into(),
                hint: "heatmap",
                action: PaletteAction::Stats,
            },
            PaletteItem {
                label: "Keyboard shortcuts".into(),
                hint: "help",
                action: PaletteAction::Help,
            },
            PaletteItem {
                label: "Export Markdown".into(),
                hint: "data",
                action: PaletteAction::ExportMarkdown,
            },
            PaletteItem {
                label: "Export JSON".into(),
                hint: "data",
                action: PaletteAction::ExportJson,
            },
            PaletteItem {
                label: "Import export.json".into(),
                hint: "data",
                action: PaletteAction::ImportJson,
            },
            PaletteItem {
                label: "Restore last backup".into(),
                hint: "state.json.bak",
                action: PaletteAction::RestoreBackup,
            },
            PaletteItem {
                label: "Open data folder".into(),
                hint: "files",
                action: PaletteAction::OpenDataFolder,
            },
            PaletteItem {
                label: "Toggle reduced motion".into(),
                hint: "accessibility",
                action: PaletteAction::ToggleReduceMotion,
            },
            PaletteItem {
                label: "Hide window".into(),
                hint: "Esc",
                action: PaletteAction::Hide,
            },
            PaletteItem {
                label: "Quit tasks".into(),
                hint: "tray",
                action: PaletteAction::Quit,
            },
        ];
        for (view, count) in [
            (SmartView::Inbox, self.view_count(SmartView::Inbox)),
            (SmartView::Today, self.view_count(SmartView::Today)),
            (SmartView::Upcoming, self.view_count(SmartView::Upcoming)),
            (SmartView::Overdue, self.view_count(SmartView::Overdue)),
            (SmartView::NoDate, self.view_count(SmartView::NoDate)),
        ] {
            out.push(PaletteItem {
                label: format!("View: {}  ({count})", view.label()),
                hint: "smart view",
                action: PaletteAction::SetView(view),
            });
        }
        for (filter, label) in [
            (Filter::All, "All"),
            (Filter::Open, "Open"),
            (Filter::Done, "Done"),
            (Filter::Trash, "Trash"),
        ] {
            out.push(PaletteItem {
                label: format!("Filter: {label}"),
                hint: "filter",
                action: PaletteAction::SetFilter(filter),
            });
        }
        for mode in [SortMode::Manual, SortMode::Due, SortMode::Priority] {
            out.push(PaletteItem {
                label: format!("Sort: {}", mode.label()),
                hint: "sort",
                action: PaletteAction::SetSort(mode),
            });
        }
        if self.selected.is_some() {
            out.push(PaletteItem {
                label: "Edit selected title".into(),
                hint: "Enter",
                action: PaletteAction::EditTitle,
            });
            out.push(PaletteItem {
                label: "Edit selected notes".into(),
                hint: "notes",
                action: PaletteAction::EditNotes,
            });
            out.push(PaletteItem {
                label: "Set due date for selected".into(),
                hint: "D",
                action: PaletteAction::SetDue,
            });
        }
        let q = self.palette_query.trim().to_lowercase();
        if !q.is_empty() {
            out.retain(|item| {
                let label = item.label.to_lowercase();
                label.contains(&q) || model::subsequence(&label, &q)
            });
            let mut n = 0;
            for t in &self.doc.items {
                if t.deleted_at > 0 || n >= 6 {
                    continue;
                }
                if model::task_matches(t, &q, None) {
                    out.push(PaletteItem {
                        label: format!("Go to: {}", display_title(&t.title)),
                        hint: "task",
                        action: PaletteAction::GoTo(t.id.clone()),
                    });
                    n += 1;
                }
            }
        }
        out
    }

    fn run_palette(&mut self, action: PaletteAction, ctx: &egui::Context) {
        match action {
            PaletteAction::NewTask => self.focus_input = true,
            PaletteAction::SetView(view) => {
                self.view = view;
                self.filter = Filter::All;
                self.multi.clear();
                self.collapsed_sections.clear();
            }
            PaletteAction::SetFilter(filter) => {
                self.filter = filter;
                if filter == Filter::Trash {
                    self.view = SmartView::Inbox;
                    self.collapsed_sections.clear();
                }
                self.multi.clear();
            }
            PaletteAction::ToggleCompact => {
                let compact = !self.settings.compact.unwrap_or(false);
                self.settings.compact = Some(compact);
                model::save_settings(&self.settings);
            }
            PaletteAction::ToggleTags => self.tag_rail_open = !self.tag_rail_open,
            PaletteAction::ToggleFocus => {
                self.focus_mode = true;
                if self.selected.is_none() {
                    self.selected = self
                        .visible()
                        .iter()
                        .find(|t| !t.done)
                        .map(|t| t.id.clone());
                }
            }
            PaletteAction::SetSort(mode) => {
                self.sort = mode;
                self.settings.sort = Some(
                    match mode {
                        SortMode::Manual => "manual",
                        SortMode::Due => "due",
                        SortMode::Priority => "priority",
                    }
                    .to_string(),
                );
                model::save_settings(&self.settings);
            }
            PaletteAction::EditTitle => {
                if let Some(id) = self.selected.clone() {
                    self.focus_mode = false;
                    self.start_inline(&id, InlineField::Title);
                }
            }
            PaletteAction::EditNotes => {
                if let Some(id) = self.selected.clone() {
                    self.focus_mode = false;
                    self.start_inline(&id, InlineField::Notes);
                }
            }
            PaletteAction::SetDue => {
                self.due_request = self.selected.clone();
            }
            PaletteAction::Stats => self.stats_open = true,
            PaletteAction::Help => self.help_open = true,
            PaletteAction::ExportMarkdown => self.export_markdown(),
            PaletteAction::ExportJson => self.export_json(),
            PaletteAction::ImportJson => self.import_export(),
            PaletteAction::RestoreBackup => self.restore_backup(),
            PaletteAction::OpenDataFolder => self.open_data_folder(),
            PaletteAction::ToggleReduceMotion => {
                let reduce = !self.settings.reduce_motion.unwrap_or(false);
                self.settings.reduce_motion = Some(reduce);
                model::save_settings(&self.settings);
                ctx.style_mut_of(egui::Theme::Dark, |style| {
                    style.animation_time = if reduce { 0.0 } else { 0.16 };
                });
            }
            PaletteAction::Hide => self.hide_window(ctx),
            PaletteAction::Quit => self.quit(ctx),
            PaletteAction::GoTo(id) => {
                self.focus_mode = false;
                self.jump_to(&id);
                self.kbd_nav = true;
            }
        }
    }

    fn ui_stats_window(&mut self, ctx: &egui::Context) {
        if !self.stats_open {
            return;
        }
        let mut open = true;
        let center = ctx.content_rect().center();
        egui::Window::new("Statistics")
            .collapsible(false)
            .resizable(false)
            .default_width(332.0)
            .fixed_pos(Pos2::new(center.x - 166.0, center.y - 150.0))
            .open(&mut open)
            .show(ctx, |ui| {
                let (open_n, done_n, trash_n) = self.counts();
                let overdue = self.overdue_count();
                ui.horizontal(|ui| {
                    for (label, value, color) in [
                        ("Open", open_n, self.theme.fg),
                        ("Done", done_n, self.theme.ok),
                        ("Overdue", overdue, self.theme.danger),
                        ("Trash", trash_n, self.theme.dim),
                    ] {
                        ui.vertical(|ui| {
                            ui.label(
                                egui::RichText::new(value.to_string())
                                    .font(FontId::new(18.0, FontFamily::Name("bold".into())))
                                    .color(color),
                            );
                            ui.label(
                                egui::RichText::new(label)
                                    .size(9.5)
                                    .color(rgba(self.theme.fg, 0.48)),
                            );
                        });
                        ui.add_space(10.0);
                    }
                });
                ui.add_space(10.0);
                ui.label(
                    egui::RichText::new("Completed · last 4 weeks")
                        .size(10.5)
                        .color(rgba(self.theme.fg, 0.52)),
                );
                let today = Local::now().date_naive();
                let offset = today.weekday().num_days_from_monday() as i64;
                let start = today - chrono::Duration::days(offset + 21);
                let mut counts: HashMap<chrono::NaiveDate, u32> = HashMap::new();
                for t in &self.doc.items {
                    if t.deleted_at > 0 || t.completed_at <= 0 {
                        continue;
                    }
                    if let Some(dt) = Local.timestamp_millis_opt(t.completed_at).single() {
                        *counts.entry(dt.date_naive()).or_insert(0) += 1;
                    }
                }
                let max = counts.values().copied().max().unwrap_or(1).max(1);
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.add_space(24.0);
                    for day in calendar::WEEKDAYS {
                        ui.add_sized(
                            [20.0, 14.0],
                            egui::Label::new(
                                egui::RichText::new(day)
                                    .size(8.5)
                                    .color(rgba(self.theme.fg, 0.42)),
                            ),
                        );
                    }
                });
                for week in 0..4 {
                    ui.horizontal(|ui| {
                        let count = (0..7)
                            .filter_map(|d| {
                                let date = start + chrono::Duration::days(week * 7 + d);
                                if date > today {
                                    None
                                } else {
                                    Some(counts.get(&date).copied().unwrap_or(0))
                                }
                            })
                            .sum::<u32>();
                        ui.add_sized(
                            [20.0, 18.0],
                            egui::Label::new(
                                egui::RichText::new(count.to_string())
                                    .size(8.5)
                                    .color(rgba(self.theme.fg, 0.32)),
                            ),
                        );
                        for d in 0..7 {
                            let date = start + chrono::Duration::days(week * 7 + d);
                            let (rect, resp) =
                                ui.allocate_exact_size(Vec2::new(20.0, 18.0), Sense::hover());
                            if date <= today {
                                let n = counts.get(&date).copied().unwrap_or(0);
                                let t = if n == 0 {
                                    0.0
                                } else {
                                    0.22 + 0.78 * (n as f32 / max as f32)
                                };
                                ui.painter().rect_filled(
                                    rect.shrink(1.5),
                                    CornerRadius::same(3),
                                    mix(self.theme.surface, self.theme.accent, t),
                                );
                                resp.on_hover_text(format!(
                                    "{}  ·  {} completed",
                                    date.format("%d %b"),
                                    n
                                ));
                            } else {
                                ui.painter().rect_filled(
                                    rect.shrink(1.5),
                                    CornerRadius::same(3),
                                    rgba(self.theme.fg, 0.025),
                                );
                            }
                        }
                    });
                }
                let total: u32 = crate::stats::activity_counts(&self.doc, 28, now_ms())
                    .into_iter()
                    .sum();
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new(format!("{total} completions in the last 28 days"))
                        .size(10.0)
                        .color(rgba(self.theme.fg, 0.50)),
                );
            });
        if !open {
            self.stats_open = false;
        }
    }

    fn ui_help_window(&mut self, ctx: &egui::Context) {
        if !self.help_open {
            return;
        }
        let mut open = true;
        let center = ctx.content_rect().center();
        egui::Window::new("Keyboard & gestures")
            .collapsible(false)
            .resizable(false)
            .default_width(380.0)
            .fixed_pos(Pos2::new(center.x - 190.0, center.y - 200.0))
            .open(&mut open)
            .show(ctx, |ui| {
                for (key, what) in [
                    ("Ctrl K", "command palette"),
                    ("↑ ↓ / J K", "move selection"),
                    ("Space", "complete / reopen"),
                    ("Enter", "edit title inline"),
                    ("M", "row actions"),
                    ("D", "due date"),
                    ("/", "search"),
                    ("Ctrl 1…4", "All / Open / Done / Trash"),
                    ("1…5", "Inbox / Today / Upcoming / Overdue / No date"),
                    ("Alt 1…5", "same, works while a field is focused"),
                    ("Ctrl A", "select all"),
                    ("Ctrl Z / ⇧Ctrl Z", "undo / redo"),
                    ("Double click", "edit title"),
                    ("Drag node", "link to another task"),
                    ("Drag ⠿", "reorder row"),
                    ("Esc", "close / hide"),
                ] {
                    ui.horizontal(|ui| {
                        ui.add_sized(
                            [130.0, 18.0],
                            egui::Label::new(
                                egui::RichText::new(key).size(10.5).color(self.theme.accent),
                            ),
                        );
                        ui.label(
                            egui::RichText::new(what)
                                .size(10.5)
                                .color(rgba(self.theme.fg, 0.62)),
                        );
                    });
                }
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new("Tip: click a tag to filter, or use the # rail.")
                        .size(10.0)
                        .color(rgba(self.theme.fg, 0.42)),
                );
            });
        if !open {
            self.help_open = false;
        }
    }

    fn ui_focus(&mut self, ui: &mut egui::Ui) {
        let all_ids: Vec<String> = self.visible().iter().map(|t| t.id.clone()).collect();
        let id = self
            .selected
            .clone()
            .filter(|id| self.task(id).is_some_and(|t| t.deleted_at == 0))
            .or_else(|| {
                self.visible()
                    .iter()
                    .find(|t| !t.done)
                    .map(|t| t.id.clone())
            })
            .or_else(|| all_ids.first().cloned());
        let Some(id) = id else {
            self.ui_focus_empty(ui);
            return;
        };
        self.selected = Some(id.clone());
        let Some(t) = self.task(&id).cloned() else {
            self.ui_focus_empty(ui);
            return;
        };
        let children: Vec<Task> = self
            .doc
            .items
            .iter()
            .filter(|c| c.parent == id && c.deleted_at == 0)
            .cloned()
            .collect();
        let done_children = children.iter().filter(|c| c.done).count();
        let parent_title = self
            .task(&t.parent)
            .map(|p| display_title(&p.title))
            .unwrap_or_default();
        let index = all_ids
            .iter()
            .position(|x| x == &id)
            .map(|i| i + 1)
            .unwrap_or(1);
        let total = all_ids.len().max(1);

        let mut nav_delta = 0;
        let mut parent_jump = false;
        ui.horizontal(|ui| {
            let (mark, _) = ui.allocate_exact_size(Vec2::new(26.0, 26.0), Sense::hover());
            paint_app_mark(ui.painter(), mark, &self.theme);
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new("focus")
                    .size(11.0)
                    .color(rgba(self.theme.fg, 0.50)),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if chip_button(
                    ui,
                    "Exit  ✕",
                    self.theme.fg,
                    rgba(self.theme.fg, 0.06),
                    "Back to the list  ·  Esc",
                ) {
                    self.focus_mode = false;
                }
                if icon_button(ui, Vec2::new(24.0, 24.0), "›", self.theme.dim, "Next task") {
                    nav_delta = 1;
                }
                if icon_button(
                    ui,
                    Vec2::new(24.0, 24.0),
                    "‹",
                    self.theme.dim,
                    "Previous task",
                ) {
                    nav_delta = -1;
                }
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(format!("{index} / {total}"))
                        .size(10.5)
                        .color(rgba(self.theme.fg, 0.40)),
                );
            });
        });
        if nav_delta != 0 && !all_ids.is_empty() {
            let at = all_ids.iter().position(|x| x == &id).unwrap_or(0) as i32;
            let next = ((at + nav_delta).rem_euclid(all_ids.len() as i32)) as usize;
            self.selected = Some(all_ids[next].clone());
        }

        ui.add_space(14.0);
        let card_w = (ui.available_width() - 4.0).min(548.0);
        ui.horizontal(|ui| {
            ui.add_space((ui.available_width() - card_w) / 2.0);
            let card_fill = mix(self.theme.surface, self.theme.bg, 0.18);
            let card = egui::Frame::NONE
                .fill(card_fill)
                .stroke(Stroke::new(1.0, rgba(self.theme.accent, 0.13)))
                .corner_radius(CornerRadius::same(14))
                .inner_margin(egui::Margin::symmetric(24, 18))
                .shadow(egui::epaint::Shadow {
                    offset: [0, 12],
                    blur: 30,
                    spread: 0,
                    color: Color32::from_black_alpha(95),
                })
                .show(ui, |ui| {
                    ui.vertical(|ui| {
                        ui.set_width((card_w - 48.0).max(320.0));
                        if !parent_title.is_empty() {
                            let label = egui::Label::new(
                                egui::RichText::new(format!("↳ {parent_title}"))
                                    .size(10.5)
                                    .color(rgba(self.theme.accent, 0.62)),
                            )
                            .sense(Sense::click());
                            if ui.add(label).on_hover_text("Open parent").clicked() {
                                parent_jump = true;
                            }
                            ui.add_space(2.0);
                        }
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            if t.priority > 0 {
                                focus_pill(
                                    ui,
                                    &format!("P{}", t.priority),
                                    priority_color(t.priority, &self.theme),
                                    &self.theme,
                                );
                            }
                            if t.due > 0 {
                                let overdue = t.due < now_ms();
                                focus_pill(
                                    ui,
                                    &format!("due {}", due_label(t.due)),
                                    if overdue {
                                        self.theme.danger
                                    } else {
                                        self.theme.accent
                                    },
                                    &self.theme,
                                );
                            }
                            if !t.repeat.is_empty() {
                                focus_pill(
                                    ui,
                                    &format!("↻ {}", t.repeat),
                                    self.theme.ok,
                                    &self.theme,
                                );
                            }
                            for tag in t.tags() {
                                focus_pill(
                                    ui,
                                    &format!("#{tag}"),
                                    tag_color(&tag, &self.theme),
                                    &self.theme,
                                );
                            }
                        });
                        ui.add_space(9.0);
                        let mut title = egui::RichText::new(display_title(&t.title))
                            .font(FontId::new(28.0, FontFamily::Name("bold".into())))
                            .color(if t.done {
                                mix(self.theme.fg, self.theme.dim, 0.55)
                            } else {
                                self.theme.fg
                            });
                        if t.done {
                            title = title.strikethrough();
                        }
                        ui.add(egui::Label::new(title).wrap());
                        if !t.notes.is_empty() {
                            ui.add_space(13.0);
                            ui.label(
                                egui::RichText::new("Notes")
                                    .size(9.5)
                                    .color(rgba(self.theme.fg, 0.38)),
                            );
                            ui.add_space(3.0);
                            let notes_width = ui.available_width();
                            let notes = egui::Frame::NONE
                                .fill(mix(self.theme.bg, self.theme.surface, 0.55))
                                .stroke(Stroke::new(1.0, rgba(self.theme.fg, 0.05)))
                                .corner_radius(CornerRadius::same(8))
                                .inner_margin(egui::Margin::symmetric(12, 9))
                                .show(ui, |ui| {
                                    ui.set_min_width((notes_width - 24.0).max(120.0));
                                    egui::ScrollArea::vertical()
                                        .id_salt("focus-notes")
                                        .max_height(118.0)
                                        .show(ui, |ui| {
                                            ui.add(
                                                egui::Label::new(
                                                    egui::RichText::new(&t.notes)
                                                        .size(13.0)
                                                        .color(rgba(self.theme.fg, 0.66)),
                                                )
                                                .wrap(),
                                            );
                                        });
                                });
                            let accent = Rect::from_min_size(
                                Pos2::new(
                                    notes.response.rect.left() + 1.0,
                                    notes.response.rect.top() + 8.0,
                                ),
                                Vec2::new(2.0, (notes.response.rect.height() - 16.0).max(8.0)),
                            );
                            ui.painter().rect_filled(
                                accent,
                                CornerRadius::same(1),
                                rgba(self.theme.accent, 0.4),
                            );
                        }
                        if !children.is_empty() {
                            ui.add_space(13.0);
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new("Subtasks")
                                        .size(9.5)
                                        .color(rgba(self.theme.fg, 0.38)),
                                );
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    ui.label(
                                        egui::RichText::new(format!(
                                            "{done_children} / {}",
                                            children.len()
                                        ))
                                        .size(9.5)
                                        .color(rgba(self.theme.fg, 0.42)),
                                    );
                                });
                            });
                            let (bar, _) = ui.allocate_exact_size(
                                Vec2::new(ui.available_width(), 2.0),
                                Sense::hover(),
                            );
                            let ratio = done_children as f32 / children.len() as f32;
                            ui.painter().rect_filled(
                                bar,
                                CornerRadius::same(1),
                                rgba(self.theme.fg, 0.06),
                            );
                            if ratio > 0.01 {
                                ui.painter().rect_filled(
                                    Rect::from_min_size(
                                        bar.min,
                                        Vec2::new(bar.width() * ratio, bar.height()),
                                    ),
                                    CornerRadius::same(1),
                                    rgba(self.theme.ok, 0.75),
                                );
                            }
                            ui.add_space(4.0);
                            let mut toggle_child = None;
                            egui::ScrollArea::vertical()
                                .id_salt("focus-subtasks")
                                .max_height(118.0)
                                .show(ui, |ui| {
                                    let width = ui.available_width();
                                    for child in &children {
                                        let (rect, resp) = ui.allocate_exact_size(
                                            Vec2::new(width, 27.0),
                                            Sense::click(),
                                        );
                                        if resp.hovered() {
                                            ui.painter().rect_filled(
                                                rect,
                                                CornerRadius::same(5),
                                                rgba(self.theme.fg, 0.035),
                                            );
                                        }
                                        let c = Pos2::new(rect.left() + 9.0, rect.center().y);
                                        ui.painter().circle_filled(
                                            c,
                                            5.5,
                                            if child.done {
                                                self.theme.accent
                                            } else {
                                                self.theme.bg
                                            },
                                        );
                                        ui.painter().circle_stroke(
                                            c,
                                            5.5,
                                            Stroke::new(1.2, rgba(self.theme.fg, 0.42)),
                                        );
                                        if child.done {
                                            ui.painter().text(
                                                c,
                                                Align2::CENTER_CENTER,
                                                "✓",
                                                FontId::new(8.0, FontFamily::Name("bold".into())),
                                                self.theme.bg,
                                            );
                                        }
                                        let max_w = (width - 34.0).max(40.0);
                                        let label = fit_text(
                                            ui,
                                            &display_title(&child.title),
                                            max_w,
                                            12.5,
                                            false,
                                        );
                                        let color = if child.done {
                                            rgba(self.theme.fg, 0.38)
                                        } else {
                                            rgba(self.theme.fg, 0.82)
                                        };
                                        ui.painter().text(
                                            Pos2::new(rect.left() + 22.0, rect.center().y),
                                            Align2::LEFT_CENTER,
                                            &label,
                                            FontId::new(12.5, FontFamily::Monospace),
                                            color,
                                        );
                                        if child.done {
                                            let tw = text_width(ui, &label, 12.5, false).min(max_w);
                                            ui.painter().line_segment(
                                                [
                                                    Pos2::new(rect.left() + 22.0, rect.center().y),
                                                    Pos2::new(
                                                        rect.left() + 22.0 + tw,
                                                        rect.center().y,
                                                    ),
                                                ],
                                                Stroke::new(1.0, rgba(self.theme.fg, 0.32)),
                                            );
                                        }
                                        if resp.clicked() {
                                            toggle_child = Some(child.id.clone());
                                        }
                                    }
                                });
                            if let Some(child) = toggle_child {
                                self.toggle(&child);
                            }
                        }
                        ui.add_space(18.0);
                        let mut complete = false;
                        let mut next = false;
                        let mut open = false;
                        ui.horizontal(|ui| {
                            let (label, text_color, fill, stroke) = if t.done {
                                (
                                    "Reopen",
                                    self.theme.accent,
                                    rgba(self.theme.accent, 0.08),
                                    Stroke::new(1.0, rgba(self.theme.accent, 0.25)),
                                )
                            } else {
                                (
                                    "Complete task",
                                    self.theme.bg,
                                    self.theme.accent,
                                    Stroke::NONE,
                                )
                            };
                            if ui
                                .add(
                                    egui::Button::new(
                                        egui::RichText::new(label).size(12.5).color(text_color),
                                    )
                                    .fill(fill)
                                    .stroke(stroke)
                                    .corner_radius(CornerRadius::same(7))
                                    .min_size(Vec2::new(142.0, 34.0)),
                                )
                                .clicked()
                            {
                                complete = true;
                            }
                            if ui
                                .add(
                                    egui::Button::new(
                                        egui::RichText::new("Next  →")
                                            .size(12.0)
                                            .color(rgba(self.theme.fg, 0.70)),
                                    )
                                    .fill(rgba(self.theme.fg, 0.045))
                                    .corner_radius(CornerRadius::same(7))
                                    .min_size(Vec2::new(88.0, 34.0)),
                                )
                                .clicked()
                            {
                                next = true;
                            }
                            if ui
                                .add(
                                    egui::Button::new(
                                        egui::RichText::new("Open in list")
                                            .size(11.0)
                                            .color(rgba(self.theme.fg, 0.50)),
                                    )
                                    .frame(false)
                                    .min_size(Vec2::new(90.0, 28.0)),
                                )
                                .clicked()
                            {
                                open = true;
                            }
                        });
                        ui.add_space(4.0);
                        ui.label(
                            egui::RichText::new("Space complete  ·  J K move  ·  Esc exit")
                                .size(9.0)
                                .color(rgba(self.theme.fg, 0.34)),
                        );
                        if complete {
                            self.toggle(&id);
                            if !t.done {
                                let next_id = self
                                    .visible()
                                    .iter()
                                    .find(|x| x.id != id && !x.done)
                                    .map(|x| x.id.clone());
                                if let Some(next) = next_id {
                                    self.selected = Some(next);
                                }
                            }
                        }
                        if next {
                            let at = all_ids.iter().position(|x| x == &id).unwrap_or(0) as i32;
                            if !all_ids.is_empty() {
                                let n = ((at + 1).rem_euclid(all_ids.len() as i32)) as usize;
                                self.selected = Some(all_ids[n].clone());
                            }
                        }
                        if open {
                            let parent =
                                self.task(&id).map(|x| x.parent.clone()).unwrap_or_default();
                            self.focus_mode = false;
                            if parent.is_empty() {
                                self.jump_to(&id);
                            }
                        }
                    });
                });
            let rect = card.response.rect;
            ui.painter().rect_filled(
                Rect::from_min_size(
                    Pos2::new(rect.left() + 24.0, rect.top()),
                    Vec2::new(38.0, 2.0),
                ),
                CornerRadius::same(1),
                rgba(self.theme.accent, 0.55),
            );
        });
        if parent_jump {
            let parent = self.task(&id).map(|x| x.parent.clone()).unwrap_or_default();
            if !parent.is_empty() {
                self.focus_mode = false;
                self.jump_to(&parent);
            }
        }
    }

    fn ui_focus_empty(&mut self, ui: &mut egui::Ui) {
        ui.add_space(26.0);
        let card_w = (ui.available_width() - 4.0).min(500.0);
        ui.horizontal(|ui| {
            ui.add_space((ui.available_width() - card_w) / 2.0);
            egui::Frame::NONE
                .fill(mix(self.theme.surface, self.theme.bg, 0.18))
                .stroke(Stroke::new(1.0, rgba(self.theme.accent, 0.12)))
                .corner_radius(CornerRadius::same(14))
                .inner_margin(egui::Margin::symmetric(26, 24))
                .shadow(egui::epaint::Shadow {
                    offset: [0, 12],
                    blur: 30,
                    spread: 0,
                    color: Color32::from_black_alpha(95),
                })
                .show(ui, |ui| {
                    ui.set_width((card_w - 52.0).max(300.0));
                    ui.vertical_centered(|ui| {
                        let (mark, _) =
                            ui.allocate_exact_size(Vec2::new(36.0, 36.0), Sense::hover());
                        paint_app_mark(ui.painter(), mark, &self.theme);
                        ui.add_space(12.0);
                        ui.label(
                            egui::RichText::new("All clear")
                                .font(FontId::new(24.0, FontFamily::Name("bold".into())))
                                .color(self.theme.fg),
                        );
                        ui.add_space(3.0);
                        ui.label(
                            egui::RichText::new("Nothing left in this view.")
                                .size(12.0)
                                .color(rgba(self.theme.fg, 0.52)),
                        );
                        ui.add_space(14.0);
                        ui.horizontal(|ui| {
                            if chip_button(
                                ui,
                                "Back to list",
                                self.theme.fg,
                                rgba(self.theme.fg, 0.06),
                                "Exit focus mode",
                            ) {
                                self.focus_mode = false;
                            }
                            if chip_button(
                                ui,
                                "New task",
                                self.theme.accent,
                                rgba(self.theme.accent, 0.10),
                                "Capture a task",
                            ) {
                                self.focus_mode = false;
                                self.focus_input = true;
                            }
                        });
                    });
                });
        });
    }

    fn ui_palette(&mut self, ctx: &egui::Context) {
        if !self.palette_open {
            return;
        }
        let mut open = true;
        let mut chosen: Option<PaletteAction> = None;
        let center = ctx.content_rect().center();
        let resp = egui::Window::new("Commands")
            .title_bar(false)
            .resizable(false)
            .collapsible(false)
            .fixed_pos(Pos2::new(center.x - 230.0, center.y - 190.0))
            .default_width(460.0)
            .show(ctx, |ui| {
                ui.set_min_width(440.0);
                let search = ui.add_sized(
                    [ui.available_width(), 30.0],
                    egui::TextEdit::singleline(&mut self.palette_query)
                        .id(Id::unique("palette-search"))
                        .hint_text("Type a command or task…")
                        .font(FontId::new(13.0, FontFamily::Monospace)),
                );
                if self.palette_focus {
                    search.request_focus();
                    self.palette_focus = false;
                }
                if search.changed() {
                    self.palette_idx = 0;
                }
                let items = self.palette_items();
                if items.is_empty() {
                    ui.add_space(12.0);
                    ui.label(
                        egui::RichText::new("No matches")
                            .size(12.0)
                            .color(rgba(self.theme.fg, 0.55)),
                    );
                } else {
                    if self.palette_idx >= items.len() {
                        self.palette_idx = items.len() - 1;
                    }
                    let step = ui.input(|i| {
                        if i.key_pressed(egui::Key::ArrowDown) {
                            1
                        } else if i.key_pressed(egui::Key::ArrowUp) {
                            -1
                        } else {
                            0
                        }
                    });
                    if step > 0 {
                        self.palette_idx = (self.palette_idx + 1).min(items.len() - 1);
                    } else if step < 0 {
                        self.palette_idx = self.palette_idx.saturating_sub(1);
                    }
                    let mut scroll_to = None;
                    egui::ScrollArea::vertical()
                        .id_salt("palette-list")
                        .max_height(330.0)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            for (i, item) in items.iter().enumerate() {
                                let selected = i == self.palette_idx;
                                let mut button = egui::Button::new(
                                    egui::RichText::new(format!(
                                        "{}   ·  {}",
                                        item.label, item.hint
                                    ))
                                    .size(12.0)
                                    .color(if selected {
                                        self.theme.fg
                                    } else {
                                        rgba(self.theme.fg, 0.70)
                                    }),
                                )
                                .frame(selected);
                                if selected {
                                    button = button
                                        .fill(rgba(self.theme.accent, 0.12))
                                        .stroke(Stroke::new(1.0, rgba(self.theme.accent, 0.24)));
                                }
                                let row = ui.add_sized([ui.available_width(), 26.0], button);
                                if selected {
                                    scroll_to = Some(row.rect);
                                }
                                if row.clicked() {
                                    chosen = Some(item.action.clone());
                                }
                            }
                            if let Some(rect) = scroll_to {
                                ui.scroll_to_rect(rect, None);
                            }
                        });
                    if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        chosen = Some(items[self.palette_idx].action.clone());
                    }
                }
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    open = false;
                }
                ui.add_space(3.0);
                ui.label(
                    egui::RichText::new("↑↓ navigate  ·  ↵ run  ·  Esc close")
                        .size(9.5)
                        .color(rgba(self.theme.fg, 0.42)),
                );
            });
        if let Some(inner) = &resp {
            if ctx.input(|i| i.pointer.any_click()) {
                if let Some(pos) = ctx.input(|i| i.pointer.interact_pos()) {
                    if !inner.response.rect.contains(pos) {
                        open = false;
                    }
                }
            }
        }
        if !open {
            self.palette_open = false;
            self.palette_query.clear();
        }
        if let Some(action) = chosen {
            self.palette_open = false;
            self.palette_query.clear();
            self.run_palette(action, ctx);
        }
    }

    fn ui_header(&mut self, ui: &mut egui::Ui) {
        let (open, done, _) = self.counts();
        let overdue = self.overdue_count();
        ui.horizontal(|ui| {
            let (mark, mresp) = ui.allocate_exact_size(Vec2::new(30.0, 30.0), Sense::hover());
            paint_app_mark(ui.painter(), mark, &self.theme);
            mresp.on_hover_text("Tasks");
            ui.add_space(9.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.label(
                    egui::RichText::new("tasks")
                        .font(FontId::new(18.5, FontFamily::Name("bold".into())))
                        .color(self.theme.fg),
                );
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 5.0;
                    ui.label(
                        egui::RichText::new(format!("{open} open"))
                            .size(11.0)
                            .color(rgba(self.theme.fg, 0.62)),
                    );
                    ui.label(
                        egui::RichText::new("·")
                            .size(11.0)
                            .color(rgba(self.theme.fg, 0.25)),
                    );
                    ui.label(
                        egui::RichText::new(format!("{done} done"))
                            .size(11.0)
                            .color(rgba(self.theme.fg, 0.62)),
                    );
                    if overdue > 0 {
                        ui.label(
                            egui::RichText::new("·")
                                .size(11.0)
                                .color(rgba(self.theme.fg, 0.25)),
                        );
                        ui.label(
                            egui::RichText::new(format!("{overdue} overdue"))
                                .size(11.0)
                                .color(rgba(self.theme.danger, 0.9)),
                        );
                    }
                });
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if icon_button(
                    ui,
                    Vec2::new(28.0, 28.0),
                    "–",
                    self.theme.dim,
                    "Hide to tray  ·  Esc",
                ) {
                    self.hide_window(ui.ctx());
                }
                if icon_button(ui, Vec2::new(28.0, 28.0), "+", self.theme.dim, "New task") {
                    self.focus_input = true;
                }
                if icon_button(
                    ui,
                    Vec2::new(28.0, 28.0),
                    "⌘",
                    if self.palette_open {
                        self.theme.accent
                    } else {
                        self.theme.dim
                    },
                    "Commands  ·  Ctrl+K",
                ) {
                    self.palette_open = true;
                    self.palette_query.clear();
                    self.palette_idx = 0;
                    self.palette_focus = true;
                }
                let compact = self.settings.compact.unwrap_or(false);
                if icon_button(
                    ui,
                    Vec2::new(28.0, 28.0),
                    "≡",
                    if compact {
                        self.theme.accent
                    } else {
                        self.theme.dim
                    },
                    if compact {
                        "Comfortable rows"
                    } else {
                        "Compact rows"
                    },
                ) {
                    self.settings.compact = Some(!compact);
                    model::save_settings(&self.settings);
                }
                if icon_button(
                    ui,
                    Vec2::new(28.0, 28.0),
                    "◎",
                    if self.focus_mode {
                        self.theme.accent
                    } else {
                        self.theme.dim
                    },
                    "Focus mode",
                ) {
                    self.focus_mode = true;
                    if self.selected.is_none() {
                        self.selected = self
                            .visible()
                            .iter()
                            .find(|t| !t.done)
                            .map(|t| t.id.clone());
                    }
                }
                if icon_button(
                    ui,
                    Vec2::new(28.0, 28.0),
                    "#",
                    if self.tag_rail_open {
                        self.theme.accent2
                    } else {
                        self.theme.dim
                    },
                    if self.tag_rail_open {
                        "Hide tags"
                    } else {
                        "Show tags"
                    },
                ) {
                    self.tag_rail_open = !self.tag_rail_open;
                }
                ui.add_space(2.0);
                let label = if self.daemon_present { "" } else { "tray off" };
                let w = if label.is_empty() {
                    18.0
                } else {
                    text_width(ui, label, 10.0, false) + 18.0
                };
                let (dot, dresp) = ui.allocate_exact_size(Vec2::new(w, 24.0), Sense::hover());
                if self.daemon_present {
                    ui.painter()
                        .circle_filled(dot.center(), 3.0, rgba(self.theme.fg, 0.34));
                } else {
                    ui.painter().circle_filled(
                        Pos2::new(dot.left() + 4.0, dot.center().y),
                        3.0,
                        rgba(self.theme.danger, 0.9),
                    );
                    ui.painter().text(
                        Pos2::new(dot.left() + 11.0, dot.center().y),
                        Align2::LEFT_CENTER,
                        label,
                        FontId::new(10.0, FontFamily::Monospace),
                        rgba(self.theme.danger, 0.9),
                    );
                }
                dresp.on_hover_text(if self.daemon_present {
                    "Tray daemon running"
                } else {
                    "Tray daemon stopped — closing will exit"
                });
            });
        });
    }

    fn ui_progress(&mut self, ui: &mut egui::Ui) {
        let (open, done, _) = self.counts();
        let total = open + done;
        let ratio = if total > 0 {
            done as f32 / total as f32
        } else {
            0.0
        };
        let anim = ui
            .ctx()
            .animate_value_with_time(Id::unique("progress-anim"), ratio, 0.35);
        ui.add_space(12.0);
        let (rect, resp) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 3.0), Sense::hover());
        let p = ui.painter();
        p.rect_filled(rect, CornerRadius::same(2), rgba(self.theme.fg, 0.065));
        if anim > 0.001 {
            let w = (rect.width() * anim).max(3.0);
            let fill = Rect::from_min_size(rect.min, Vec2::new(w, rect.height()));
            p.rect_filled(
                fill,
                CornerRadius::same(2),
                mix(self.theme.accent2, self.theme.accent, 0.72),
            );
            if w < rect.width() - 1.0 {
                p.circle_filled(
                    Pos2::new(fill.right(), fill.center().y),
                    2.4,
                    rgba(self.theme.accent, 0.9),
                );
            }
        }
        resp.on_hover_text(format!(
            "{done} of {total} done  ·  {}%",
            if total > 0 {
                (done * 100 + total / 2) / total
            } else {
                0
            }
        ));
        ui.add_space(13.0);
    }

    fn ui_filters(&mut self, ui: &mut egui::Ui) {
        let (open, done, trash) = self.counts();
        let items = [
            ("All", open + done, Filter::All),
            ("Open", open, Filter::Open),
            ("Done", done, Filter::Done),
            ("Trash", trash, Filter::Trash),
        ];
        let labels: Vec<(String, String)> = items
            .iter()
            .map(|(l, c, _)| (l.to_string(), c.to_string()))
            .collect();
        let widths: Vec<f32> = labels
            .iter()
            .map(|(l, c)| text_width(ui, l, 12.0, false) + text_width(ui, c, 12.0, false) + 32.0)
            .collect();
        let total: f32 = widths.iter().sum::<f32>() + 8.0;
        let (rect, _) = ui.allocate_exact_size(Vec2::new(total, 32.0), Sense::hover());
        ui.painter()
            .rect_filled(rect, CornerRadius::same(8), self.theme.surface);
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(8),
            Stroke::new(1.0, rgba(self.theme.fg, 0.045)),
            egui::StrokeKind::Inside,
        );
        let mut x = rect.left() + 4.0;
        for (((_, _, filter), (label_text, count_text)), w) in
            items.iter().zip(labels.iter()).zip(widths.iter())
        {
            let w = *w;
            let tr = Rect::from_min_size(
                Pos2::new(x, rect.top() + 3.0),
                Vec2::new(w, rect.height() - 6.0),
            );
            let resp = ui.interact(tr, Id::unique(("filter-tab", label_text)), Sense::click());
            let active = self.filter == *filter;
            if active {
                ui.painter()
                    .rect_filled(tr, CornerRadius::same(6), rgba(self.theme.accent, 0.10));
                ui.painter().rect_stroke(
                    tr,
                    CornerRadius::same(6),
                    Stroke::new(1.0, rgba(self.theme.accent, 0.20)),
                    egui::StrokeKind::Inside,
                );
            } else if resp.hovered() {
                ui.painter()
                    .rect_filled(tr, CornerRadius::same(6), rgba(self.theme.fg, 0.04));
            }
            let lw = text_width(ui, label_text, 12.0, false);
            let cw = text_width(ui, count_text, 12.0, false);
            let start = tr.center().x - (lw + 5.0 + cw) / 2.0;
            let label_color = if active {
                self.theme.accent
            } else if resp.hovered() {
                rgba(self.theme.fg, 0.78)
            } else {
                rgba(self.theme.fg, 0.72)
            };
            let count_color = if active {
                rgba(self.theme.accent, 0.62)
            } else {
                rgba(self.theme.fg, 0.55)
            };
            ui.painter().text(
                Pos2::new(start, tr.center().y),
                Align2::LEFT_CENTER,
                label_text,
                FontId::new(12.0, FontFamily::Monospace),
                label_color,
            );
            ui.painter().text(
                Pos2::new(start + lw + 5.0, tr.center().y),
                Align2::LEFT_CENTER,
                count_text,
                FontId::new(12.0, FontFamily::Monospace),
                count_color,
            );
            if resp.clicked() {
                self.filter = *filter;
                if *filter == Filter::Trash {
                    self.view = SmartView::Inbox;
                    self.collapsed_sections.clear();
                }
                self.multi.clear();
            }
            x += w;
        }
        ui.add_space(8.0);
    }

    fn ui_views(&mut self, ui: &mut egui::Ui) {
        let views = [
            SmartView::Inbox,
            SmartView::Today,
            SmartView::Upcoming,
            SmartView::Overdue,
            SmartView::NoDate,
        ];
        let mut new_view = None;
        let mut new_sort = None;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for view in views {
                let active = self.view == view;
                let color = match view {
                    SmartView::Inbox => self.theme.fg,
                    SmartView::Today => self.theme.accent,
                    SmartView::Upcoming => self.theme.accent2,
                    SmartView::Overdue => self.theme.danger,
                    SmartView::NoDate => self.theme.dim,
                };
                let label = format!("{} {}", view.label(), self.view_count(view));
                let chip_color = if active { color } else { rgba(color, 0.62) };
                if chip_button(
                    ui,
                    &label,
                    chip_color,
                    rgba(color, if active { 0.16 } else { 0.045 }),
                    "Smart view",
                ) {
                    new_view = Some(view);
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let label = format!("⇅ {}", self.sort.label());
                let w = text_width(ui, &label, 10.5, false) + 16.0;
                let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 22.0), Sense::click());
                if resp.hovered() {
                    ui.painter().rect_filled(
                        rect,
                        CornerRadius::same(5),
                        rgba(self.theme.fg, 0.06),
                    );
                }
                ui.painter().text(
                    rect.center(),
                    Align2::CENTER_CENTER,
                    &label,
                    FontId::new(10.5, FontFamily::Monospace),
                    rgba(self.theme.fg, if resp.hovered() { 0.85 } else { 0.58 }),
                );
                egui::Popup::menu(&resp)
                    .align(egui::RectAlign::BOTTOM_END)
                    .gap(3.0)
                    .width(148.0)
                    .show(|ui| {
                        for mode in [SortMode::Manual, SortMode::Due, SortMode::Priority] {
                            if ui
                                .selectable_label(
                                    self.sort == mode,
                                    egui::RichText::new(mode.label()).size(12.0),
                                )
                                .clicked()
                            {
                                new_sort = Some(mode);
                            }
                        }
                    });
            });
        });
        ui.add_space(5.0);
        if let Some(view) = new_view {
            self.view = view;
            self.filter = Filter::All;
            self.multi.clear();
            self.collapsed_sections.clear();
        }
        if self.view == SmartView::Inbox {
            self.collapsed_sections.clear();
        } else {
            let sections: Vec<String> = self.collapsed_sections.iter().cloned().collect();
            for section in sections {
                if self.section_count(&section) == 0 {
                    self.collapsed_sections.remove(&section);
                }
            }
        }
        if !self.collapsed_sections.is_empty() {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.label(
                    egui::RichText::new("collapsed")
                        .size(9.5)
                        .color(rgba(self.theme.fg, 0.40)),
                );
                let mut expand = None;
                let mut sections: Vec<String> = self.collapsed_sections.iter().cloned().collect();
                sections.sort();
                for section in sections {
                    let count = self.section_count(&section);
                    if chip_button(
                        ui,
                        &format!("{section}  {count}  +"),
                        self.theme.dim,
                        rgba(self.theme.fg, 0.05),
                        "Expand section",
                    ) {
                        expand = Some(section);
                    }
                }
                if let Some(section) = expand {
                    self.collapsed_sections.remove(&section);
                }
            });
            ui.add_space(2.0);
        }
        if let Some(mode) = new_sort {
            self.sort = mode;
            self.settings.sort = Some(
                match mode {
                    SortMode::Manual => "manual",
                    SortMode::Due => "due",
                    SortMode::Priority => "priority",
                }
                .to_string(),
            );
            model::save_settings(&self.settings);
        }
    }

    fn ui_active_filters(&mut self, ui: &mut egui::Ui) -> bool {
        if self.tag_filter.is_none() && self.search.is_empty() {
            return false;
        }
        let mut clear_tag = false;
        let mut clear_search = false;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.label(
                egui::RichText::new("filtering")
                    .size(10.5)
                    .color(rgba(self.theme.fg, 0.56)),
            );
            if let Some(tag) = self.tag_filter.clone() {
                if chip_button(
                    ui,
                    &format!("#{tag}  ✕"),
                    self.theme.accent,
                    rgba(self.theme.accent, 0.10),
                    "Clear tag filter",
                ) {
                    clear_tag = true;
                }
            }
            if !self.search.is_empty() {
                if chip_button(
                    ui,
                    &format!("/ {}  ✕", truncate(&self.search, 28)),
                    self.theme.accent2,
                    rgba(self.theme.accent2, 0.10),
                    "Clear search",
                ) {
                    clear_search = true;
                }
            }
        });
        ui.add_space(4.0);
        if clear_tag {
            self.tag_filter = None;
        }
        if clear_search {
            self.search.clear();
            if self.input.starts_with('/') {
                self.input.clear();
            }
        }
        false
    }

    fn ui_error_banner(&mut self, ui: &mut egui::Ui) {
        let Some(err) = self.load_error.clone() else {
            return;
        };
        let (rect, _) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 50.0), Sense::hover());
        ui.painter()
            .rect_filled(rect, CornerRadius::same(8), rgba(self.theme.danger, 0.08));
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(8),
            Stroke::new(1.0, rgba(self.theme.danger, 0.28)),
            egui::StrokeKind::Inside,
        );
        ui.painter().circle_stroke(
            Pos2::new(rect.left() + 18.0, rect.center().y),
            7.0,
            Stroke::new(1.2, rgba(self.theme.danger, 0.8)),
        );
        ui.painter().text(
            Pos2::new(rect.left() + 18.0, rect.center().y),
            Align2::CENTER_CENTER,
            "!",
            FontId::new(10.0, FontFamily::Name("bold".into())),
            self.theme.danger,
        );
        ui.painter().text(
            Pos2::new(rect.left() + 34.0, rect.top() + 15.0),
            Align2::LEFT_CENTER,
            "State file unreadable — editing is off",
            FontId::new(12.0, FontFamily::Monospace),
            self.theme.danger,
        );
        ui.painter().text(
            Pos2::new(rect.left() + 34.0, rect.top() + 33.0),
            Align2::LEFT_CENTER,
            truncate(&err, 76),
            FontId::new(10.5, FontFamily::Monospace),
            rgba(self.theme.fg, 0.66),
        );
        let retry = Rect::from_min_size(
            Pos2::new(rect.right() - 68.0, rect.center().y - 11.0),
            Vec2::new(58.0, 22.0),
        );
        if chip_rect(
            ui,
            retry,
            "Retry",
            self.theme.accent,
            Id::unique("retry-state"),
        ) {
            self.reload_from_disk(false);
        }
        ui.add_space(8.0);
    }

    fn ui_tag_rail(&mut self, ui: &mut egui::Ui, h: f32) {
        let width = 108.0;
        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, h), Sense::hover());
        let p = ui.painter();
        p.rect_filled(rect, CornerRadius::same(8), self.theme.surface);
        p.rect_stroke(
            rect,
            CornerRadius::same(8),
            Stroke::new(1.0, rgba(self.theme.fg, 0.045)),
            egui::StrokeKind::Inside,
        );
        let content = rect.shrink2(Vec2::new(8.0, 8.0));
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(content)
                .layout(Layout::top_down(Align::Min)),
        );
        child.spacing_mut().item_spacing = Vec2::new(4.0, 3.0);
        child.horizontal(|ui| {
            ui.label(
                egui::RichText::new("Tags")
                    .size(11.5)
                    .color(rgba(self.theme.fg, 0.58)),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new("‹")
                                .size(13.0)
                                .color(rgba(self.theme.fg, 0.48)),
                        )
                        .frame(false),
                    )
                    .on_hover_text("Hide tags")
                    .clicked()
                {
                    self.tag_rail_open = false;
                }
            });
        });
        let (divider, _) =
            child.allocate_exact_size(Vec2::new(child.available_width(), 1.0), Sense::hover());
        child
            .painter()
            .rect_filled(divider, CornerRadius::ZERO, rgba(self.theme.fg, 0.05));
        child.add_space(3.0);
        let (open, done, _) = self.counts();
        if rail_item(
            &mut child,
            "All tasks",
            open + done,
            self.tag_filter.is_none(),
            self.theme.accent,
            &self.theme,
        ) {
            self.tag_filter = None;
        }
        let tags = model::tag_counts(&self.doc);
        let rail_h = (child.available_height() - 4.0).max(40.0);
        egui::ScrollArea::vertical()
            .id_salt("tag-rail")
            .auto_shrink([false, false])
            .max_height(rail_h)
            .show(&mut child, |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                if tags.is_empty() {
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new("No tags yet. Add #tags in a title or notes.")
                            .size(10.0)
                            .color(rgba(self.theme.fg, 0.42)),
                    );
                }
                for (tag, count) in tags {
                    let active = self
                        .tag_filter
                        .as_deref()
                        .is_some_and(|t| t.eq_ignore_ascii_case(&tag));
                    if rail_item(
                        ui,
                        &format!("#{tag}"),
                        count,
                        active,
                        tag_color(&tag, &self.theme),
                        &self.theme,
                    ) {
                        self.set_tag_filter(&tag);
                    }
                }
            });
    }

    fn ui_list(&mut self, ui: &mut egui::Ui) {
        let tasks: Vec<Task> = self.visible().into_iter().cloned().collect();
        let titles: HashMap<String, String> = self
            .doc
            .items
            .iter()
            .map(|t| (t.id.clone(), t.title.clone()))
            .collect();
        let width = ui.available_width();
        let notes_wrap = (width - TITLE_X - 132.0).max(120.0);
        let compact = self.settings.compact.unwrap_or(false);
        let expand_time = if self.settings.reduce_motion.unwrap_or(false) {
            0.001
        } else {
            0.16
        };

        struct Metrics {
            h: f32,
            header: Option<String>,
            header_h: f32,
            meta_y: f32,
            meta_h: f32,
            notes_y: f32,
            notes_h: f32,
            actions_y: f32,
            actions_h: f32,
        }

        let mut metrics: Vec<Metrics> = Vec::with_capacity(tasks.len());
        let mut anims: Vec<f32> = Vec::with_capacity(tasks.len());
        let mut last_header: Option<&str> = None;
        for t in &tasks {
            let header_h = match self.due_bucket(t) {
                Some(section) if last_header != Some(section) => {
                    last_header = Some(section);
                    26.0
                }
                _ => 0.0,
            };
            let editing_notes = self
                .inline
                .as_ref()
                .is_some_and(|e| e.id == t.id && e.field == InlineField::Notes);
            let row_compact = compact && !editing_notes;
            let anim = ui.ctx().animate_bool_with_time(
                Id::unique(("expand-anim", &t.id)),
                t.expanded,
                expand_time,
            );
            anims.push(anim);
            let has_meta = !row_compact
                && (!t.parent.is_empty()
                    || !t.tags().is_empty()
                    || t.priority > 0
                    || !t.repeat.is_empty());
            let meta_h = if has_meta { 19.0 } else { 0.0 };
            let full_notes_h = if editing_notes {
                82.0
            } else if row_compact || t.notes.is_empty() {
                0.0
            } else {
                let galley = ui.painter().layout(
                    t.notes.clone(),
                    FontId::new(12.5, FontFamily::Monospace),
                    rgba(self.theme.fg, 0.58),
                    notes_wrap,
                );
                galley.size().y.min(150.0)
            };
            let collapsed_notes_h = if editing_notes {
                82.0
            } else if row_compact || t.notes.is_empty() {
                0.0
            } else {
                17.0
            };
            let notes_h = if editing_notes {
                82.0
            } else {
                collapsed_notes_h + (full_notes_h - collapsed_notes_h) * anim
            };
            let actions_h = if row_compact || editing_notes {
                0.0
            } else {
                28.0 * anim
            };
            let mut y = if row_compact { 24.0 } else { 31.0 };
            let meta_y = y;
            y += meta_h;
            let notes_y = y;
            y += notes_h;
            let actions_y = y;
            y += actions_h;
            let h = if row_compact {
                (y + 8.0).max(38.0)
            } else {
                (y + 8.0).max(54.0)
            };
            metrics.push(Metrics {
                h,
                header: self.due_bucket(t).map(str::to_string),
                header_h,
                meta_y,
                meta_h,
                notes_y,
                notes_h,
                actions_y,
                actions_h,
            });
        }

        let total_h: f32 = metrics.iter().map(|m| m.h + m.header_h).sum::<f32>() + 6.0;
        let empty_h = if tasks.is_empty() {
            ui.available_height().max(190.0)
        } else {
            60.0
        };
        let (content_rect, _) =
            ui.allocate_exact_size(Vec2::new(width, total_h.max(empty_h)), Sense::hover());

        let mut positions: Vec<(String, Rect)> = Vec::new();
        let mut y = content_rect.top() + 2.0;
        for (t, m) in tasks.iter().zip(metrics.iter()) {
            y += m.header_h;
            let r = Rect::from_min_size(Pos2::new(content_rect.left(), y), Vec2::new(width, m.h));
            positions.push((t.id.clone(), r));
            y += m.h;
        }
        let pos_of = |id: &str| positions.iter().find(|(i, _)| i == id).map(|(_, r)| *r);
        let lane_x = |lane: usize| content_rect.left() + LANE_X + lane as f32 * LANE_STEP;
        let ny_for = |r: Rect| {
            if compact && r.height() <= 44.0 {
                16.0
            } else {
                NODE_Y
            }
        };
        let node_y = |r: Rect| r.top() + ny_for(r);

        let mut expand_id: Option<String> = None;
        let mut toggle_id: Option<String> = None;
        let mut delete_id: Option<String> = None;
        let mut restore_id: Option<String> = None;
        let mut purge_id: Option<String> = None;
        let mut due_id: Option<String> = self.due_request.take();
        let mut edit_id: Option<String> = None;
        let mut link_start: Option<String> = None;
        let mut link_now: Option<(String, String)> = None;
        let mut tag_click: Option<String> = None;
        let mut parent_jump: Option<String> = None;
        let mut subtask_id: Option<String> = None;
        let mut unlink_id: Option<String> = None;
        let mut mark_id: Option<(String, bool)> = None;
        let mut reorder: Option<(String, String, bool, f32)> = None;
        let mut clear_filters = false;
        let mut inline_start: Option<(String, InlineField)> = None;
        let mut inline_finish: Option<bool> = None;
        let mut duplicate_id: Option<String> = None;
        let mut duplicate_subtree_id: Option<String> = None;
        let mut priority_set: Option<(String, u8)> = None;
        let drop_target: Option<String> = if self.drag_id.is_some() && self.drag_y > 0.0 {
            positions
                .iter()
                .find(|(_, r)| self.drag_y < r.center().y)
                .map(|(id, _)| id.clone())
        } else {
            None
        };

        // Graph branches behind the rows: smooth S-curves from each parent to
        // its children. Unrelated roots stay visually independent.
        {
            let p = ui.painter();
            let lineage = self
                .selected
                .as_ref()
                .map(|id| self.lineage_ids(id))
                .unwrap_or_default();
            let palette = [
                self.theme.accent,
                self.theme.accent2,
                mix(self.theme.accent, self.theme.accent2, 0.5),
                mix(self.theme.accent, self.theme.ok, 0.55),
            ];
            let flash_child = self
                .link_flash
                .as_ref()
                .filter(|f| f.at.elapsed() < Duration::from_millis(950))
                .map(|f| f.child.clone());
            for t in &tasks {
                if t.parent.is_empty() || flash_child.as_deref() == Some(t.id.as_str()) {
                    continue;
                }
                let Some(pr) = pos_of(&t.parent) else {
                    continue;
                };
                let Some(cr) = pos_of(&t.id) else { continue };
                let lane = self.lane(&t.id);
                let from = Pos2::new(lane_x(self.lane(&t.parent)), node_y(pr));
                let to = Pos2::new(lane_x(lane), node_y(cr));
                let color = palette[lane % palette.len()];
                let on_path =
                    lineage.is_empty() || lineage.contains(&t.id) || lineage.contains(&t.parent);
                let alpha: f32 = if t.done { 0.22 } else { 0.58 };
                let alpha = if lineage.is_empty() {
                    alpha
                } else if on_path {
                    (alpha + 0.22).min(0.92)
                } else {
                    alpha * 0.28
                };
                paint_branch(p, from, to, Stroke::new(1.35, rgba(color, alpha)));
            }
        }

        // Animated link formation.
        if let Some(flash) = &self.link_flash {
            let elapsed = flash.at.elapsed();
            if elapsed < Duration::from_millis(950) {
                let t = (elapsed.as_secs_f32() / 0.95).clamp(0.0, 1.0);
                if let (Some(pr), Some(cr)) = (pos_of(&flash.parent), pos_of(&flash.child)) {
                    let from = Pos2::new(lane_x(self.lane(&flash.parent)), node_y(pr));
                    let to = Pos2::new(lane_x(self.lane(&flash.child)), node_y(cr));
                    let color = rgba(self.theme.accent, (1.0 - t) * 0.95 + 0.05);
                    let p = ui.painter();
                    let eased = 1.0 - (1.0 - t).powi(3);
                    let points = branch_points(from, to);
                    let steps = 28;
                    let mut path = Vec::with_capacity(steps + 1);
                    for k in 0..=steps {
                        let u = eased * k as f32 / steps as f32;
                        path.push(cubic_point(points, u));
                    }
                    let stroke = Stroke::new(1.8 + 1.6 * (1.0 - t), color);
                    p.add(egui::Shape::line(path, stroke));
                    let head = cubic_point(points, eased);
                    p.circle_filled(head, 3.0 * (1.0 - t) + 1.5, color);
                    p.circle_stroke(to, 8.0 + 10.0 * t, Stroke::new(2.0 * (1.0 - t), color));
                }
                ui.ctx().request_repaint_after(Duration::from_millis(16));
            } else {
                self.link_flash = None;
            }
        }

        // Section headers for smart views.
        let mut new_collapse: Option<String> = None;
        for (t, m) in tasks.iter().zip(metrics.iter()) {
            if m.header_h <= 0.0 {
                continue;
            }
            let Some(header) = &m.header else { continue };
            let Some(r) = pos_of(&t.id) else { continue };
            let count = tasks
                .iter()
                .filter(|x| self.due_bucket(x) == Some(header.as_str()))
                .count();
            let color = match header.as_str() {
                "Overdue" => self.theme.danger,
                "Today" | "Tomorrow" => self.theme.accent,
                "This week" => self.theme.accent2,
                _ => self.theme.dim,
            };
            let label = format!("{header}  {count}");
            let y = r.top() - m.header_h + 13.0;
            let header_rect = Rect::from_min_size(
                Pos2::new(r.left() + TITLE_X, r.top() - m.header_h),
                Vec2::new(r.width() - TITLE_X - 8.0, m.header_h),
            );
            let hresp = ui.interact(header_rect, Id::unique(("section", header)), Sense::click());
            if hresp.hovered() {
                ui.painter().rect_filled(
                    header_rect,
                    CornerRadius::same(5),
                    rgba(self.theme.fg, 0.03),
                );
            }
            hresp.clone().on_hover_text("Collapse section");
            if hresp.clicked() {
                new_collapse = Some(header.clone());
            }
            let p = ui.painter();
            p.text(
                Pos2::new(r.left() + TITLE_X, y),
                Align2::LEFT_CENTER,
                &label,
                FontId::new(10.5, FontFamily::Name("bold".into())),
                rgba(color, 0.82),
            );
            let tw = text_width(ui, &label, 10.5, true);
            p.line_segment(
                [
                    Pos2::new(r.left() + TITLE_X + tw + 8.0, y),
                    Pos2::new(r.right() - 26.0, y),
                ],
                Stroke::new(1.0, rgba(self.theme.fg, 0.055)),
            );
            p.text(
                Pos2::new(r.right() - 14.0, y),
                Align2::CENTER_CENTER,
                "⌄",
                FontId::new(10.0, FontFamily::Monospace),
                rgba(self.theme.fg, 0.32),
            );
        }
        if let Some(section) = new_collapse {
            self.collapsed_sections.insert(section);
        }

        for (idx, t) in tasks.iter().enumerate() {
            let r = positions[idx].1;
            let m = &metrics[idx];
            let anim = anims[idx];
            let row = r.shrink2(Vec2::new(0.0, 2.0));
            let resp = ui.interact(row, Id::unique(("row", &t.id)), Sense::click());
            let selected = self.selected.as_deref() == Some(t.id.as_str());
            let hovered = resp.hovered();
            let editing_title = self
                .inline
                .as_ref()
                .is_some_and(|e| e.id == t.id && e.field == InlineField::Title);
            let editing_notes = self
                .inline
                .as_ref()
                .is_some_and(|e| e.id == t.id && e.field == InlineField::Notes);
            let row_compact = compact && !editing_notes;
            let ny = ny_for(r);
            let link_highlight =
                self.link_from.is_some() && self.link_from.as_deref() != Some(t.id.as_str());
            if selected {
                ui.painter().rect_filled(
                    row,
                    CornerRadius::same(8),
                    rgba(self.theme.accent, 0.045),
                );
                ui.painter().rect_stroke(
                    row,
                    CornerRadius::same(8),
                    Stroke::new(
                        if self.kbd_nav { 1.2 } else { 1.0 },
                        rgba(self.theme.accent, if self.kbd_nav { 0.38 } else { 0.12 }),
                    ),
                    egui::StrokeKind::Inside,
                );
                ui.painter().rect_filled(
                    Rect::from_min_size(
                        Pos2::new(row.left() + 1.0, row.top() + 10.0),
                        Vec2::new(2.0, (row.height() - 20.0).max(8.0)),
                    ),
                    CornerRadius::same(1),
                    rgba(self.theme.accent, if self.kbd_nav { 1.0 } else { 0.85 }),
                );
            } else if hovered {
                ui.painter()
                    .rect_filled(row, CornerRadius::same(8), rgba(self.theme.fg, 0.028));
            }
            if drop_target.as_deref() == Some(t.id.as_str())
                && self.drag_id.as_deref() != Some(t.id.as_str())
            {
                ui.painter().rect_filled(
                    row,
                    CornerRadius::same(8),
                    rgba(self.theme.accent, 0.035),
                );
                ui.painter().rect_stroke(
                    row,
                    CornerRadius::same(8),
                    Stroke::new(1.0, rgba(self.theme.accent, 0.18)),
                    egui::StrokeKind::Inside,
                );
            }
            if link_highlight && hovered {
                ui.painter().rect_stroke(
                    row,
                    CornerRadius::same(8),
                    Stroke::new(1.0, rgba(self.theme.accent, 0.5)),
                    egui::StrokeKind::Inside,
                );
            }
            if let Some(at) = self.done_flash.get(&t.id) {
                let e = (at.elapsed().as_secs_f32() / 0.5).clamp(0.0, 1.0);
                if e < 1.0 {
                    ui.painter().rect_filled(
                        row,
                        CornerRadius::same(8),
                        rgba(self.theme.accent, 0.055 * (1.0 - e)),
                    );
                    ui.ctx().request_repaint_after(Duration::from_millis(16));
                }
            }

            if resp.clicked() {
                self.kbd_nav = false;
                if let Some(from) = self.link_from.clone() {
                    if from != t.id {
                        link_now = Some((from, t.id.clone()));
                    }
                } else if ui.input(|i| i.modifiers.command) {
                    if self.multi.contains(&t.id) {
                        self.multi.remove(&t.id);
                    } else {
                        self.multi.insert(t.id.clone());
                    }
                } else {
                    self.selected = Some(t.id.clone());
                }
            }
            if resp.double_clicked() && self.filter != Filter::Trash {
                inline_start = Some((t.id.clone(), InlineField::Title));
            }

            // Multi-select checkbox.
            if !self.multi.is_empty() {
                let boxr = Rect::from_center_size(
                    Pos2::new(row.left() + 9.0, row.top() + ny),
                    Vec2::new(13.0, 13.0),
                );
                let on = self.multi.contains(&t.id);
                ui.painter().rect_filled(
                    boxr,
                    CornerRadius::same(3),
                    if on {
                        rgba(self.theme.accent, 0.16)
                    } else {
                        Color32::TRANSPARENT
                    },
                );
                ui.painter().rect_stroke(
                    boxr,
                    CornerRadius::same(3),
                    Stroke::new(1.0, rgba(self.theme.fg, if on { 0.6 } else { 0.3 })),
                    egui::StrokeKind::Inside,
                );
                if on {
                    ui.painter().text(
                        boxr.center(),
                        Align2::CENTER_CENTER,
                        "✓",
                        FontId::new(10.0, FontFamily::Monospace),
                        self.theme.accent,
                    );
                }
            }

            // Done node.
            let lane = self.lane(&t.id);
            let center = Pos2::new(lane_x(lane), node_y(r));
            let done_t = ui.ctx().animate_bool_with_time(
                Id::unique(("done-anim", &t.id)),
                t.done,
                if self.settings.reduce_motion.unwrap_or(false) {
                    0.001
                } else {
                    0.2
                },
            );
            let p = ui.painter().with_clip_rect(row);
            if self.filter == Filter::Trash {
                p.text(
                    center,
                    Align2::CENTER_CENTER,
                    "\u{f1f8}",
                    FontId::new(9.0, FontFamily::Monospace),
                    rgba(self.theme.danger, 0.55),
                );
            } else {
                let node = Rect::from_center_size(center, Vec2::new(26.0, 26.0));
                let nresp = ui.interact(node, Id::unique(("node", &t.id)), Sense::click_and_drag());
                if nresp.hovered() || selected {
                    p.circle_filled(center, 10.5, rgba(self.theme.accent, 0.07));
                }
                p.circle_filled(
                    center,
                    7.0,
                    if t.done {
                        self.theme.accent
                    } else {
                        self.theme.bg
                    },
                );
                p.circle_stroke(
                    center,
                    7.0,
                    Stroke::new(
                        if t.done { 1.0 } else { 1.4 },
                        if t.done {
                            rgba(self.theme.accent, 0.9)
                        } else if nresp.hovered() {
                            rgba(self.theme.fg, 0.78)
                        } else {
                            rgba(self.theme.fg, 0.38)
                        },
                    ),
                );
                if t.done || done_t > 0.02 {
                    p.text(
                        center,
                        Align2::CENTER_CENTER,
                        "✓",
                        FontId::new(10.0, FontFamily::Name("bold".into())),
                        rgba(self.theme.bg, done_t),
                    );
                }
                if let Some(at) = self.done_flash.get(&t.id) {
                    let e = (at.elapsed().as_secs_f32() / 0.5).clamp(0.0, 1.0);
                    if e < 1.0 {
                        p.circle_stroke(
                            center,
                            7.0 + 16.0 * e,
                            Stroke::new(2.2 * (1.0 - e), rgba(self.theme.accent, 0.9 * (1.0 - e))),
                        );
                        ui.ctx().request_repaint_after(Duration::from_millis(16));
                    }
                }
                if nresp.clicked() && self.link_from.is_none() && self.link_drag.is_none() {
                    toggle_id = Some(t.id.clone());
                }
                if nresp.drag_started() {
                    self.link_drag = Some(t.id.clone());
                    self.selected = Some(t.id.clone());
                }
            }

            // Chevron for expand/collapse.
            if !row_compact {
                let chev = Rect::from_center_size(
                    Pos2::new(row.left() + TITLE_X - 17.0, row.top() + ny),
                    Vec2::new(18.0, 24.0),
                );
                let cresp = ui.interact(chev, Id::unique(("chev", &t.id)), Sense::click());
                if cresp.hovered() {
                    ui.painter().rect_filled(
                        chev,
                        CornerRadius::same(5),
                        rgba(self.theme.fg, 0.055),
                    );
                }
                {
                    let c = chev.center();
                    let col = rgba(self.theme.fg, if cresp.hovered() { 0.9 } else { 0.42 });
                    let stroke = Stroke::new(1.4, col);
                    let angle = (1.0 - anim) * std::f32::consts::FRAC_PI_2;
                    let (sin, cos) = angle.sin_cos();
                    let rot = |x: f32, y: f32| {
                        Pos2::new(c.x + x * cos - y * sin, c.y + x * sin + y * cos)
                    };
                    let tip = rot(0.0, 2.5);
                    ui.painter().line_segment([rot(-4.0, -2.0), tip], stroke);
                    ui.painter().line_segment([tip, rot(4.0, -2.0)], stroke);
                }
                if cresp.clicked() {
                    expand_id = Some(t.id.clone());
                }
            }

            if row_compact && t.priority > 0 {
                ui.painter().rect_filled(
                    Rect::from_min_size(
                        Pos2::new(row.left() + TITLE_X - 8.0, row.top() + ny - 5.0),
                        Vec2::new(2.5, 10.0),
                    ),
                    CornerRadius::same(1),
                    priority_color(t.priority, &self.theme),
                );
            }

            // Title.
            let title_right = if self.filter == Filter::Trash {
                row.right() - 206.0
            } else {
                row.right() - 160.0
            };
            let title_clip = Rect::from_min_max(
                Pos2::new(row.left() + TITLE_X, row.top() + 5.0),
                Pos2::new(title_right, row.top() + ny + 15.0),
            );
            let title_color = mix(
                self.theme.fg,
                self.theme.dim,
                if t.deleted_at > 0 {
                    0.30
                } else if t.done {
                    0.58
                } else {
                    0.0
                },
            );
            let shown_title = display_title(&t.title);
            if editing_title {
                let plate = Rect::from_min_max(
                    Pos2::new(row.left() + TITLE_X - 4.0, row.top() + 3.0),
                    Pos2::new(title_right, row.top() + ny + 14.0),
                );
                p.rect_filled(plate, CornerRadius::same(5), self.theme.bg);
                p.rect_stroke(
                    plate,
                    CornerRadius::same(5),
                    Stroke::new(1.0, rgba(self.theme.accent, 0.38)),
                    egui::StrokeKind::Inside,
                );
                let fg = self.theme.fg;
                let mut finish = None;
                if let Some(e) = self.inline.as_mut() {
                    let edit = egui::TextEdit::singleline(&mut e.title)
                        .id(Id::unique(("inline-title", &e.id)))
                        .font(FontId::new(15.0, FontFamily::Name("bold".into())))
                        .text_color(fg)
                        .align(Align2::LEFT_CENTER)
                        .frame(egui::Frame::NONE)
                        .margin(egui::Margin::symmetric(4, 0))
                        .desired_width((plate.width() - 8.0).max(40.0))
                        .cursor_at_end(true);
                    let resp = ui.place(plate.shrink2(Vec2::new(2.0, 0.0)), edit);
                    if e.focus && !resp.has_focus() {
                        resp.request_focus();
                    }
                    if resp.has_focus() {
                        e.focus = false;
                    }
                    if resp.lost_focus() {
                        let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));
                        finish = Some(!esc);
                    }
                }
                if let Some(save) = finish {
                    inline_finish = Some(save);
                }
            } else {
                let title = fit_text(ui, &shown_title, title_clip.width(), 15.0, true);
                paint_text(
                    &p,
                    Pos2::new(row.left() + TITLE_X, row.top() + ny),
                    &title,
                    15.0,
                    title_color,
                    true,
                    title_clip,
                );
                if title != t.title {
                    resp.clone().on_hover_text(&t.title);
                }
                let tw = text_width(ui, &title, 15.0, true).min(title_clip.width());
                if done_t > 0.01 {
                    let line = Rect::from_min_size(
                        Pos2::new(row.left() + TITLE_X, row.top() + ny),
                        Vec2::new(tw * done_t, 1.4),
                    );
                    p.rect_filled(line, CornerRadius::ZERO, rgba(self.theme.fg, 0.48));
                }
            }

            // Parent chip + tags.
            if m.meta_h > 0.0 {
                let mut mx = row.left() + TITLE_X;
                let meta_y = row.top() + m.meta_y + 9.0;
                let max_x = row.right() - 135.0;
                if t.priority > 0 {
                    let color = priority_color(t.priority, &self.theme);
                    let label = format!("P{}", t.priority);
                    let w = text_width(ui, &label, 10.0, false) + 12.0;
                    let rect = Rect::from_min_size(Pos2::new(mx, meta_y - 8.0), Vec2::new(w, 16.0));
                    ui.painter()
                        .rect_filled(rect, CornerRadius::same(5), rgba(color, 0.12));
                    ui.painter().text(
                        rect.center(),
                        Align2::CENTER_CENTER,
                        label,
                        FontId::new(10.0, FontFamily::Name("bold".into())),
                        color,
                    );
                    mx = rect.right() + 5.0;
                }
                if !t.repeat.is_empty() {
                    let label = format!("↻ {}", t.repeat);
                    let w = text_width(ui, &label, 10.0, false) + 12.0;
                    let rect = Rect::from_min_size(Pos2::new(mx, meta_y - 8.0), Vec2::new(w, 16.0));
                    ui.painter().rect_filled(
                        rect,
                        CornerRadius::same(5),
                        rgba(self.theme.ok, 0.08),
                    );
                    ui.painter().text(
                        rect.left_top() + Vec2::new(6.0, 8.0),
                        Align2::LEFT_CENTER,
                        label,
                        FontId::new(10.0, FontFamily::Monospace),
                        rgba(self.theme.ok, 0.85),
                    );
                    mx = rect.right() + 5.0;
                }
                if !t.parent.is_empty() {
                    let ptitle = titles
                        .get(&t.parent)
                        .cloned()
                        .unwrap_or_else(|| "missing".into());
                    let ptitle = display_title(&ptitle);
                    let label = format!("↳ {}", truncate(&ptitle, 24));
                    let w = text_width(ui, &label, 10.5, false) + 12.0;
                    let rect = Rect::from_min_size(
                        Pos2::new(mx, meta_y - 8.0),
                        Vec2::new(w.min(max_x - mx), 16.0),
                    );
                    if rect.width() > 8.0 {
                        let pr = ui.interact(rect, Id::unique(("parent", &t.id)), Sense::click());
                        ui.painter().rect_filled(
                            rect,
                            CornerRadius::same(5),
                            rgba(self.theme.accent, if pr.hovered() { 0.14 } else { 0.065 }),
                        );
                        ui.painter().text(
                            Pos2::new(rect.left() + 6.0, rect.center().y),
                            Align2::LEFT_CENTER,
                            &label,
                            FontId::new(10.5, FontFamily::Monospace),
                            rgba(self.theme.accent, if pr.hovered() { 1.0 } else { 0.76 }),
                        );
                        if pr.clicked() {
                            parent_jump = Some(t.parent.clone());
                        }
                        mx = rect.right() + 6.0;
                    }
                }
                let tags = t.tags();
                let mut shown = 0;
                for (i, tag) in tags.iter().enumerate() {
                    if mx > max_x - 20.0 {
                        let more = format!("+{}", tags.len() - i);
                        ui.painter().text(
                            Pos2::new(mx, meta_y),
                            Align2::LEFT_CENTER,
                            more,
                            FontId::new(10.0, FontFamily::Monospace),
                            rgba(self.theme.fg, 0.56),
                        );
                        break;
                    }
                    let label = format!("#{tag}");
                    let w = text_width(ui, &label, 10.5, false) + 12.0;
                    let rect = Rect::from_min_size(Pos2::new(mx, meta_y - 8.0), Vec2::new(w, 16.0));
                    let active = self
                        .tag_filter
                        .as_deref()
                        .map(|x| x.eq_ignore_ascii_case(tag))
                        .unwrap_or(false);
                    let tr = ui.interact(rect, Id::unique(("tag", &t.id, tag)), Sense::click());
                    let color = tag_color(tag, &self.theme);
                    ui.painter().rect_filled(
                        rect,
                        CornerRadius::same(5),
                        rgba(
                            color,
                            if active {
                                0.20
                            } else if tr.hovered() {
                                0.14
                            } else {
                                0.075
                            },
                        ),
                    );
                    ui.painter().text(
                        Pos2::new(rect.left() + 6.0, rect.center().y),
                        Align2::LEFT_CENTER,
                        &label,
                        FontId::new(10.5, FontFamily::Monospace),
                        rgba(color, if active || tr.hovered() { 1.0 } else { 0.90 }),
                    );
                    if tr.clicked() {
                        tag_click = Some(tag.clone());
                    }
                    mx = rect.right() + 5.0;
                    shown += 1;
                }
                let _ = shown;
            }

            // Notes: inline editor while editing, otherwise preview/wrapped.
            if editing_notes {
                let plate = Rect::from_min_max(
                    Pos2::new(row.left() + TITLE_X - 4.0, row.top() + m.notes_y - 2.0),
                    Pos2::new(row.right() - 126.0, row.top() + m.notes_y + m.notes_h - 4.0),
                );
                p.rect_filled(plate, CornerRadius::same(6), self.theme.bg);
                p.rect_stroke(
                    plate,
                    CornerRadius::same(6),
                    Stroke::new(1.0, rgba(self.theme.accent, 0.34)),
                    egui::StrokeKind::Inside,
                );
                p.text(
                    Pos2::new(plate.right() - 2.0, plate.bottom() + 9.0),
                    Align2::RIGHT_CENTER,
                    "Ctrl ↵ save  ·  Esc cancel",
                    FontId::new(9.0, FontFamily::Monospace),
                    rgba(self.theme.fg, 0.45),
                );
                let fg = self.theme.fg;
                let mut finish = None;
                if let Some(e) = self.inline.as_mut() {
                    let edit = egui::TextEdit::multiline(&mut e.notes)
                        .id(Id::unique(("inline-notes", &e.id)))
                        .font(FontId::new(12.5, FontFamily::Monospace))
                        .text_color(fg)
                        .frame(egui::Frame::NONE)
                        .margin(egui::Margin::symmetric(6, 4))
                        .desired_width((plate.width() - 12.0).max(60.0))
                        .desired_rows(3)
                        .hint_text("Notes, #tags, links…");
                    let resp = ui.place(plate.shrink2(Vec2::new(2.0, 2.0)), edit);
                    if e.focus && !resp.has_focus() {
                        resp.request_focus();
                    }
                    if resp.has_focus() {
                        e.focus = false;
                    }
                    let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));
                    let save_key =
                        ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Enter));
                    if esc {
                        finish = Some(false);
                    } else if save_key || resp.lost_focus() {
                        finish = Some(true);
                    }
                }
                if let Some(save) = finish {
                    inline_finish = Some(save);
                }
            } else if !t.notes.is_empty() && m.notes_h > 1.0 {
                let notes_clip = Rect::from_min_max(
                    Pos2::new(row.left() + TITLE_X, row.top() + m.notes_y),
                    Pos2::new(row.right() - 130.0, row.top() + m.notes_y + m.notes_h + 2.0),
                );
                if anim > 0.55 {
                    let galley = ui.painter().layout(
                        t.notes.clone(),
                        FontId::new(12.5, FontFamily::Monospace),
                        rgba(self.theme.fg, 0.58),
                        notes_clip.width(),
                    );
                    let text_pos = Pos2::new(notes_clip.left(), notes_clip.top());
                    ui.painter().with_clip_rect(notes_clip).galley(
                        text_pos,
                        galley,
                        rgba(self.theme.fg, 0.58),
                    );
                } else {
                    let preview =
                        fit_text(ui, &one_line(&t.notes), notes_clip.width(), 12.5, false);
                    paint_text(
                        &p,
                        Pos2::new(notes_clip.left(), notes_clip.top() + 8.0),
                        &preview,
                        12.5,
                        rgba(self.theme.fg, 0.60),
                        false,
                        notes_clip,
                    );
                }
            }

            // Expanded actions.
            if m.actions_h > 12.0 && anim > 0.55 {
                let ay = row.top() + m.actions_y + 2.0;
                let mut ax = row.left() + TITLE_X;
                let w = text_width(ui, "Add subtask", 10.5, false) + 18.0;
                let by = Rect::from_min_size(Pos2::new(ax, ay), Vec2::new(w, 22.0));
                if action_button(
                    ui,
                    by,
                    "Add subtask",
                    self.theme.accent2,
                    "Add a child task",
                ) {
                    subtask_id = Some(t.id.clone());
                }
                ax += w + 6.0;
                if !t.parent.is_empty() {
                    let w = text_width(ui, "Unlink", 10.5, false) + 18.0;
                    let r = Rect::from_min_size(Pos2::new(ax, ay), Vec2::new(w, 22.0));
                    if action_button(ui, r, "Unlink", self.theme.dim, "Detach from parent") {
                        unlink_id = Some(t.id.clone());
                    }
                    ax += w + 6.0;
                }
                let label = if t.done { "Reopen" } else { "Mark done" };
                let w = text_width(ui, label, 10.5, false) + 18.0;
                let r = Rect::from_min_size(Pos2::new(ax, ay), Vec2::new(w, 22.0));
                if action_button(ui, r, label, self.theme.ok, "Toggle completion") {
                    mark_id = Some((t.id.clone(), !t.done));
                }
                ax += w + 6.0;
                if self.filter != Filter::Trash {
                    let w = text_width(ui, "Delete", 10.5, false) + 18.0;
                    let r = Rect::from_min_size(Pos2::new(ax, ay), Vec2::new(w, 22.0));
                    if action_button(ui, r, "Delete", self.theme.danger, "Move to trash") {
                        delete_id = Some(t.id.clone());
                    }
                }
            }

            // Right side: age, reminder bell, due chip.
            let mut rx = row.right() - 12.0;
            let age_text = if t.deleted_at > 0 {
                age(t.deleted_at)
            } else {
                age(t.created)
            };
            ui.painter().text(
                Pos2::new(rx, row.top() + ny),
                Align2::RIGHT_CENTER,
                &age_text,
                FontId::new(10.5, FontFamily::Monospace),
                rgba(self.theme.fg, 0.38),
            );
            rx -= text_width(ui, &age_text, 10.5, false) + 8.0;

            if self.filter != Filter::Trash {
                if t.remind_at > 0 {
                    let bell = Rect::from_center_size(
                        Pos2::new(rx - 8.0, row.top() + ny),
                        Vec2::new(18.0, 20.0),
                    );
                    let br = ui.interact(bell, Id::unique(("bell", &t.id)), Sense::click());
                    let col = if t.notified {
                        rgba(self.theme.fg, 0.28)
                    } else if t.remind_at <= now_ms() {
                        self.theme.danger
                    } else {
                        self.theme.accent
                    };
                    ui.painter().text(
                        bell.center(),
                        Align2::CENTER_CENTER,
                        "\u{f0f3}",
                        FontId::new(11.0, FontFamily::Monospace),
                        col,
                    );
                    br.clone().on_hover_text(if t.notified {
                        "Reminder sent"
                    } else {
                        "Reminder pending"
                    });
                    if br.clicked() {
                        due_id = Some(t.id.clone());
                    }
                    rx -= 19.0;
                }
                if t.due > 0 {
                    let overdue = t.due < now_ms();
                    let soon = t.due - now_ms() < 6 * 3_600_000;
                    let label = due_label(t.due);
                    let color = if overdue {
                        self.theme.danger
                    } else if soon {
                        self.theme.accent
                    } else {
                        rgba(self.theme.fg, 0.72)
                    };
                    let w = text_width(ui, &label, 10.5, false) + 12.0;
                    let rect = Rect::from_center_size(
                        Pos2::new(rx - w / 2.0, row.top() + ny),
                        Vec2::new(w, 20.0),
                    );
                    let dr = ui.interact(rect, Id::unique(("due", &t.id)), Sense::click());
                    ui.painter().rect_filled(
                        rect,
                        CornerRadius::same(5),
                        rgba(color, if dr.hovered() { 0.15 } else { 0.065 }),
                    );
                    ui.painter().text(
                        rect.center(),
                        Align2::CENTER_CENTER,
                        &label,
                        FontId::new(10.5, FontFamily::Monospace),
                        color,
                    );
                    if dr.clicked() {
                        due_id = Some(t.id.clone());
                    }
                    rx -= w + 6.0;
                } else if hovered {
                    let rect = Rect::from_center_size(
                        Pos2::new(rx - 13.0, row.top() + ny),
                        Vec2::new(26.0, 20.0),
                    );
                    let dr = ui.interact(rect, Id::unique(("adddue", &t.id)), Sense::click());
                    ui.painter().text(
                        rect.center(),
                        Align2::CENTER_CENTER,
                        "\u{f017}",
                        FontId::new(11.0, FontFamily::Monospace),
                        rgba(self.theme.fg, if dr.hovered() { 0.82 } else { 0.42 }),
                    );
                    dr.clone().on_hover_text("Set due date");
                    if dr.clicked() {
                        due_id = Some(t.id.clone());
                    }
                    rx -= 30.0;
                }
            } else {
                let label = "Restore";
                let w = text_width(ui, label, 10.5, false) + 12.0;
                let rect = Rect::from_center_size(
                    Pos2::new(rx - w / 2.0, row.top() + ny),
                    Vec2::new(w, 20.0),
                );
                if chip_rect(
                    ui,
                    rect,
                    label,
                    self.theme.ok,
                    Id::unique(("restore", &t.id)),
                ) {
                    restore_id = Some(t.id.clone());
                }
                rx -= w + 6.0;
                let label = "Delete forever";
                let w = text_width(ui, label, 10.5, false) + 12.0;
                let rect = Rect::from_center_size(
                    Pos2::new(rx - w / 2.0, row.top() + ny),
                    Vec2::new(w, 20.0),
                );
                if chip_rect(
                    ui,
                    rect,
                    label,
                    self.theme.danger,
                    Id::unique(("purge", &t.id)),
                ) {
                    purge_id = Some(t.id.clone());
                }
                rx -= w + 6.0;
            }

            // Hover affordances (drag / delete / edit / link / menu).
            let actions_visible = self.filter != Filter::Trash
                && (hovered
                    || selected
                    || self.drag_id.as_deref() == Some(t.id.as_str())
                    || self.link_from.as_deref() == Some(t.id.as_str())
                    || self.link_drag.as_deref() == Some(t.id.as_str()));
            let mut more_response = None;
            if actions_visible && !row_compact {
                let by = row.top() + 39.0;
                let mut bx = row.right() - 12.0;
                let drag = Rect::from_center_size(Pos2::new(bx - 8.0, by), Vec2::new(18.0, 22.0));
                let hr = ui.interact(drag, Id::unique(("drag", &t.id)), Sense::click_and_drag());
                if hr.hovered() {
                    ui.painter().rect_filled(
                        drag,
                        CornerRadius::same(5),
                        rgba(self.theme.fg, 0.055),
                    );
                }
                ui.painter().text(
                    drag.center(),
                    Align2::CENTER_CENTER,
                    "⠿",
                    FontId::new(10.0, FontFamily::Monospace),
                    rgba(self.theme.fg, if hr.hovered() { 0.7 } else { 0.26 }),
                );
                hr.clone().on_hover_text("Drag to reorder");
                if hr.drag_started() {
                    self.drag_id = Some(t.id.clone());
                    self.drag_y = hr.interact_pointer_pos().map(|p| p.y).unwrap_or(row.top());
                }
                if hr.dragged() {
                    self.drag_y = ui
                        .ctx()
                        .input(|i| i.pointer.interact_pos())
                        .map(|p| p.y)
                        .unwrap_or(self.drag_y);
                }
                if hr.drag_stopped() {
                    if let Some(did) = self.drag_id.clone() {
                        let idx = positions
                            .iter()
                            .position(|(_, rr)| self.drag_y < rr.center().y);
                        if let Some((target_id, target_rect)) =
                            idx.and_then(|i| positions.get(i)).cloned()
                        {
                            reorder = Some((did, target_id, false, target_rect.top()));
                        } else if let Some((target_id, target_rect)) = positions.last().cloned() {
                            reorder = Some((did, target_id, true, target_rect.bottom()));
                        }
                    }
                    self.drag_id = None;
                    self.drag_y = -1.0;
                }
                bx -= 26.0;
                let del = Rect::from_center_size(Pos2::new(bx - 7.0, by), Vec2::new(18.0, 22.0));
                let dr = ui.interact(del, Id::unique(("del", &t.id)), Sense::click());
                if dr.hovered() {
                    ui.painter().rect_filled(
                        del,
                        CornerRadius::same(5),
                        rgba(self.theme.danger, 0.09),
                    );
                }
                ui.painter().text(
                    del.center(),
                    Align2::CENTER_CENTER,
                    "✕",
                    FontId::new(10.5, FontFamily::Monospace),
                    rgba(self.theme.danger, if dr.hovered() { 1.0 } else { 0.5 }),
                );
                dr.clone().on_hover_text("Move to trash");
                if dr.clicked() {
                    delete_id = Some(t.id.clone());
                }
                bx -= 24.0;
                let ed = Rect::from_center_size(Pos2::new(bx - 7.0, by), Vec2::new(18.0, 22.0));
                let er = ui.interact(ed, Id::unique(("edit", &t.id)), Sense::click());
                if er.hovered() {
                    ui.painter()
                        .rect_filled(ed, CornerRadius::same(5), rgba(self.theme.fg, 0.055));
                }
                ui.painter().text(
                    ed.center(),
                    Align2::CENTER_CENTER,
                    "✎",
                    FontId::new(10.5, FontFamily::Monospace),
                    rgba(self.theme.fg, if er.hovered() { 0.9 } else { 0.38 }),
                );
                er.clone().on_hover_text("Edit title");
                if er.clicked() {
                    inline_start = Some((t.id.clone(), InlineField::Title));
                }
                bx -= 24.0;
                let lk = Rect::from_center_size(Pos2::new(bx - 7.0, by), Vec2::new(18.0, 22.0));
                let lr = ui.interact(lk, Id::unique(("link", &t.id)), Sense::click());
                let active = self.link_from.as_deref() == Some(t.id.as_str());
                if lr.hovered() || active {
                    ui.painter().rect_filled(
                        lk,
                        CornerRadius::same(5),
                        rgba(self.theme.accent2, if active { 0.13 } else { 0.07 }),
                    );
                }
                ui.painter().text(
                    lk.center(),
                    Align2::CENTER_CENTER,
                    "\u{f0c1}",
                    FontId::new(11.0, FontFamily::Monospace),
                    rgba(
                        self.theme.accent2,
                        if active {
                            1.0
                        } else if lr.hovered() {
                            0.9
                        } else {
                            0.38
                        },
                    ),
                );
                lr.clone()
                    .on_hover_text("Link: click a target task (or drag the node)");
                if lr.clicked() {
                    link_start = Some(t.id.clone());
                }
                bx -= 24.0;
                let more = Rect::from_center_size(Pos2::new(bx - 7.0, by), Vec2::new(18.0, 22.0));
                more_response = Some(menu_button(ui, more, Id::unique(("more", &t.id))));
            } else if actions_visible && row_compact {
                let more = Rect::from_center_size(
                    Pos2::new(row.left() + TITLE_X - 14.0, row.top() + ny),
                    Vec2::new(20.0, 20.0),
                );
                more_response = Some(menu_button(ui, more, Id::unique(("more", &t.id))));
            }

            if let Some(mr) = more_response {
                if mr.clicked() {
                    self.selected = Some(t.id.clone());
                    self.kbd_nav = false;
                }
                let id = t.id.clone();
                let done = t.done;
                let danger = self.theme.danger;
                egui::Popup::menu(&mr)
                    .align(egui::RectAlign::BOTTOM_END)
                    .gap(4.0)
                    .width(176.0)
                    .show(|ui| {
                        ui.spacing_mut().item_spacing.y = 1.0;
                        if ui.button("Edit title").clicked() {
                            inline_start = Some((id.clone(), InlineField::Title));
                        }
                        if ui.button("Edit notes").clicked() {
                            inline_start = Some((id.clone(), InlineField::Notes));
                        }
                        if ui.button("Open full editor").clicked() {
                            edit_id = Some(id.clone());
                        }
                        if ui.button("Set due date").clicked() {
                            due_id = Some(id.clone());
                        }
                        if ui.button("Add subtask").clicked() {
                            subtask_id = Some(id.clone());
                        }
                        if ui.button("Link to task").clicked() {
                            link_start = Some(id.clone());
                        }
                        if ui.button("Duplicate").clicked() {
                            duplicate_id = Some(id.clone());
                        }
                        if ui.button("Duplicate with subtasks").clicked() {
                            duplicate_subtree_id = Some(id.clone());
                        }
                        ui.menu_button("Priority", |ui| {
                            for (label, value) in [
                                ("None", 0u8),
                                ("P1  High", 1),
                                ("P2  Medium", 2),
                                ("P3  Low", 3),
                            ] {
                                if ui.selectable_label(t.priority == value, label).clicked() {
                                    priority_set = Some((id.clone(), value));
                                    ui.close();
                                }
                            }
                        });
                        ui.separator();
                        if ui
                            .button(if done { "Reopen" } else { "Mark done" })
                            .clicked()
                        {
                            mark_id = Some((id.clone(), !done));
                        }
                        if ui
                            .button(
                                egui::RichText::new("Move to trash")
                                    .size(12.0)
                                    .color(danger),
                            )
                            .clicked()
                        {
                            delete_id = Some(id.clone());
                        }
                    });
            }
            let _ = rx;
        }

        // Auto-scroll while dragging near the list edges.
        if self.drag_id.is_some() || self.link_drag.is_some() {
            if let Some(ptr) = ui.ctx().input(|i| i.pointer.interact_pos()) {
                let view = ui.clip_rect();
                const EDGE: f32 = 34.0;
                const SPEED: f32 = 900.0;
                let dt = ui.ctx().input(|i| i.stable_dt).min(0.05);
                let mut dy = 0.0;
                if ptr.y > view.bottom() - EDGE {
                    dy = -SPEED * dt * ((ptr.y - (view.bottom() - EDGE)) / EDGE).clamp(0.0, 1.0);
                } else if ptr.y < view.top() + EDGE {
                    dy = SPEED * dt * (((view.top() + EDGE) - ptr.y) / EDGE).clamp(0.0, 1.0);
                }
                if dy != 0.0 {
                    ui.scroll_with_delta(Vec2::new(0.0, dy));
                    ui.ctx().request_repaint();
                }
            }
        }

        if self.drag_id.is_some() && self.drag_y > 0.0 {
            let p = ui.painter();
            // Ghost of the row following the pointer.
            if let (Some(id), Some(ptr), Some(src)) = (
                self.drag_id.clone(),
                ui.ctx().input(|i| i.pointer.interact_pos()),
                self.drag_id.as_deref().and_then(pos_of),
            ) {
                let ghost = Rect::from_min_size(
                    Pos2::new(content_rect.left() + 4.0, ptr.y - src.height() * 0.5),
                    Vec2::new(width - 8.0, src.height()),
                );
                p.rect_filled(ghost, CornerRadius::same(8), rgba(self.theme.surface, 0.94));
                p.rect_stroke(
                    ghost,
                    CornerRadius::same(8),
                    Stroke::new(1.0, rgba(self.theme.accent, 0.5)),
                    egui::StrokeKind::Inside,
                );
                if let Some(title) = titles.get(&id) {
                    let title = fit_text(
                        ui,
                        &display_title(title),
                        (ghost.width() - TITLE_X - 20.0).max(40.0),
                        15.0,
                        true,
                    );
                    let ghost_y = if ghost.height() <= 44.0 { 16.0 } else { NODE_Y };
                    paint_text(
                        p,
                        Pos2::new(ghost.left() + TITLE_X, ghost.top() + ghost_y),
                        &title,
                        15.0,
                        self.theme.fg,
                        true,
                        ghost,
                    );
                }
            }
            let y = self.drag_y.clamp(content_rect.top(), content_rect.bottom());
            p.rect_filled(
                Rect::from_min_size(
                    Pos2::new(content_rect.left() + TITLE_X, y - 1.0),
                    Vec2::new(width - TITLE_X - 20.0, 2.0),
                ),
                CornerRadius::same(1),
                rgba(self.theme.accent, 0.85),
            );
            p.circle_filled(
                Pos2::new(content_rect.left() + TITLE_X - 6.0, y),
                3.0,
                rgba(self.theme.accent, 0.9),
            );
            ui.ctx().request_repaint();
        }

        // Link line (node drag or click-to-link mode).
        if let Some(from) = self.link_from.clone().or_else(|| self.link_drag.clone()) {
            if let Some(r) = pos_of(&from) {
                if let Some(ptr) = ui.ctx().input(|i| i.pointer.interact_pos()) {
                    let start = Pos2::new(lane_x(self.lane(&from)), node_y(r));
                    let p = ui.painter();
                    let mut a = start;
                    let steps = 18;
                    for k in 1..=steps {
                        let t = k as f32 / steps as f32;
                        let b = start + (ptr - start) * t;
                        if k % 2 == 0 {
                            p.line_segment([a, b], Stroke::new(1.5, self.theme.accent2));
                        }
                        a = b;
                    }
                    p.circle_filled(ptr, 3.0, self.theme.accent2);
                }
            }
        }

        // Empty states.
        if tasks.is_empty() {
            let c = content_rect.center();
            let (open, done, _) = self.counts();
            let any_tasks = open + done > 0;
            let (title, sub): (String, String) = if !self.collapsed_sections.is_empty() {
                (
                    "Sections collapsed".into(),
                    "Use the chips above to expand a section.".into(),
                )
            } else if self.filter == Filter::Trash {
                (
                    "Trash is empty".into(),
                    "Deleted tasks wait here until you remove them for good.".into(),
                )
            } else if !self.search.is_empty() || self.tag_filter.is_some() {
                (
                    "Nothing matches".into(),
                    if let Some(tag) = &self.tag_filter {
                        format!("No tasks tagged #{tag} right now.")
                    } else {
                        format!("No tasks contain “{}”.", truncate(&self.search, 24))
                    },
                )
            } else if self.view == SmartView::Today && self.filter != Filter::Done {
                (
                    "Nothing due today".into(),
                    "Overdue and today's tasks will show up here.".into(),
                )
            } else if self.view == SmartView::Upcoming && self.filter != Filter::Done {
                (
                    "Nothing coming up".into(),
                    "Tasks due in the next week land here.".into(),
                )
            } else if self.view == SmartView::Overdue && self.filter != Filter::Done {
                ("Nothing overdue".into(), "You're on top of it.".into())
            } else if self.view == SmartView::NoDate && self.filter != Filter::Done {
                (
                    "Everything has a date".into(),
                    "Unscheduled tasks will show up here.".into(),
                )
            } else if self.filter == Filter::Open && any_tasks {
                (
                    "All caught up".into(),
                    "Every task is done. Add another when you're ready.".into(),
                )
            } else if self.filter == Filter::Done {
                (
                    "Nothing completed yet".into(),
                    "Finished tasks will collect here.".into(),
                )
            } else {
                (
                    "No tasks yet".into(),
                    "Add one below — use #tags to group it, / to search.".into(),
                )
            };
            let g = Pos2::new(c.x, c.y - 66.0);
            let p = ui.painter();
            if self.filter == Filter::Trash {
                p.text(
                    g,
                    Align2::CENTER_CENTER,
                    "\u{f1f8}",
                    FontId::new(20.0, FontFamily::Monospace),
                    rgba(self.theme.danger, 0.35),
                );
            } else if (self.filter == Filter::Open && any_tasks) || self.filter == Filter::Done {
                p.circle_stroke(g, 9.0, Stroke::new(1.2, rgba(self.theme.ok, 0.45)));
                p.text(
                    g,
                    Align2::CENTER_CENTER,
                    "✓",
                    FontId::new(11.0, FontFamily::Name("bold".into())),
                    rgba(self.theme.ok, 0.7),
                );
            } else {
                let faint = rgba(self.theme.fg, 0.13);
                p.line_segment(
                    [Pos2::new(g.x, g.y), Pos2::new(g.x, g.y + 24.0)],
                    Stroke::new(1.0, faint),
                );
                p.line_segment(
                    [
                        Pos2::new(g.x - 17.0, g.y + 24.0),
                        Pos2::new(g.x + 17.0, g.y + 24.0),
                    ],
                    Stroke::new(1.0, faint),
                );
                p.circle_stroke(g, 3.5, Stroke::new(1.2, rgba(self.theme.accent, 0.45)));
                p.circle_filled(Pos2::new(g.x - 17.0, g.y + 24.0), 2.3, faint);
                p.circle_filled(Pos2::new(g.x + 17.0, g.y + 24.0), 2.3, faint);
            }
            p.text(
                Pos2::new(c.x, c.y - 12.0),
                Align2::CENTER_CENTER,
                title,
                FontId::new(14.0, FontFamily::Name("bold".into())),
                rgba(self.theme.fg, 0.68),
            );
            p.text(
                Pos2::new(c.x, c.y + 10.0),
                Align2::CENTER_CENTER,
                sub,
                FontId::new(10.5, FontFamily::Monospace),
                rgba(self.theme.fg, 0.60),
            );
            if !self.search.is_empty() || self.tag_filter.is_some() {
                let w = 106.0;
                let rect = Rect::from_center_size(Pos2::new(c.x, c.y + 42.0), Vec2::new(w, 24.0));
                if chip_rect(
                    ui,
                    rect,
                    "Clear filters",
                    self.theme.accent,
                    Id::unique("clear-empty"),
                ) {
                    clear_filters = true;
                }
            }
        }

        // Apply row actions after all immutable painting is done.
        if let Some((from, to)) = link_now {
            self.link(&from, &to);
            self.link_from = None;
            self.link_drag = None;
        }
        if let Some(id) = toggle_id {
            self.toggle(&id);
        }
        if let Some(id) = delete_id {
            self.soft_delete(&id);
        }
        if let Some(id) = restore_id {
            self.restore(&id);
        }
        if let Some(id) = purge_id {
            self.purge(&id);
        }
        if let Some(id) = due_id {
            let due = self.task(&id).map(|t| t.due).unwrap_or(0);
            let base = if due > 0 { due } else { now_ms() };
            let anchor = positions.iter().find(|(i, _)| i == &id).map(|(_, r)| {
                let view = ui.ctx().content_rect();
                let max_x = (view.right() - 346.0).max(view.left() + 12.0);
                let max_y = (view.bottom() - 535.0).max(view.top() + 8.0);
                Pos2::new(
                    (r.right() - 340.0).clamp(view.left() + 12.0, max_x),
                    (r.bottom() + 6.0).clamp(view.top() + 12.0, max_y),
                )
            });
            self.due_anchor = anchor;
            let (repeat, lead) = self
                .task(&id)
                .map(|t| (t.repeat.clone(), model::task_lead(t)))
                .unwrap_or_default();
            self.due_edit = Some(DueEdit {
                id,
                draft: if due > 0 {
                    chrono::Local
                        .timestamp_millis_opt(due)
                        .single()
                        .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
                        .unwrap_or_default()
                } else {
                    String::new()
                },
                month: calendar::month_of(base),
                repeat,
                lead,
            });
            self.due_focus = true;
        }
        if let Some((id, title, notes)) = edit_id.and_then(|id| {
            self.task(&id)
                .map(|t| (id.clone(), t.title.clone(), t.notes.clone()))
        }) {
            self.edit = Some((id, title, notes));
            self.edit_focus = true;
        }
        if let Some(save) = inline_finish {
            if let Some(e) = self.inline.take() {
                if save {
                    match e.field {
                        InlineField::Title => {
                            let clean = e.title.trim().to_string();
                            if !clean.is_empty() && self.task(&e.id).is_some() {
                                self.snapshot();
                                if let Some(t) = self.task_mut(&e.id) {
                                    t.title = clean;
                                }
                                self.dirty = true;
                            }
                        }
                        InlineField::Notes => {
                            if self.task(&e.id).is_some() {
                                self.snapshot();
                                if let Some(t) = self.task_mut(&e.id) {
                                    t.notes = e.notes.trim_end().to_string();
                                }
                                self.dirty = true;
                            }
                        }
                    }
                }
            }
        }
        if let Some((id, field)) = inline_start {
            self.start_inline(&id, field);
        }
        if let Some(id) = duplicate_id {
            self.duplicate(&id);
        }
        if let Some(id) = duplicate_subtree_id {
            self.duplicate_with_subtasks(&id);
        }
        if let Some((id, priority)) = priority_set {
            self.set_priority(&id, priority);
        }
        if let Some(id) = link_start {
            if self.link_from.as_deref() == Some(id.as_str()) {
                self.link_from = None;
                self.toast("Link cancelled");
            } else {
                self.link_from = Some(id);
                self.toast("Click another task to link under it");
            }
        }
        if let Some(tag) = tag_click {
            self.set_tag_filter(&tag);
        }
        if let Some(id) = parent_jump {
            self.jump_to(&id);
        }
        if let Some(id) = subtask_id {
            self.input_parent = Some(id.clone());
            self.focus_input = true;
            if let Some(t) = self.task_mut(&id) {
                t.expanded = true;
            }
            self.toast("Type the subtask title and press ↵");
        }
        if let Some(id) = unlink_id {
            self.unlink(&id);
        }
        if let Some((id, done)) = mark_id {
            self.set_done_many(&[id], done);
        }
        if let Some((id, target, after, _y)) = reorder {
            self.move_relative(&id, &target, after);
        }
        if let Some(id) = expand_id {
            if let Some(t) = self.task_mut(&id) {
                t.expanded = !t.expanded;
                self.dirty = true;
            }
        }
        if clear_filters {
            self.search.clear();
            self.tag_filter = None;
            self.input.clear();
        }

        // Node-drag link release (click-to-link mode is handled above).
        if self.link_drag.is_some() && ui.ctx().input(|i| i.pointer.any_released()) {
            if let Some(from) = self.link_drag.take() {
                if let Some(ptr) = ui.ctx().input(|i| i.pointer.interact_pos()) {
                    if let Some((to, _)) = positions.iter().find(|(_, r)| r.contains(ptr)) {
                        if &from != to {
                            self.link(&from, to);
                        }
                    }
                }
            }
        }

        if let Some(id) = self.scroll_to.clone() {
            if let Some(r) = positions.iter().find(|(i, _)| i == &id).map(|(_, r)| *r) {
                ui.scroll_to_rect(r, Some(Align::Center));
                self.scroll_to = None;
            }
        }

        // Keep collapse animations smooth.
        if anims.iter().any(|a| *a > 0.001 && *a < 0.999) {
            ui.ctx().request_repaint_after(Duration::from_millis(16));
        }
        self.done_flash
            .retain(|_, at| at.elapsed() < Duration::from_millis(700));
        if !self.done_flash.is_empty() {
            ui.ctx().request_repaint_after(Duration::from_millis(16));
        }
    }

    fn ui_batch(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        let trash = self.filter == Filter::Trash;
        egui::Frame::NONE
            .fill(self.theme.surface)
            .stroke(Stroke::new(1.0, rgba(self.theme.fg, 0.05)))
            .corner_radius(CornerRadius::same(8))
            .inner_margin(egui::Margin::symmetric(12, 9))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    ui.label(
                        egui::RichText::new(format!("{} selected", self.multi.len()))
                            .font(FontId::new(12.0, FontFamily::Name("bold".into())))
                            .color(self.theme.accent),
                    );
                    ui.add_space(4.0);
                    if trash {
                        if chip_button(
                            ui,
                            "Restore",
                            self.theme.ok,
                            rgba(self.theme.ok, 0.09),
                            "Restore selected tasks",
                        ) {
                            let ids: Vec<String> = self.multi.iter().cloned().collect();
                            self.restore_many(&ids);
                        }
                        if chip_button(
                            ui,
                            "Delete forever",
                            self.theme.danger,
                            rgba(self.theme.danger, 0.08),
                            "Permanently delete selected tasks",
                        ) {
                            let ids: Vec<String> = self.multi.iter().cloned().collect();
                            self.purge_many(&ids);
                        }
                    } else {
                        if chip_button(
                            ui,
                            "Mark done",
                            self.theme.ok,
                            rgba(self.theme.ok, 0.09),
                            "Complete selected",
                        ) {
                            let ids: Vec<String> = self.multi.iter().cloned().collect();
                            self.set_done_many(&ids, true);
                            self.multi.clear();
                        }
                        if chip_button(
                            ui,
                            "Reopen",
                            self.theme.fg,
                            rgba(self.theme.fg, 0.06),
                            "Reopen selected",
                        ) {
                            let ids: Vec<String> = self.multi.iter().cloned().collect();
                            self.set_done_many(&ids, false);
                            self.multi.clear();
                        }
                        if chip_button(
                            ui,
                            "Move to trash",
                            self.theme.danger,
                            rgba(self.theme.danger, 0.08),
                            "Move selected to trash",
                        ) {
                            let ids: Vec<String> = self.multi.iter().cloned().collect();
                            self.delete_many(&ids);
                        }
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new("Clear  ✕")
                                        .size(10.5)
                                        .color(rgba(self.theme.fg, 0.55)),
                                )
                                .frame(false),
                            )
                            .clicked()
                        {
                            self.multi.clear();
                        }
                    });
                });
                ui.add_space(2.0);
                ui.label(
                    egui::RichText::new("ctrl click to add  ·  esc to clear")
                        .size(10.0)
                        .color(rgba(self.theme.fg, 0.56)),
                );
            });
    }

    fn ui_input(&mut self, ui: &mut egui::Ui) {
        ui.add_space(7.0);
        let (line, _) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), Sense::hover());
        ui.painter()
            .rect_filled(line, CornerRadius::ZERO, rgba(self.theme.fg, 0.055));
        ui.add_space(9.0);
        let mut enter = false;
        let accent = if self.input.starts_with('/') {
            self.theme.accent2
        } else {
            self.theme.accent
        };
        let mut focused = false;
        let frame = egui::Frame::NONE
            .fill(self.theme.surface)
            .stroke(Stroke::new(1.0, rgba(accent, 0.10)))
            .corner_radius(CornerRadius::same(8))
            .inner_margin(egui::Margin::symmetric(10, 8))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let (glyph, _) = ui.allocate_exact_size(Vec2::new(18.0, 24.0), Sense::hover());
                    let (mark, mark_color) = if self.read_only {
                        ("!", self.theme.danger)
                    } else if self.input.starts_with('/') {
                        ("/", self.theme.accent2)
                    } else {
                        ("+", self.theme.accent)
                    };
                    ui.painter().text(
                        glyph.center(),
                        Align2::CENTER_CENTER,
                        mark,
                        FontId::new(16.0, FontFamily::Name("bold".into())),
                        mark_color,
                    );
                    let hint = if self.read_only {
                        "State file is read-only — fix it, then press Retry".to_string()
                    } else if let Some(parent) = &self.input_parent {
                        let title = self
                            .task(parent)
                            .map(|t| truncate(&t.title, 26))
                            .unwrap_or_default();
                        format!("Subtask of “{title}”…")
                    } else if self.input.starts_with('/') {
                        "Filter tasks…".to_string()
                    } else {
                        "Add a task…  !p1 · tomorrow 18:00".to_string()
                    };
                    let edit = egui::TextEdit::singleline(&mut self.input)
                        .id(Id::unique("task-input"))
                        .hint_text(hint)
                        .font(FontId::new(14.0, FontFamily::Monospace))
                        .text_color(self.theme.fg)
                        .frame(egui::Frame::NONE)
                        .desired_width(ui.available_width() - 30.0)
                        .interactive(!self.read_only);
                    let resp = ui.add_sized([ui.available_width() - 30.0, 24.0], edit);
                    focused = resp.has_focus();
                    let (key, _) = ui.allocate_exact_size(Vec2::new(22.0, 20.0), Sense::hover());
                    ui.painter().rect_filled(
                        key,
                        CornerRadius::same(4),
                        rgba(self.theme.fg, 0.045),
                    );
                    ui.painter().rect_stroke(
                        key,
                        CornerRadius::same(4),
                        Stroke::new(1.0, rgba(self.theme.fg, 0.075)),
                        egui::StrokeKind::Inside,
                    );
                    ui.painter().text(
                        key.center(),
                        Align2::CENTER_CENTER,
                        "↵",
                        FontId::new(10.0, FontFamily::Monospace),
                        rgba(self.theme.fg, 0.42),
                    );
                    if (self.first_frame || self.focus_input) && !self.hidden && !self.read_only {
                        resp.request_focus();
                        self.first_frame = false;
                        self.focus_input = false;
                    }
                    enter = ui.input(|i| i.key_pressed(egui::Key::Enter))
                        && (resp.has_focus() || resp.lost_focus());
                    if self.input.starts_with('/') {
                        self.search = self.input.trim_start_matches('/').trim().to_string();
                    }
                });
            });
        if focused {
            ui.painter().rect_stroke(
                frame.response.rect,
                CornerRadius::same(8),
                Stroke::new(1.0, rgba(accent, 0.38)),
                egui::StrokeKind::Inside,
            );
        }
        if enter {
            if let Some(q) = self.input.strip_prefix('/') {
                self.search = q.trim().to_string();
                self.input.clear();
            } else if !self.input.trim().is_empty() && !self.read_only {
                let text = self.input.clone();
                let parent = self.input_parent.take().unwrap_or_default();
                self.input.clear();
                let quick = parse_quick(&text);
                self.add_parsed(&quick.title, &parent, quick.priority, quick.due);
                self.focus_input = true;
            }
        }
        ui.add_space(7.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            if !self.input.is_empty() && !self.input.starts_with('/') {
                let quick = parse_quick(&self.input);
                ui.label(
                    egui::RichText::new("↵ add")
                        .size(10.0)
                        .color(rgba(self.theme.fg, 0.55)),
                );
                if quick.priority > 0 {
                    ui.label(
                        egui::RichText::new(format!("P{}", quick.priority))
                            .size(10.0)
                            .color(priority_color(quick.priority, &self.theme)),
                    );
                }
                for tag in model::parse_tags(&quick.title) {
                    ui.label(
                        egui::RichText::new(format!("#{tag}"))
                            .size(10.0)
                            .color(tag_color(&tag, &self.theme)),
                    );
                }
                if let Some(due) = quick.due {
                    ui.label(
                        egui::RichText::new(format!("due {}", due_label(due)))
                            .size(10.0)
                            .color(self.theme.accent),
                    );
                }
            } else {
                let hint = if self.link_from.is_some() {
                    "Click a task to link under it — or drag a node  ·  Esc cancels".to_string()
                } else if let Some(parent) = &self.input_parent {
                    let title = self
                        .task(parent)
                        .map(|t| truncate(&t.title, 24))
                        .unwrap_or_default();
                    format!("Subtask of “{title}”  ·  Esc cancels")
                } else if self.search.is_empty() && self.tag_filter.is_none() {
                    "↑↓ move  ·  Space done  ·  Enter edit  ·  M menu  ·  D due  ·  Esc hide"
                        .to_string()
                } else {
                    "Showing filtered results  ·  Esc clears".to_string()
                };
                ui.label(
                    egui::RichText::new(hint)
                        .size(10.0)
                        .color(rgba(self.theme.fg, 0.60)),
                );
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if !self.search.is_empty() || self.tag_filter.is_some() {
                    if ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new("Clear filter  ✕")
                                    .size(10.0)
                                    .color(rgba(self.theme.fg, 0.66)),
                            )
                            .frame(false),
                        )
                        .clicked()
                    {
                        self.search.clear();
                        self.tag_filter = None;
                        if self.input.starts_with('/') {
                            self.input.clear();
                        }
                    }
                }
            });
        });
    }

    fn ui_edit_window(&mut self, ctx: &egui::Context) {
        let Some((id, title, notes)) = self.edit.clone() else {
            return;
        };
        let mut open = true;
        let mut title = title;
        let mut notes = notes;
        let mut save = false;
        let mut cancel = false;
        egui::Window::new("Edit task")
            .collapsible(false)
            .resizable(false)
            .default_width(440.0)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(
                    egui::RichText::new("Title")
                        .size(11.0)
                        .color(self.theme.dim),
                );
                let tr = ui.add_sized(
                    [ui.available_width(), 30.0],
                    egui::TextEdit::singleline(&mut title)
                        .font(FontId::new(14.0, FontFamily::Monospace)),
                );
                if self.edit_focus {
                    tr.request_focus();
                    self.edit_focus = false;
                }
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new("Notes  ·  #tags are parsed automatically")
                        .size(11.0)
                        .color(self.theme.dim),
                );
                ui.add_sized(
                    [ui.available_width(), 108.0],
                    egui::TextEdit::multiline(&mut notes)
                        .font(FontId::new(12.5, FontFamily::Monospace)),
                );
                let tags = model::parse_tags(&format!("{title} {notes}"));
                if !tags.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 5.0;
                        for tag in tags {
                            ui.label(
                                egui::RichText::new(format!("#{tag}"))
                                    .size(10.5)
                                    .color(tag_color(&tag, &self.theme)),
                            );
                        }
                    });
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new("Save")
                                    .size(12.0)
                                    .color(self.theme.accent),
                            )
                            .fill(rgba(self.theme.accent, 0.12))
                            .stroke(Stroke::new(1.0, rgba(self.theme.accent, 0.22)))
                            .corner_radius(CornerRadius::same(6))
                            .min_size(Vec2::new(78.0, 28.0)),
                        )
                        .clicked()
                    {
                        save = true;
                    }
                    if ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new("Cancel")
                                    .size(12.0)
                                    .color(rgba(self.theme.fg, 0.55)),
                            )
                            .frame(false)
                            .min_size(Vec2::new(64.0, 28.0)),
                        )
                        .clicked()
                    {
                        cancel = true;
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new("Ctrl ↵ save  ·  Esc cancel")
                                .size(10.0)
                                .color(rgba(self.theme.fg, 0.56)),
                        );
                    });
                });
                if ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Enter)) {
                    save = true;
                }
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    cancel = true;
                }
            });
        if !open || cancel {
            self.edit = None;
            return;
        }
        if save {
            let clean = title.trim().to_string();
            if !clean.is_empty() {
                self.snapshot();
                if let Some(t) = self.task_mut(&id) {
                    t.title = clean;
                    t.notes = notes;
                }
                self.dirty = true;
                self.toast("Saved");
            }
            self.edit = None;
        } else if let Some((_, t, n)) = self.edit.as_mut() {
            *t = title;
            *n = notes;
        }
    }

    fn ui_due_window(&mut self, ctx: &egui::Context) {
        let Some(due) = self.due_edit.clone() else {
            return;
        };
        let id = due.id.clone();
        let mut draft = due.draft.clone();
        let mut month = due.month;
        let mut repeat = due.repeat.clone();
        let mut lead = if due.lead > 0 { due.lead } else { 600_000 };
        let mut open = true;
        let mut apply: Option<Option<i64>> = None;
        let title = self.task(&id).map(|t| t.title.clone()).unwrap_or_default();
        let mut win = egui::Window::new("Due date")
            .collapsible(false)
            .resizable(false)
            .default_width(334.0)
            .open(&mut open);
        if let Some(pos) = self.due_anchor {
            win = win.fixed_pos(pos);
        }
        win.show(ctx, |ui| {
            ui.set_min_width(300.0);
            ui.label(
                egui::RichText::new(truncate(&title, 44))
                    .size(11.5)
                    .color(self.theme.dim),
            );
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                for (label, val) in [
                    ("Today", "today 18:00"),
                    ("Tomorrow", "tomorrow 09:00"),
                    ("+1h", "+1h"),
                    ("+3d", "+3d"),
                ] {
                    if chip_button(
                        ui,
                        label,
                        self.theme.fg,
                        rgba(self.theme.fg, 0.055),
                        "Set due date",
                    ) {
                        draft = val.to_string();
                        if let Some(ms) = parse_when(&draft) {
                            month = calendar::month_of(ms);
                        }
                    }
                }
            });
            ui.add_space(7.0);

            // Month navigation.
            let (nav, _) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), 26.0), Sense::hover());
            let prev = Rect::from_min_size(nav.min, Vec2::new(26.0, 26.0));
            let next = Rect::from_min_size(
                Pos2::new(nav.right() - 26.0, nav.top()),
                Vec2::new(26.0, 26.0),
            );
            if icon_rect(ui, prev, "‹", self.theme.dim, Id::unique("due-prev")) {
                month = calendar::shift_month(month.0, month.1, -1);
            }
            if icon_rect(ui, next, "›", self.theme.dim, Id::unique("due-next")) {
                month = calendar::shift_month(month.0, month.1, 1);
            }
            ui.painter().text(
                nav.center(),
                Align2::CENTER_CENTER,
                calendar::month_label(month.0, month.1),
                FontId::new(12.0, FontFamily::Name("bold".into())),
                self.theme.fg,
            );

            // Weekdays and day grid.
            let (cal, _) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), 186.0), Sense::hover());
            let grid_w = 7.0 * 36.0;
            let sx = cal.center().x - grid_w / 2.0;
            for (i, day) in calendar::WEEKDAYS.iter().enumerate() {
                ui.painter().text(
                    Pos2::new(sx + i as f32 * 36.0 + 18.0, cal.top() + 8.0),
                    Align2::CENTER_CENTER,
                    day,
                    FontId::new(10.0, FontFamily::Monospace),
                    rgba(self.theme.fg, 0.55),
                );
            }
            let parsed = parse_when(&draft);
            let selected_date = parsed
                .and_then(|ms| Local.timestamp_millis_opt(ms).single())
                .map(|d| d.date_naive());
            let today = Local::now().date_naive();
            let grid = calendar::month_grid(month.0, month.1);
            for (i, date) in grid.iter().enumerate() {
                let Some(date) = date else { continue };
                let col = i % 7;
                let row = i / 7;
                let cell = Rect::from_min_size(
                    Pos2::new(sx + col as f32 * 36.0, cal.top() + 20.0 + row as f32 * 27.0),
                    Vec2::new(36.0, 27.0),
                );
                let resp = ui.interact(cell, Id::unique(("due-day", i)), Sense::click());
                let is_selected = selected_date == Some(*date);
                let is_today = *date == today;
                let is_weekend = col >= 5;
                if is_selected {
                    ui.painter().rect_filled(
                        cell.shrink2(Vec2::new(2.0, 1.0)),
                        CornerRadius::same(5),
                        rgba(self.theme.accent, 0.14),
                    );
                } else if resp.hovered() {
                    ui.painter().rect_filled(
                        cell.shrink2(Vec2::new(2.0, 1.0)),
                        CornerRadius::same(5),
                        rgba(self.theme.fg, 0.05),
                    );
                }
                let color = if is_selected {
                    self.theme.accent
                } else if is_today {
                    self.theme.fg
                } else if is_weekend {
                    rgba(self.theme.fg, 0.42)
                } else {
                    rgba(self.theme.fg, 0.66)
                };
                ui.painter().text(
                    cell.center() - Vec2::new(0.0, 1.0),
                    Align2::CENTER_CENTER,
                    date.day().to_string(),
                    FontId::new(10.5, FontFamily::Monospace),
                    color,
                );
                if is_today && !is_selected {
                    ui.painter().circle_filled(
                        Pos2::new(cell.center().x, cell.bottom() - 4.5),
                        1.5,
                        rgba(self.theme.accent, 0.8),
                    );
                }
                if resp.clicked() {
                    let (h, m) = parsed
                        .and_then(|ms| Local.timestamp_millis_opt(ms).single())
                        .map(|d| (d.hour(), d.minute()))
                        .unwrap_or((18, 0));
                    if let Some(ms) = calendar::date_ms(date.year(), date.month(), date.day(), h, m)
                    {
                        if let Some(dt) = Local.timestamp_millis_opt(ms).single() {
                            draft = dt.format("%Y-%m-%d %H:%M").to_string();
                        }
                    }
                }
            }

            ui.add_space(3.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                for (label, h, m) in [
                    ("09:00", 9, 0),
                    ("12:00", 12, 0),
                    ("18:00", 18, 0),
                    ("21:00", 21, 0),
                ] {
                    if chip_button(
                        ui,
                        label,
                        self.theme.fg,
                        rgba(self.theme.fg, 0.055),
                        "Set time",
                    ) {
                        let date = selected_date.unwrap_or(today);
                        if let Some(ms) =
                            calendar::date_ms(date.year(), date.month(), date.day(), h, m)
                        {
                            if let Some(dt) = Local.timestamp_millis_opt(ms).single() {
                                draft = dt.format("%Y-%m-%d %H:%M").to_string();
                            }
                        }
                    }
                }
            });
            ui.add_space(5.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 5.0;
                ui.label(
                    egui::RichText::new("Repeat")
                        .size(10.0)
                        .color(rgba(self.theme.fg, 0.45)),
                );
                for (label, value) in [
                    ("Once", ""),
                    ("Daily", "daily"),
                    ("Weekly", "weekly"),
                    ("Monthly", "monthly"),
                ] {
                    let active = repeat == value;
                    if chip_button(
                        ui,
                        label,
                        if active { self.theme.ok } else { self.theme.fg },
                        rgba(
                            if active { self.theme.ok } else { self.theme.fg },
                            if active { 0.12 } else { 0.05 },
                        ),
                        "Repeat this task",
                    ) {
                        repeat = value.to_string();
                    }
                }
            });
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 5.0;
                ui.label(
                    egui::RichText::new("Remind")
                        .size(10.0)
                        .color(rgba(self.theme.fg, 0.45)),
                );
                for (label, value) in [
                    ("5m", 300_000i64),
                    ("1h", 3_600_000),
                    ("1d", 86_400_000),
                    ("At time", 1),
                ] {
                    let active = lead == value;
                    if chip_button(
                        ui,
                        label,
                        if active {
                            self.theme.accent
                        } else {
                            self.theme.fg
                        },
                        rgba(
                            if active {
                                self.theme.accent
                            } else {
                                self.theme.fg
                            },
                            if active { 0.12 } else { 0.05 },
                        ),
                        "Reminder lead time",
                    ) {
                        lead = value;
                    }
                }
            });
            ui.add_space(5.0);
            let resp = ui.add_sized(
                [ui.available_width(), 30.0],
                egui::TextEdit::singleline(&mut draft)
                    .font(FontId::new(12.5, FontFamily::Monospace))
                    .hint_text("or type: tomorrow 09:00 · +2h"),
            );
            if self.due_focus {
                resp.request_focus();
                self.due_focus = false;
            }
            ui.add_space(3.0);
            if draft.trim().is_empty() {
                ui.label(
                    egui::RichText::new("No due date")
                        .size(10.5)
                        .color(self.theme.dim),
                );
            } else if let Some(ms) = parse_when(&draft) {
                ui.label(
                    egui::RichText::new(format!(
                        "{}  ·  reminder {}",
                        due_label(ms),
                        lead_label(lead)
                    ))
                    .size(10.5)
                    .color(self.theme.accent),
                );
            } else {
                ui.label(
                    egui::RichText::new("Couldn't parse that — try “tomorrow 09:00”")
                        .size(10.5)
                        .color(self.theme.danger),
                );
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new("Clear")
                                .size(12.0)
                                .color(self.theme.danger),
                        )
                        .frame(false)
                        .min_size(Vec2::new(56.0, 28.0)),
                    )
                    .clicked()
                {
                    apply = Some(None);
                }
                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new("Cancel")
                                .size(12.0)
                                .color(rgba(self.theme.fg, 0.55)),
                        )
                        .frame(false)
                        .min_size(Vec2::new(64.0, 28.0)),
                    )
                    .clicked()
                {
                    self.due_edit = None;
                    self.due_anchor = None;
                }
                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new("Save")
                                .size(12.0)
                                .color(self.theme.accent),
                        )
                        .fill(rgba(self.theme.accent, 0.12))
                        .stroke(Stroke::new(1.0, rgba(self.theme.accent, 0.22)))
                        .corner_radius(CornerRadius::same(6))
                        .min_size(Vec2::new(78.0, 28.0)),
                    )
                    .clicked()
                    || (resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                {
                    if let Some(ms) = parse_when(&draft) {
                        apply = Some(Some(ms));
                    }
                }
            });
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.due_edit = None;
                self.due_anchor = None;
            }
        });
        if !open {
            self.due_edit = None;
            self.due_anchor = None;
        }
        if let Some(value) = apply {
            self.snapshot();
            let remind = match value {
                Some(ms) => (ms - lead).max(now_ms() + 5_000),
                None => 0,
            };
            if let Some(t) = self.task_mut(&id) {
                t.due = value.unwrap_or(0);
                t.remind_at = if value.is_some() { remind } else { 0 };
                t.remind_lead = lead;
                t.repeat = repeat;
                t.notified = false;
            }
            self.dirty = true;
            self.due_edit = None;
            self.due_anchor = None;
        } else if let Some(e) = self.due_edit.as_mut() {
            e.draft = draft;
            e.month = month;
            e.repeat = repeat;
            e.lead = lead;
        }
    }

    fn ui_toast(&mut self, ctx: &egui::Context) {
        let Some((msg, at)) = self.toast.clone() else {
            return;
        };
        let elapsed = at.elapsed().as_secs_f32();
        const LIFE: f32 = 2.8;
        if elapsed > LIFE {
            self.toast = None;
            return;
        }
        let alpha = if self.settings.reduce_motion.unwrap_or(false) {
            1.0
        } else if elapsed < 0.14 {
            elapsed / 0.14
        } else if elapsed > LIFE - 0.32 {
            (LIFE - elapsed) / 0.32
        } else {
            1.0
        }
        .clamp(0.0, 1.0);
        let lift = (1.0 - alpha) * 7.0;
        let can_undo = self
            .undo_snapshot
            .as_ref()
            .is_some_and(|(_, at)| at.elapsed() < Duration::from_secs(3));
        let mut undo_clicked = false;
        egui::Area::new(Id::unique("toast"))
            .order(egui::Order::Foreground)
            .anchor(Align2::CENTER_BOTTOM, [0.0, -92.0 + lift])
            .interactable(can_undo)
            .show(ctx, |ui| {
                egui::Frame::NONE
                    .fill(self.theme.surface)
                    .stroke(Stroke::new(1.0, rgba(self.theme.accent, 0.22)))
                    .corner_radius(CornerRadius::same(8))
                    .inner_margin(egui::Margin::symmetric(12, 8))
                    .shadow(egui::epaint::Shadow {
                        offset: [0, 8],
                        blur: 24,
                        spread: 0,
                        color: Color32::from_black_alpha(120),
                    })
                    .multiply_with_opacity(alpha)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let (bar, _) =
                                ui.allocate_exact_size(Vec2::new(2.0, 14.0), Sense::hover());
                            ui.painter().rect_filled(
                                bar,
                                CornerRadius::same(1),
                                rgba(self.theme.accent, 0.75 * alpha),
                            );
                            ui.add_space(2.0);
                            ui.label(
                                egui::RichText::new(&msg)
                                    .size(11.5)
                                    .color(rgba(self.theme.fg, 0.92 * alpha)),
                            );
                            if can_undo {
                                ui.add_space(6.0);
                                if ui
                                    .add(
                                        egui::Button::new(
                                            egui::RichText::new("Undo")
                                                .size(10.5)
                                                .color(rgba(self.theme.accent, alpha)),
                                        )
                                        .frame(false),
                                    )
                                    .clicked()
                                {
                                    undo_clicked = true;
                                }
                            }
                        });
                    });
            });
        if undo_clicked {
            if let Some((items, _)) = self.undo_snapshot.take() {
                self.doc.items = items;
                self.selected = None;
                self.multi.clear();
                self.dirty = true;
                self.toast("Undone");
            }
        }
        let next = if elapsed < 0.2 || elapsed > LIFE - 0.4 {
            Duration::from_millis(16)
        } else {
            Duration::from_millis(90)
        };
        ctx.request_repaint_after(next);
    }

    fn maybe_save_geometry(&mut self, ctx: &egui::Context) {
        if self.hidden || self.quitting {
            return;
        }
        if self.last_geom_save.elapsed() < Duration::from_millis(900) {
            return;
        }
        let size = ctx.content_rect().size();
        if (size.x - self.last_size.x).abs() > 1.0 || (size.y - self.last_size.y).abs() > 1.0 {
            self.last_size = size;
            self.settings.width = Some(size.x);
            self.settings.height = Some(size.y);
            model::save_settings(&self.settings);
            self.last_geom_save = Instant::now();
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.handle_commands(&ctx);
        if !self.motion_applied {
            let reduce = self.settings.reduce_motion.unwrap_or(false);
            ctx.style_mut_of(egui::Theme::Dark, |style| {
                style.animation_time = if reduce { 0.0 } else { 0.16 };
            });
            self.motion_applied = true;
        }
        // Closing the window is harmless: the daemon and tray stay resident.
        if ctx.input(|i| i.viewport().close_requested()) && !self.quitting {
            self.save_geometry(&ctx);
            self.save_if_dirty();
            self.quitting = true;
            ipc::remove_gui_socket();
        }

        if self.last_daemon_check.elapsed() >= Duration::from_secs(10) {
            self.last_daemon_check = Instant::now();
            self.daemon_present = ipc::daemon_alive();
        }
        if !self.daemon_present {
            self.check_reminders();
        }
        self.maybe_external_reload();
        self.maybe_save_geometry(&ctx);
        if !self.hidden
            && !self.pos_applied
            && self.opened_at.elapsed() > Duration::from_millis(220)
        {
            self.apply_position(&ctx);
        }

        // Keyboard.
        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::K)) {
            if self.palette_open {
                self.palette_open = false;
            } else {
                self.palette_open = true;
                self.palette_query.clear();
                self.palette_idx = 0;
                self.palette_focus = true;
            }
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            if self.palette_open {
                self.palette_open = false;
                self.palette_query.clear();
            } else if self.help_open {
                self.help_open = false;
            } else if self.stats_open {
                self.stats_open = false;
            } else if self.focus_mode {
                self.focus_mode = false;
            } else if self.edit.is_none()
                && self.due_edit.is_none()
                && !egui::Popup::is_any_open(&ctx)
            {
                if self.inline.is_some() {
                    self.inline = None;
                } else if self.link_from.is_some() || self.link_drag.is_some() {
                    self.link_from = None;
                    self.link_drag = None;
                } else if !self.multi.is_empty() {
                    self.multi.clear();
                } else if self.input_parent.is_some() {
                    self.input_parent = None;
                } else if !self.search.is_empty() || !self.input.is_empty() {
                    self.search.clear();
                    self.input.clear();
                } else {
                    self.hide_window(&ctx);
                }
            }
        }
        let typing = ctx.egui_wants_keyboard_input()
            || ctx.text_edit_focused()
            || self.inline.is_some()
            || self.edit.is_some()
            || self.due_edit.is_some()
            || self.palette_open
            || self.stats_open
            || self.help_open;
        let popup_open = egui::Popup::is_any_open(&ctx);
        let empty_command_focus = ctx.text_edit_focused() && self.input.is_empty();
        let nav_ok = self.inline.is_none()
            && self.edit.is_none()
            && self.due_edit.is_none()
            && !popup_open
            && (!typing || empty_command_focus);
        if nav_ok {
            let step = ctx.input(|i| {
                let arrow = if i.key_pressed(egui::Key::ArrowDown) {
                    1
                } else if i.key_pressed(egui::Key::ArrowUp) {
                    -1
                } else {
                    0
                };
                if arrow != 0 {
                    arrow
                } else if !typing {
                    if i.key_pressed(egui::Key::J) {
                        1
                    } else if i.key_pressed(egui::Key::K) {
                        -1
                    } else {
                        0
                    }
                } else {
                    0
                }
            });
            if step != 0 {
                ctx.memory_mut(|m| m.stop_text_input());
                let ids: Vec<String> = self.visible().iter().map(|t| t.id.clone()).collect();
                if !ids.is_empty() {
                    let next = match self
                        .selected
                        .as_ref()
                        .and_then(|s| ids.iter().position(|i| i == s))
                    {
                        Some(i) => (i as i32 + step).clamp(0, ids.len() as i32 - 1) as usize,
                        None if step > 0 => 0,
                        None => ids.len() - 1,
                    };
                    self.selected = Some(ids[next].clone());
                    self.scroll_to = Some(ids[next].clone());
                    self.kbd_nav = true;
                }
            }
        }
        if !typing && !popup_open {
            if ctx.input(|i| i.key_pressed(egui::Key::Space))
                && self.filter != Filter::Trash
                && self.selected.is_some()
            {
                if let Some(id) = self.selected.clone() {
                    self.toggle(&id);
                }
            }
            if ctx.input(|i| i.key_pressed(egui::Key::Enter))
                && !self.focus_mode
                && self.filter != Filter::Trash
                && self.selected.is_some()
            {
                if let Some(id) = self.selected.clone() {
                    self.start_inline(&id, InlineField::Title);
                    self.kbd_nav = true;
                }
            }
            if ctx.input(|i| i.key_pressed(egui::Key::M))
                && !self.focus_mode
                && self.filter != Filter::Trash
                && self.selected.is_some()
            {
                if let Some(id) = self.selected.clone() {
                    egui::Popup::open_id(&ctx, Id::unique(("more", &id)).with("popup"));
                    self.kbd_nav = true;
                }
            }
            if ctx.input(|i| i.key_pressed(egui::Key::D))
                && self.filter != Filter::Trash
                && self.selected.is_some()
            {
                self.due_request = self.selected.clone();
                self.kbd_nav = true;
            }
        }
        if !typing && ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::A)) {
            let ids: Vec<String> = self.visible().iter().map(|t| t.id.clone()).collect();
            self.multi = ids.into_iter().collect();
        }
        if (!typing || self.input.is_empty())
            && self.inline.is_none()
            && ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Z))
        {
            if ctx.input(|i| i.modifiers.shift) {
                self.redo();
            } else {
                self.undo();
            }
        }
        if !typing && !popup_open && !self.focus_mode && self.filter != Filter::Trash {
            for (key, view) in [
                (egui::Key::Num1, SmartView::Inbox),
                (egui::Key::Num2, SmartView::Today),
                (egui::Key::Num3, SmartView::Upcoming),
                (egui::Key::Num4, SmartView::Overdue),
                (egui::Key::Num5, SmartView::NoDate),
            ] {
                if ctx.input(|i| i.key_pressed(key)) {
                    self.view = view;
                    self.filter = Filter::All;
                    self.multi.clear();
                    self.collapsed_sections.clear();
                }
            }
        }
        if ctx.input(|i| i.modifiers.alt) {
            for (key, view) in [
                (egui::Key::Num1, SmartView::Inbox),
                (egui::Key::Num2, SmartView::Today),
                (egui::Key::Num3, SmartView::Upcoming),
                (egui::Key::Num4, SmartView::Overdue),
                (egui::Key::Num5, SmartView::NoDate),
            ] {
                if ctx.input(|i| i.key_pressed(key)) {
                    self.view = view;
                    self.filter = Filter::All;
                    self.multi.clear();
                    self.collapsed_sections.clear();
                }
            }
        }
        if ctx.input(|i| i.modifiers.command) {
            for (key, filter) in [
                (egui::Key::Num1, Filter::All),
                (egui::Key::Num2, Filter::Open),
                (egui::Key::Num3, Filter::Done),
                (egui::Key::Num4, Filter::Trash),
            ] {
                if ctx.input(|i| i.key_pressed(key)) {
                    self.filter = filter;
                    if filter == Filter::Trash {
                        self.view = SmartView::Inbox;
                        self.collapsed_sections.clear();
                    }
                    self.multi.clear();
                }
            }
            if ctx.input(|i| i.key_pressed(egui::Key::F)) {
                self.focus_input = true;
                if self.input.is_empty() && self.search.is_empty() {
                    self.input = "/".into();
                }
            }
        }
        if !ctx.egui_wants_keyboard_input()
            && self.inline.is_none()
            && ctx.input(|i| i.key_pressed(egui::Key::Slash))
            && !self.read_only
        {
            self.input = "/".into();
            self.focus_input = true;
        }
        if let Some(id) = self.selected.clone() {
            if !typing
                && self.filter != Filter::Trash
                && self.inline.is_none()
                && ui.input(|i| i.key_pressed(egui::Key::Delete))
            {
                self.soft_delete(&id);
            }
        }
        if self.dirty && self.last_save.elapsed() > Duration::from_millis(300) {
            self.save_now();
        }

        let bg = self.theme.bg;
        let border = rgba(self.theme.fg, 0.065);
        let frame = egui::Frame::NONE
            .fill(bg)
            .stroke(Stroke::new(1.0, border))
            .corner_radius(CornerRadius::same(12))
            .inner_margin(egui::Margin::symmetric(22, 18));
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            if self.focus_mode {
                self.ui_focus(ui);
            } else {
                self.ui_header(ui);
                self.ui_progress(ui);
                self.ui_filters(ui);
                self.ui_views(ui);
                self.ui_active_filters(ui);
                self.ui_error_banner(ui);
                let reserved = 92.0;
                let list_h = (ui.available_height() - reserved).max(90.0);
                if self.tag_rail_open && ui.available_width() < 560.0 {
                    self.tag_rail_open = false;
                }
                let rail_open = self.tag_rail_open;
                ui.horizontal_top(|ui| {
                    if rail_open {
                        self.ui_tag_rail(ui, list_h);
                        ui.add_space(8.0);
                    }
                    egui::ScrollArea::vertical()
                        .id_salt("task-list")
                        .auto_shrink([false, false])
                        .max_height(list_h)
                        .show(ui, |ui| {
                            self.ui_list(ui);
                        });
                });
                if self.multi.is_empty() {
                    self.ui_input(ui);
                } else {
                    self.ui_batch(ui);
                }
            }
        });
        self.ui_edit_window(&ctx);
        self.ui_due_window(&ctx);
        self.ui_palette(&ctx);
        self.ui_stats_window(&ctx);
        self.ui_help_window(&ctx);
        self.ui_toast(&ctx);

        if self.link_from.is_some() || self.link_drag.is_some() || self.drag_id.is_some() {
            ctx.request_repaint();
        }
        if self.toast.is_some() {
            ctx.request_repaint_after(Duration::from_millis(120));
        }
        if self.hidden {
            ctx.request_repaint_after(Duration::from_secs(10));
        } else {
            ctx.request_repaint_after(Duration::from_secs(5));
        }
        let _ = self.start_hidden;
    }

    fn on_exit(&mut self) {
        self.save_now();
        model::save_settings(&self.settings);
        ipc::remove_gui_socket();
    }
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
    let fam = if bold {
        FontFamily::Name("bold".into())
    } else {
        FontFamily::Monospace
    };
    let p = painter.with_clip_rect(clip);
    p.text(
        pos,
        Align2::LEFT_CENTER,
        text,
        FontId::new(size, fam),
        color,
    );
}

fn text_width(ui: &egui::Ui, text: &str, size: f32, bold: bool) -> f32 {
    let fam = if bold {
        FontFamily::Name("bold".into())
    } else {
        FontFamily::Monospace
    };
    ui.painter()
        .layout_no_wrap(text.to_string(), FontId::new(size, fam), Color32::WHITE)
        .size()
        .x
}

/// Row titles drop inline `#tags`: they are drawn as chips underneath, so the
/// headline stays quiet. The editor still shows the raw title.
fn display_title(title: &str) -> String {
    if !title.split_whitespace().any(|w| w.starts_with('#')) {
        return title.to_string();
    }
    let words: Vec<&str> = title
        .split_whitespace()
        .filter(|w| !w.starts_with('#'))
        .collect();
    if words.is_empty() {
        title.to_string()
    } else {
        words.join(" ")
    }
}

/// Truncate on a character boundary and add an ellipsis when the text is wider
/// than `max_w`. Measured with the same painter that will draw it, so the result
/// never ends in a half-cut glyph.
fn fit_text(ui: &egui::Ui, text: &str, max_w: f32, size: f32, bold: bool) -> String {
    if max_w <= 0.0 {
        return String::new();
    }
    if text_width(ui, text, size, bold) <= max_w {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let (mut lo, mut hi) = (0usize, chars.len());
    while lo < hi {
        let mid = (lo + hi + 1) / 2;
        let candidate = format!("{}…", chars.iter().take(mid).collect::<String>());
        if text_width(ui, &candidate, size, bold) <= max_w {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    if lo == 0 {
        "…".into()
    } else {
        format!("{}…", chars.iter().take(lo).collect::<String>())
    }
}

fn action_button(ui: &mut egui::Ui, rect: Rect, label: &str, color: Color32, tip: &str) -> bool {
    let resp = ui.interact(
        rect,
        Id::unique(("action", label, rect.min.y as i32)),
        Sense::click(),
    );
    let hovered = resp.hovered();
    if hovered {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(5), rgba(color, 0.10));
    }
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        FontId::new(10.5, FontFamily::Monospace),
        rgba(color, if hovered { 1.0 } else { 0.82 }),
    );
    resp.on_hover_text(tip).clicked()
}

fn chip_button(ui: &mut egui::Ui, label: &str, color: Color32, fill: Color32, tip: &str) -> bool {
    let w = text_width(ui, label, 10.5, false) + 16.0;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 22.0), Sense::click());
    ui.painter().rect_filled(
        rect,
        CornerRadius::same(5),
        if resp.hovered() {
            rgba(color, 0.15)
        } else {
            fill
        },
    );
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        FontId::new(10.5, FontFamily::Monospace),
        rgba(color, if resp.hovered() { 1.0 } else { 0.90 }),
    );
    resp.on_hover_text(tip).clicked()
}

fn chip_rect(ui: &mut egui::Ui, rect: Rect, label: &str, color: Color32, id: Id) -> bool {
    let resp = ui.interact(rect, id, Sense::click());
    let hovered = resp.hovered();
    ui.painter().rect_filled(
        rect,
        CornerRadius::same(5),
        rgba(color, if hovered { 0.18 } else { 0.08 }),
    );
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        FontId::new(10.5, FontFamily::Monospace),
        rgba(color, if hovered { 1.0 } else { 0.95 }),
    );
    resp.clicked()
}

fn icon_rect(ui: &mut egui::Ui, rect: Rect, glyph: &str, color: Color32, id: Id) -> bool {
    let resp = ui.interact(rect, id, Sense::click());
    if resp.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(5), rgba(color, 0.09));
    }
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        glyph,
        FontId::new(14.0, FontFamily::Monospace),
        rgba(color, if resp.hovered() { 1.0 } else { 0.70 }),
    );
    resp.clicked()
}

fn menu_button(ui: &mut egui::Ui, rect: Rect, id: Id) -> egui::Response {
    let resp = ui.interact(rect, id, Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(
            rect,
            CornerRadius::same(5),
            rgba(ui.style().visuals.text_color(), 0.07),
        );
    }
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        "⋯",
        FontId::new(12.0, FontFamily::Monospace),
        rgba(
            ui.style().visuals.text_color(),
            if resp.hovered() { 1.0 } else { 0.55 },
        ),
    );
    resp.clone().on_hover_text("More actions");
    resp
}

fn rail_item(
    ui: &mut egui::Ui,
    label: &str,
    count: usize,
    active: bool,
    color: Color32,
    theme: &Theme,
) -> bool {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 25.0), Sense::click());
    if active {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(5), rgba(color, 0.10));
        ui.painter().rect_filled(
            Rect::from_min_size(
                Pos2::new(rect.left() + 1.0, rect.top() + 6.0),
                Vec2::new(2.0, rect.height() - 12.0),
            ),
            CornerRadius::same(1),
            color,
        );
    } else if resp.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(5), rgba(theme.fg, 0.04));
    }
    let count = count.to_string();
    let cw = text_width(ui, &count, 9.5, false);
    let label = fit_text(ui, label, (w - cw - 22.0).max(30.0), 10.5, false);
    ui.painter().text(
        Pos2::new(rect.left() + 8.0, rect.center().y),
        Align2::LEFT_CENTER,
        &label,
        FontId::new(10.5, FontFamily::Monospace),
        if active { color } else { rgba(theme.fg, 0.68) },
    );
    ui.painter().text(
        Pos2::new(rect.right() - 7.0, rect.center().y),
        Align2::RIGHT_CENTER,
        count,
        FontId::new(9.5, FontFamily::Monospace),
        rgba(theme.fg, if active { 0.55 } else { 0.40 }),
    );
    resp.clicked()
}

fn icon_button(ui: &mut egui::Ui, size: Vec2, glyph: &str, color: Color32, tip: &str) -> bool {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    if resp.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(6), rgba(color, 0.085));
    }
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        glyph,
        FontId::new(15.0, FontFamily::Monospace),
        rgba(color, if resp.hovered() { 1.0 } else { 0.74 }),
    );
    resp.on_hover_text(tip).clicked()
}

fn focus_pill(ui: &mut egui::Ui, label: &str, color: Color32, theme: &Theme) {
    let w = text_width(ui, label, 10.0, false) + 14.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, 20.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::same(10), rgba(color, 0.09));
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        FontId::new(10.0, FontFamily::Monospace),
        rgba(color, if color == theme.dim { 0.75 } else { 0.92 }),
    );
}

fn lead_label(ms: i64) -> String {
    if ms <= 1 {
        "at due time".into()
    } else if ms < 3_600_000 {
        format!("{} min before", (ms / 60_000).max(1))
    } else if ms < 86_400_000 {
        format!("{} h before", (ms / 3_600_000).max(1))
    } else {
        format!("{} d before", (ms / 86_400_000).max(1))
    }
}

fn priority_color(priority: u8, theme: &Theme) -> Color32 {
    match priority {
        1 => theme.danger,
        2 => theme.accent,
        3 => theme.accent2,
        _ => theme.dim,
    }
}

fn tag_color(tag: &str, theme: &Theme) -> Color32 {
    let mut h: u32 = 2_166_136_261;
    for b in tag.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(16_777_619);
    }
    let palette = [
        theme.accent,
        theme.accent2,
        mix(theme.accent, theme.ok, 0.55),
        mix(theme.accent2, theme.fg, 0.35),
        theme.ok,
    ];
    palette[(h as usize) % palette.len()]
}

fn paint_app_mark(p: &egui::Painter, rect: Rect, theme: &Theme) {
    p.rect_filled(rect, CornerRadius::same(8), rgba(theme.accent, 0.055));
    p.rect_stroke(
        rect,
        CornerRadius::same(8),
        Stroke::new(1.0, rgba(theme.accent, 0.16)),
        egui::StrokeKind::Inside,
    );
    let x = rect.left() + 9.0;
    let top = rect.top() + 9.0;
    let bottom = rect.bottom() - 9.0;
    let mid = rect.center().y;
    let branch = rect.right() - 9.0;
    let stroke = Stroke::new(1.25, rgba(theme.accent, 0.48));
    p.line_segment([Pos2::new(x, top), Pos2::new(x, bottom)], stroke);
    p.line_segment([Pos2::new(x, mid), Pos2::new(branch, mid)], stroke);
    p.circle_filled(Pos2::new(x, top), 1.7, rgba(theme.accent, 0.9));
    p.circle_filled(Pos2::new(x, bottom), 1.7, rgba(theme.accent, 0.9));
    p.circle_stroke(
        Pos2::new(branch, mid),
        2.2,
        Stroke::new(1.1, rgba(theme.accent, 0.9)),
    );
}

/// Smooth vertical S-curve between a parent node and a child node.
fn branch_points(from: Pos2, to: Pos2) -> [Pos2; 4] {
    let dy = to.y - from.y;
    [
        from,
        Pos2::new(from.x, from.y + dy * 0.55),
        Pos2::new(to.x, to.y - dy * 0.55),
        to,
    ]
}

fn paint_branch(p: &egui::Painter, from: Pos2, to: Pos2, stroke: Stroke) {
    p.add(egui::epaint::CubicBezierShape::from_points_stroke(
        branch_points(from, to),
        false,
        Color32::TRANSPARENT,
        stroke,
    ));
}

fn cubic_point(points: [Pos2; 4], t: f32) -> Pos2 {
    let u = 1.0 - t;
    let w0 = u * u * u;
    let w1 = 3.0 * u * u * t;
    let w2 = 3.0 * u * t * t;
    let w3 = t * t * t;
    Pos2::new(
        points[0].x * w0 + points[1].x * w1 + points[2].x * w2 + points[3].x * w3,
        points[0].y * w0 + points[1].y * w1 + points[2].y * w2 + points[3].y * w3,
    )
}
