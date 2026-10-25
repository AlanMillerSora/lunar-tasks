use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use chrono::TimeZone;

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontFamily, FontId, Id, Layout, Pos2, Rect, Sense,
    Stroke, Vec2,
};

use crate::hypr;
use crate::ipc;
use crate::model::{self, Doc, Settings, Task};
use crate::theme::{self, Theme, mix, rgba};
use crate::util::{age, due_label, now_ms, one_line, parse_when, truncate};

const TITLE_X: f32 = 104.0;
const LANE_X: f32 = 26.0;
const LANE_STEP: f32 = 15.0;
const DEFAULT_W: f32 = 640.0;
const DEFAULT_H: f32 = 780.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Filter {
    All,
    Open,
    Done,
    Trash,
}

struct LinkFlash {
    child: String,
    parent: String,
    at: Instant,
}

pub struct App {
    doc: Doc,
    path: PathBuf,
    load_error: Option<String>,
    read_only: bool,
    theme: Theme,
    settings: Settings,
    filter: Filter,
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
    due_edit: Option<(String, String)>,
    edit: Option<(String, String, String)>,
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
        App {
            doc,
            path,
            load_error,
            read_only,
            theme: theme::load_theme(),
            settings,
            filter: Filter::All,
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
            edit: None,
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
        self.toast = Some((msg.into(), Instant::now()));
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

    fn toggle(&mut self, id: &str) {
        self.snapshot();
        if let Some(t) = self.task_mut(id) {
            t.done = !t.done;
        }
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
        if self.selected.as_deref() == Some(id) {
            self.selected = None;
        }
        self.dirty = true;
        self.toast(format!(
            "Moved {} task{} to trash",
            n,
            if n == 1 { "" } else { "s" }
        ));
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

    fn purge(&mut self, id: &str) {
        let ids = self.subtree_ids(id);
        self.snapshot();
        self.doc.items.retain(|t| !ids.contains(&t.id));
        self.dirty = true;
        self.toast("Deleted forever");
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
        self.snapshot();
        self.doc.items.retain(|t| !all.contains(&t.id));
        for t in &mut self.doc.items {
            if all.contains(&t.parent) {
                t.parent.clear();
            }
        }
        self.multi.clear();
        self.dirty = true;
        self.toast(format!("Deleted {} task(s) forever", ids.len()));
    }

    fn set_done_many(&mut self, ids: &[String], done: bool) {
        if ids.is_empty() {
            return;
        }
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
            self.toast(format!(
                "{} {} task{}",
                if done { "Completed" } else { "Reopened" },
                n,
                if n == 1 { "" } else { "s" }
            ));
        }
    }

    fn delete_many(&mut self, ids: &[String]) {
        if ids.is_empty() {
            return;
        }
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
            self.toast(format!(
                "Moved {} task{} to trash",
                n,
                if n == 1 { "" } else { "s" }
            ));
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
        if let Some(ci) = self.doc.items.iter().position(|t| t.id == child) {
            let item = self.doc.items.remove(ci);
            if let Some(pi) = self.doc.items.iter().position(|t| t.id == parent) {
                self.doc.items.insert(pi + 1, item);
            } else {
                self.doc.items.insert(0, item);
            }
        }
        self.dirty = true;
        self.link_flash = Some(LinkFlash {
            child: child.to_string(),
            parent: parent.to_string(),
            at: Instant::now(),
        });
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

    fn visible(&self) -> Vec<&Task> {
        self.doc
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
            .collect()
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

    fn ui_header(&mut self, ui: &mut egui::Ui) {
        let (open, done, _) = self.counts();
        let total = open + done;
        let pct = if total > 0 {
            (done * 100 + total / 2) / total
        } else {
            0
        };
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(Vec2::new(22.0, 28.0), Sense::hover());
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                "✦",
                FontId::new(20.0, FontFamily::Monospace),
                self.theme.accent,
            );
            ui.add_space(6.0);
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("TASKS")
                            .size(14.0)
                            .color(self.theme.fg)
                            .strong(),
                    );
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(format!("{pct}%"))
                            .size(13.0)
                            .color(self.theme.accent),
                    );
                });
                ui.label(
                    egui::RichText::new(format!("{open} open · {done} done"))
                        .size(12.0)
                        .color(self.theme.dim),
                );
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if icon_button(
                    ui,
                    Vec2::new(24.0, 24.0),
                    "✕",
                    self.theme.dim,
                    "Hide to tray (Esc)",
                ) {
                    self.hide_window(ui.ctx());
                }
                if icon_button(ui, Vec2::new(24.0, 24.0), "+", self.theme.dim, "New task") {
                    self.focus_input = true;
                }
                let (dot, dresp) = ui.allocate_exact_size(Vec2::new(16.0, 16.0), Sense::hover());
                ui.painter().circle_filled(
                    dot.center(),
                    3.5,
                    if self.daemon_present {
                        self.theme.ok
                    } else {
                        rgba(self.theme.danger, 0.8)
                    },
                );
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
        let (rect, _) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 4.0), Sense::hover());
        let p = ui.painter();
        p.rect_filled(rect, CornerRadius::same(2), rgba(self.theme.fg, 0.07));
        if ratio > 0.001 {
            let fill =
                Rect::from_min_size(rect.min, Vec2::new(rect.width() * ratio, rect.height()));
            p.rect_filled(fill, CornerRadius::same(2), self.theme.accent);
        }
        ui.add_space(10.0);
    }

    fn ui_filters(&mut self, ui: &mut egui::Ui) {
        let (open, done, trash) = self.counts();
        let items = [
            ("All", open + done, Filter::All),
            ("Open", open, Filter::Open),
            ("Done", done, Filter::Done),
            ("Trash", trash, Filter::Trash),
        ];
        ui.horizontal(|ui| {
            for (label, count, f) in items {
                let active = self.filter == f;
                let color = if active {
                    self.theme.accent
                } else {
                    rgba(self.theme.fg, 0.55)
                };
                let text = format!("{label} {count}");
                let galley = ui.painter().layout_no_wrap(
                    text.clone(),
                    FontId::new(13.0, FontFamily::Monospace),
                    color,
                );
                let (rect, resp) =
                    ui.allocate_exact_size(Vec2::new(galley.size().x + 12.0, 28.0), Sense::click());
                if resp.hovered() {
                    ui.painter().rect_filled(
                        rect,
                        CornerRadius::same(5),
                        rgba(self.theme.fg, 0.045),
                    );
                }
                ui.painter().text(
                    rect.center(),
                    Align2::CENTER_CENTER,
                    &text,
                    FontId::new(13.0, FontFamily::Monospace),
                    color,
                );
                if active {
                    let line = Rect::from_min_size(
                        Pos2::new(rect.left() + 4.0, rect.bottom() - 2.0),
                        Vec2::new(rect.width() - 8.0, 1.5),
                    );
                    ui.painter()
                        .rect_filled(line, CornerRadius::ZERO, self.theme.accent);
                }
                if resp.clicked() {
                    self.filter = f;
                    self.multi.clear();
                }
                ui.add_space(6.0);
            }
        });
        ui.add_space(6.0);
    }

    fn ui_active_filters(&mut self, ui: &mut egui::Ui) -> bool {
        if self.tag_filter.is_none() && self.search.is_empty() {
            return false;
        }
        let mut clear_tag = false;
        let mut clear_search = false;
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("filter")
                    .size(11.0)
                    .color(rgba(self.theme.fg, 0.35)),
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
        ui.add_space(2.0);
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
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 46.0), Sense::hover());
        ui.painter()
            .rect_filled(rect, CornerRadius::same(6), rgba(self.theme.danger, 0.10));
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(6),
            Stroke::new(1.0, rgba(self.theme.danger, 0.35)),
            egui::StrokeKind::Inside,
        );
        ui.painter().text(
            Pos2::new(rect.left() + 10.0, rect.top() + 12.0),
            Align2::LEFT_CENTER,
            "State file unreadable — editing disabled",
            FontId::new(12.0, FontFamily::Monospace),
            self.theme.danger,
        );
        ui.painter().text(
            Pos2::new(rect.left() + 10.0, rect.top() + 30.0),
            Align2::LEFT_CENTER,
            truncate(&err, 78),
            FontId::new(11.0, FontFamily::Monospace),
            rgba(self.theme.fg, 0.55),
        );
        let retry = Rect::from_min_size(
            Pos2::new(rect.right() - 70.0, rect.center().y - 12.0),
            Vec2::new(60.0, 24.0),
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
        ui.add_space(6.0);
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

        struct Metrics {
            h: f32,
            meta_y: f32,
            meta_h: f32,
            notes_y: f32,
            notes_h: f32,
            actions_y: f32,
            actions_h: f32,
        }

        let mut metrics: Vec<Metrics> = Vec::with_capacity(tasks.len());
        let mut anims: Vec<f32> = Vec::with_capacity(tasks.len());
        for t in &tasks {
            let anim = ui.ctx().animate_bool_with_time(
                Id::unique(("expand-anim", &t.id)),
                t.expanded,
                0.16,
            );
            anims.push(anim);
            let has_meta = !t.parent.is_empty() || !t.tags().is_empty();
            let meta_h = if has_meta { 19.0 } else { 0.0 };
            let full_notes_h = if t.notes.is_empty() {
                0.0
            } else {
                let galley = ui.painter().layout(
                    t.notes.clone(),
                    FontId::new(13.0, FontFamily::Monospace),
                    rgba(self.theme.fg, 0.55),
                    notes_wrap,
                );
                galley.size().y.min(150.0)
            };
            let collapsed_notes_h = if t.notes.is_empty() { 0.0 } else { 17.0 };
            let notes_h = collapsed_notes_h + (full_notes_h - collapsed_notes_h) * anim;
            let actions_h = 28.0 * anim;
            let mut y = 31.0;
            let meta_y = y;
            y += meta_h;
            let notes_y = y;
            y += notes_h;
            let actions_y = y;
            y += actions_h;
            let h = (y + 8.0).max(54.0);
            metrics.push(Metrics {
                h,
                meta_y,
                meta_h,
                notes_y,
                notes_h,
                actions_y,
                actions_h,
            });
        }

        let total_h: f32 = metrics.iter().map(|m| m.h).sum::<f32>() + 6.0;
        let (content_rect, _) = ui.allocate_exact_size(
            Vec2::new(
                width,
                total_h.max(if tasks.is_empty() { 130.0 } else { 60.0 }),
            ),
            Sense::hover(),
        );

        let mut positions: Vec<(String, Rect)> = Vec::new();
        let mut y = content_rect.top() + 2.0;
        for (t, m) in tasks.iter().zip(metrics.iter()) {
            let r = Rect::from_min_size(Pos2::new(content_rect.left(), y), Vec2::new(width, m.h));
            positions.push((t.id.clone(), r));
            y += m.h;
        }
        let pos_of = |id: &str| positions.iter().find(|(i, _)| i == id).map(|(_, r)| *r);
        let lane_x = |lane: usize| content_rect.left() + LANE_X + lane as f32 * LANE_STEP;
        let node_y = |r: Rect| r.top() + 24.0;

        let mut expand_id: Option<String> = None;
        let mut toggle_id: Option<String> = None;
        let mut delete_id: Option<String> = None;
        let mut restore_id: Option<String> = None;
        let mut purge_id: Option<String> = None;
        let mut due_id: Option<String> = None;
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

        // Graph rails behind the rows.
        {
            let p = ui.painter();
            let roots: Vec<Rect> = tasks
                .iter()
                .filter(|t| t.parent.is_empty())
                .filter_map(|t| pos_of(&t.id))
                .collect();
            for w in roots.windows(2) {
                let x = lane_x(0);
                p.line_segment(
                    [Pos2::new(x, node_y(w[0])), Pos2::new(x, node_y(w[1]))],
                    Stroke::new(1.0, rgba(self.theme.fg, 0.08)),
                );
            }
            let palette = [
                self.theme.accent,
                self.theme.accent2,
                Color32::from_rgb(94, 124, 165),
                Color32::from_rgb(255, 167, 211),
            ];
            for t in &tasks {
                if t.parent.is_empty() {
                    continue;
                }
                let Some(pr) = pos_of(&t.parent) else {
                    continue;
                };
                let Some(cr) = pos_of(&t.id) else { continue };
                let lane = self.lane(&t.id);
                let px = lane_x(self.lane(&t.parent));
                let cx = lane_x(lane);
                let py = node_y(pr);
                let cy = node_y(cr);
                let color = palette[lane % palette.len()];
                let stroke = Stroke::new(1.5, rgba(color, if t.done { 0.30 } else { 0.55 }));
                let dir = if cy >= py { 1.0 } else { -1.0 };
                let bend = cy - dir * 10.0;
                p.line_segment([Pos2::new(px, py), Pos2::new(px, bend)], stroke);
                p.line_segment([Pos2::new(px, bend), Pos2::new(cx, bend)], stroke);
                p.line_segment([Pos2::new(cx, bend), Pos2::new(cx, cy)], stroke);
                p.circle_filled(Pos2::new(cx, cy), 2.2, rgba(color, 0.95));
            }
        }

        // Animated link formation.
        if let Some(flash) = &self.link_flash {
            let elapsed = flash.at.elapsed();
            if elapsed < Duration::from_millis(950) {
                let t = (elapsed.as_secs_f32() / 0.95).clamp(0.0, 1.0);
                if let (Some(pr), Some(cr)) = (pos_of(&flash.parent), pos_of(&flash.child)) {
                    let px = lane_x(self.lane(&flash.parent));
                    let cx = lane_x(self.lane(&flash.child));
                    let py = node_y(pr);
                    let cy = node_y(cr);
                    let dir = if cy >= py { 1.0 } else { -1.0 };
                    let bend = cy - dir * 10.0;
                    let color = rgba(self.theme.accent, (1.0 - t) * 0.95 + 0.05);
                    let p = ui.painter();
                    let stroke = Stroke::new(1.8 + 2.2 * (1.0 - t), color);
                    p.line_segment([Pos2::new(px, py), Pos2::new(px, bend)], stroke);
                    p.line_segment([Pos2::new(px, bend), Pos2::new(cx, bend)], stroke);
                    p.line_segment([Pos2::new(cx, bend), Pos2::new(cx, cy)], stroke);
                    let mid = Pos2::new(cx, bend);
                    p.circle_filled(mid, 4.0 * (1.0 - t) + 1.5, color);
                    p.circle_stroke(
                        Pos2::new(cx, cy),
                        8.0 + 10.0 * t,
                        Stroke::new(2.0 * (1.0 - t), color),
                    );
                }
                ui.ctx().request_repaint_after(Duration::from_millis(16));
            } else {
                self.link_flash = None;
            }
        }

        for (idx, t) in tasks.iter().enumerate() {
            let r = positions[idx].1;
            let m = &metrics[idx];
            let anim = anims[idx];
            let row = r.shrink2(Vec2::new(0.0, 2.0));
            let resp = ui.interact(row, Id::unique(("row", &t.id)), Sense::click());
            let selected = self.selected.as_deref() == Some(t.id.as_str());
            let hovered = resp.hovered();
            let link_highlight =
                self.link_from.is_some() && self.link_from.as_deref() != Some(t.id.as_str());
            if selected {
                ui.painter().rect_filled(
                    row,
                    CornerRadius::same(6),
                    rgba(self.theme.accent, 0.055),
                );
            } else if hovered {
                ui.painter()
                    .rect_filled(row, CornerRadius::same(6), rgba(self.theme.fg, 0.025));
            }
            if link_highlight && hovered {
                ui.painter().rect_stroke(
                    row,
                    CornerRadius::same(6),
                    Stroke::new(1.0, rgba(self.theme.accent, 0.5)),
                    egui::StrokeKind::Inside,
                );
            }

            if resp.clicked() {
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
                edit_id = Some(t.id.clone());
            }

            // Multi-select checkbox.
            if !self.multi.is_empty() {
                let boxr = Rect::from_center_size(
                    Pos2::new(row.left() + 8.0, row.top() + 18.0),
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
            let center = Pos2::new(lane_x(lane), node_y(r) - 4.0);
            let node = Rect::from_center_size(center, Vec2::new(26.0, 26.0));
            let nresp = ui.interact(node, Id::unique(("node", &t.id)), Sense::click_and_drag());
            let done_t =
                ui.ctx()
                    .animate_bool_with_time(Id::unique(("done-anim", &t.id)), t.done, 0.2);
            let p = ui.painter().with_clip_rect(row);
            p.circle_filled(
                center,
                7.0,
                if t.done {
                    self.theme.accent
                } else {
                    rgba(self.theme.bg, 0.9)
                },
            );
            p.circle_stroke(
                center,
                7.0,
                Stroke::new(
                    1.5,
                    if nresp.hovered() {
                        rgba(self.theme.fg, 0.8)
                    } else {
                        rgba(self.theme.fg, 0.45)
                    },
                ),
            );
            if t.done || done_t > 0.02 {
                p.text(
                    center,
                    Align2::CENTER_CENTER,
                    "✓",
                    FontId::new(10.0, FontFamily::Monospace),
                    rgba(self.theme.bg, done_t),
                );
            }
            if nresp.clicked()
                && self.filter != Filter::Trash
                && self.link_from.is_none()
                && self.link_drag.is_none()
            {
                toggle_id = Some(t.id.clone());
            }
            if nresp.drag_started() && self.filter != Filter::Trash {
                self.link_drag = Some(t.id.clone());
                self.selected = Some(t.id.clone());
            }

            // Chevron for expand/collapse.
            let chev = Rect::from_center_size(
                Pos2::new(row.left() + TITLE_X - 17.0, row.top() + 18.0),
                Vec2::new(18.0, 24.0),
            );
            let cresp = ui.interact(chev, Id::unique(("chev", &t.id)), Sense::click());
            if cresp.hovered() {
                ui.painter()
                    .rect_filled(chev, CornerRadius::same(4), rgba(self.theme.fg, 0.06));
            }
            {
                let c = chev.center();
                let col = rgba(self.theme.fg, if cresp.hovered() { 0.9 } else { 0.48 });
                let stroke = Stroke::new(1.5, col);
                if anim > 0.5 {
                    ui.painter().line_segment(
                        [Pos2::new(c.x - 4.0, c.y - 2.0), Pos2::new(c.x, c.y + 2.5)],
                        stroke,
                    );
                    ui.painter().line_segment(
                        [Pos2::new(c.x, c.y + 2.5), Pos2::new(c.x + 4.0, c.y - 2.0)],
                        stroke,
                    );
                } else {
                    ui.painter().line_segment(
                        [Pos2::new(c.x - 2.0, c.y - 4.0), Pos2::new(c.x + 2.5, c.y)],
                        stroke,
                    );
                    ui.painter().line_segment(
                        [Pos2::new(c.x + 2.5, c.y), Pos2::new(c.x - 2.0, c.y + 4.0)],
                        stroke,
                    );
                }
            }
            if cresp.clicked() {
                expand_id = Some(t.id.clone());
            }

            // Title.
            let title_right = if self.filter == Filter::Trash {
                row.right() - 202.0
            } else {
                row.right() - 155.0
            };
            let title_clip = Rect::from_min_max(
                Pos2::new(row.left() + TITLE_X, row.top() + 6.0),
                Pos2::new(title_right, row.top() + 34.0),
            );
            let title_color = mix(
                self.theme.fg,
                self.theme.dim,
                if t.done { 0.55 } else { 0.0 },
            );
            paint_text(
                &p,
                Pos2::new(row.left() + TITLE_X, row.top() + 18.0),
                &t.title,
                15.0,
                title_color,
                false,
                title_clip,
            );
            let tw = text_width(ui, &t.title, 15.0, false).min(title_clip.width());
            if done_t > 0.01 {
                let line = Rect::from_min_size(
                    Pos2::new(row.left() + TITLE_X, row.top() + 18.0),
                    Vec2::new(tw * done_t, 1.5),
                );
                p.rect_filled(line, CornerRadius::ZERO, rgba(self.theme.fg, 0.55));
            }

            // Parent chip + tags.
            if m.meta_h > 0.0 {
                let mut mx = row.left() + TITLE_X;
                let meta_y = row.top() + m.meta_y + 9.0;
                let max_x = row.right() - 135.0;
                if !t.parent.is_empty() {
                    let ptitle = titles
                        .get(&t.parent)
                        .cloned()
                        .unwrap_or_else(|| "missing".into());
                    let label = format!("↳ {}", truncate(&ptitle, 24));
                    let w = text_width(ui, &label, 11.0, false) + 12.0;
                    let rect = Rect::from_min_size(
                        Pos2::new(mx, meta_y - 8.0),
                        Vec2::new(w.min(max_x - mx), 16.0),
                    );
                    if rect.width() > 8.0 {
                        let pr = ui.interact(rect, Id::unique(("parent", &t.id)), Sense::click());
                        ui.painter().rect_filled(
                            rect,
                            CornerRadius::same(4),
                            rgba(self.theme.accent, if pr.hovered() { 0.16 } else { 0.08 }),
                        );
                        ui.painter().text(
                            Pos2::new(rect.left() + 6.0, rect.center().y),
                            Align2::LEFT_CENTER,
                            &label,
                            FontId::new(11.0, FontFamily::Monospace),
                            rgba(self.theme.accent, if pr.hovered() { 1.0 } else { 0.78 }),
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
                            rgba(self.theme.fg, 0.4),
                        );
                        break;
                    }
                    let label = format!("#{tag}");
                    let w = text_width(ui, &label, 11.0, false) + 12.0;
                    let rect = Rect::from_min_size(Pos2::new(mx, meta_y - 8.0), Vec2::new(w, 16.0));
                    let active = self
                        .tag_filter
                        .as_deref()
                        .map(|x| x.eq_ignore_ascii_case(tag))
                        .unwrap_or(false);
                    let tr = ui.interact(rect, Id::unique(("tag", &t.id, tag)), Sense::click());
                    ui.painter().rect_filled(
                        rect,
                        CornerRadius::same(4),
                        rgba(
                            self.theme.accent2,
                            if active {
                                0.22
                            } else if tr.hovered() {
                                0.15
                            } else {
                                0.07
                            },
                        ),
                    );
                    ui.painter().text(
                        Pos2::new(rect.left() + 6.0, rect.center().y),
                        Align2::LEFT_CENTER,
                        &label,
                        FontId::new(11.0, FontFamily::Monospace),
                        rgba(
                            self.theme.accent2,
                            if active || tr.hovered() { 1.0 } else { 0.75 },
                        ),
                    );
                    if tr.clicked() {
                        tag_click = Some(tag.clone());
                    }
                    mx = rect.right() + 5.0;
                    shown += 1;
                }
                let _ = shown;
            }

            // Notes: one-line preview when collapsed, wrapped when open.
            if !t.notes.is_empty() && m.notes_h > 1.0 {
                let notes_clip = Rect::from_min_max(
                    Pos2::new(row.left() + TITLE_X, row.top() + m.notes_y),
                    Pos2::new(row.right() - 130.0, row.top() + m.notes_y + m.notes_h + 2.0),
                );
                if anim > 0.55 {
                    let galley = ui.painter().layout(
                        t.notes.clone(),
                        FontId::new(13.0, FontFamily::Monospace),
                        rgba(self.theme.fg, 0.56),
                        notes_clip.width(),
                    );
                    let text_pos = Pos2::new(notes_clip.left(), notes_clip.top());
                    ui.painter().with_clip_rect(notes_clip).galley(
                        text_pos,
                        galley,
                        rgba(self.theme.fg, 0.56),
                    );
                } else {
                    paint_text(
                        &p,
                        Pos2::new(notes_clip.left(), notes_clip.top() + 8.0),
                        &one_line(&t.notes),
                        13.0,
                        rgba(self.theme.fg, 0.45),
                        false,
                        notes_clip,
                    );
                }
            }

            // Expanded actions.
            if m.actions_h > 12.0 && anim > 0.55 {
                let ay = row.top() + m.actions_y + 2.0;
                let mut ax = row.left() + TITLE_X;
                let by = Rect::from_min_size(Pos2::new(ax, ay), Vec2::new(76.0, 22.0));
                if action_button(ui, by, "+ Subtask", self.theme.accent2, "Add a child task") {
                    subtask_id = Some(t.id.clone());
                }
                ax += 82.0;
                if !t.parent.is_empty() {
                    let r = Rect::from_min_size(Pos2::new(ax, ay), Vec2::new(64.0, 22.0));
                    if action_button(ui, r, "Unlink", self.theme.dim, "Detach from parent") {
                        unlink_id = Some(t.id.clone());
                    }
                    ax += 70.0;
                }
                let label = if t.done { "Reopen" } else { "Mark done" };
                let r = Rect::from_min_size(Pos2::new(ax, ay), Vec2::new(82.0, 22.0));
                if action_button(ui, r, label, self.theme.ok, "Toggle completion") {
                    mark_id = Some((t.id.clone(), !t.done));
                }
                ax += 88.0;
                if self.filter != Filter::Trash {
                    let r = Rect::from_min_size(Pos2::new(ax, ay), Vec2::new(58.0, 22.0));
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
                Pos2::new(rx, row.top() + 18.0),
                Align2::RIGHT_CENTER,
                &age_text,
                FontId::new(11.0, FontFamily::Monospace),
                rgba(self.theme.fg, 0.36),
            );
            rx -= text_width(ui, &age_text, 11.0, false) + 8.0;

            if self.filter != Filter::Trash {
                if t.remind_at > 0 {
                    let bell = Rect::from_center_size(
                        Pos2::new(rx - 8.0, row.top() + 18.0),
                        Vec2::new(18.0, 20.0),
                    );
                    let br = ui.interact(bell, Id::unique(("bell", &t.id)), Sense::click());
                    let col = if t.notified {
                        rgba(self.theme.fg, 0.32)
                    } else if t.remind_at <= now_ms() {
                        self.theme.danger
                    } else {
                        self.theme.accent
                    };
                    ui.painter().text(
                        bell.center(),
                        Align2::CENTER_CENTER,
                        "\u{f0f3}",
                        FontId::new(12.0, FontFamily::Monospace),
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
                    let label = due_label(t.due);
                    let color = if overdue {
                        self.theme.danger
                    } else {
                        rgba(self.theme.fg, 0.62)
                    };
                    let w = text_width(ui, &label, 11.0, false) + 12.0;
                    let rect = Rect::from_center_size(
                        Pos2::new(rx - w / 2.0, row.top() + 18.0),
                        Vec2::new(w, 20.0),
                    );
                    let dr = ui.interact(rect, Id::unique(("due", &t.id)), Sense::click());
                    ui.painter().rect_filled(
                        rect,
                        CornerRadius::same(4),
                        rgba(color, if dr.hovered() { 0.16 } else { 0.07 }),
                    );
                    ui.painter().text(
                        rect.center(),
                        Align2::CENTER_CENTER,
                        &label,
                        FontId::new(11.0, FontFamily::Monospace),
                        color,
                    );
                    if dr.clicked() {
                        due_id = Some(t.id.clone());
                    }
                    rx -= w + 6.0;
                } else if hovered {
                    let rect = Rect::from_center_size(
                        Pos2::new(rx - 13.0, row.top() + 18.0),
                        Vec2::new(26.0, 20.0),
                    );
                    let dr = ui.interact(rect, Id::unique(("adddue", &t.id)), Sense::click());
                    ui.painter().text(
                        rect.center(),
                        Align2::CENTER_CENTER,
                        "⏰",
                        FontId::new(12.0, FontFamily::Monospace),
                        rgba(self.theme.fg, if dr.hovered() { 0.85 } else { 0.45 }),
                    );
                    dr.clone().on_hover_text("Set due date");
                    if dr.clicked() {
                        due_id = Some(t.id.clone());
                    }
                    rx -= 30.0;
                }
            } else {
                let label = "Restore";
                let w = text_width(ui, label, 10.0, false) + 12.0;
                let rect = Rect::from_center_size(
                    Pos2::new(rx - w / 2.0, row.top() + 18.0),
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
                let w = text_width(ui, label, 10.0, false) + 12.0;
                let rect = Rect::from_center_size(
                    Pos2::new(rx - w / 2.0, row.top() + 18.0),
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

            // Hover affordances (drag / delete / edit / link).
            if self.filter != Filter::Trash
                && (hovered
                    || selected
                    || self.drag_id.as_deref() == Some(t.id.as_str())
                    || self.link_from.as_deref() == Some(t.id.as_str())
                    || self.link_drag.as_deref() == Some(t.id.as_str()))
            {
                let by = row.top() + 39.0;
                let mut bx = row.right() - 12.0;
                let drag = Rect::from_center_size(Pos2::new(bx - 8.0, by), Vec2::new(18.0, 22.0));
                let hr = ui.interact(drag, Id::unique(("drag", &t.id)), Sense::click_and_drag());
                ui.painter().text(
                    drag.center(),
                    Align2::CENTER_CENTER,
                    "⠿",
                    FontId::new(10.0, FontFamily::Monospace),
                    rgba(self.theme.fg, if hr.hovered() { 0.7 } else { 0.28 }),
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
                ui.painter().text(
                    del.center(),
                    Align2::CENTER_CENTER,
                    "✕",
                    FontId::new(11.0, FontFamily::Monospace),
                    rgba(self.theme.danger, if dr.hovered() { 1.0 } else { 0.55 }),
                );
                dr.clone().on_hover_text("Move to trash");
                if dr.clicked() {
                    delete_id = Some(t.id.clone());
                }
                bx -= 24.0;
                let ed = Rect::from_center_size(Pos2::new(bx - 7.0, by), Vec2::new(18.0, 22.0));
                let er = ui.interact(ed, Id::unique(("edit", &t.id)), Sense::click());
                ui.painter().text(
                    ed.center(),
                    Align2::CENTER_CENTER,
                    "✎",
                    FontId::new(11.0, FontFamily::Monospace),
                    rgba(self.theme.fg, if er.hovered() { 0.9 } else { 0.4 }),
                );
                er.clone().on_hover_text("Edit title and notes");
                if er.clicked() {
                    edit_id = Some(t.id.clone());
                }
                bx -= 24.0;
                let lk = Rect::from_center_size(Pos2::new(bx - 7.0, by), Vec2::new(18.0, 22.0));
                let lr = ui.interact(lk, Id::unique(("link", &t.id)), Sense::click());
                let active = self.link_from.as_deref() == Some(t.id.as_str());
                ui.painter().text(
                    lk.center(),
                    Align2::CENTER_CENTER,
                    "\u{f0c1}",
                    FontId::new(12.0, FontFamily::Monospace),
                    rgba(
                        self.theme.accent2,
                        if active {
                            1.0
                        } else if lr.hovered() {
                            0.9
                        } else {
                            0.4
                        },
                    ),
                );
                lr.clone()
                    .on_hover_text("Link: click a target task (or drag the node)");
                if lr.clicked() {
                    link_start = Some(t.id.clone());
                }
            }
            let _ = rx;
        }

        if self.drag_id.is_some() && self.drag_y > 0.0 {
            let p = ui.painter();
            let y = self.drag_y.clamp(content_rect.top(), content_rect.bottom());
            p.rect_filled(
                Rect::from_min_size(
                    Pos2::new(content_rect.left() + TITLE_X, y - 1.0),
                    Vec2::new(width - TITLE_X - 20.0, 2.0),
                ),
                CornerRadius::same(1),
                rgba(self.theme.accent, 0.8),
            );
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
            let (title, sub): (String, String) = if self.filter == Filter::Trash {
                (
                    "Trash is empty".into(),
                    "Deleted tasks will appear here".into(),
                )
            } else if !self.search.is_empty() || self.tag_filter.is_some() {
                (
                    "Nothing matches".into(),
                    format!(
                        "{}  ·  clear the filter to see everything",
                        if let Some(tag) = &self.tag_filter {
                            format!("#{tag}")
                        } else {
                            format!("“{}”", truncate(&self.search, 24))
                        }
                    ),
                )
            } else {
                (
                    "No tasks yet".into(),
                    "Type a task below and press Enter  ·  #tag to organise".into(),
                )
            };
            ui.painter().text(
                Pos2::new(c.x, c.y - 10.0),
                Align2::CENTER_CENTER,
                title,
                FontId::new(14.0, FontFamily::Monospace),
                rgba(self.theme.fg, 0.65),
            );
            ui.painter().text(
                Pos2::new(c.x, c.y + 14.0),
                Align2::CENTER_CENTER,
                sub,
                FontId::new(11.0, FontFamily::Monospace),
                rgba(self.theme.fg, 0.38),
            );
            if !self.search.is_empty() || self.tag_filter.is_some() {
                let w = 108.0;
                let rect = Rect::from_center_size(Pos2::new(c.x, c.y + 48.0), Vec2::new(w, 24.0));
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
            self.due_edit = Some((
                id,
                if due > 0 {
                    chrono::Local
                        .timestamp_millis_opt(due)
                        .single()
                        .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
                        .unwrap_or_default()
                } else {
                    String::new()
                },
            ));
        }
        if let Some((id, title, notes)) = edit_id.and_then(|id| {
            self.task(&id)
                .map(|t| (id.clone(), t.title.clone(), t.notes.clone()))
        }) {
            self.edit = Some((id, title, notes));
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
    }

    fn ui_batch(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        let trash = self.filter == Filter::Trash;
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(format!("{} selected", self.multi.len()))
                    .size(13.0)
                    .color(self.theme.accent)
                    .strong(),
            );
            ui.add_space(8.0);
            if trash {
                if chip_button(
                    ui,
                    "↺ Restore",
                    self.theme.ok,
                    rgba(self.theme.ok, 0.10),
                    "Restore selected tasks",
                ) {
                    let ids: Vec<String> = self.multi.iter().cloned().collect();
                    self.restore_many(&ids);
                }
                if chip_button(
                    ui,
                    "✕ Delete forever",
                    self.theme.danger,
                    rgba(self.theme.danger, 0.10),
                    "Permanently delete selected tasks",
                ) {
                    let ids: Vec<String> = self.multi.iter().cloned().collect();
                    self.purge_many(&ids);
                }
            } else {
                if chip_button(
                    ui,
                    "✓ Done",
                    self.theme.ok,
                    rgba(self.theme.ok, 0.10),
                    "Complete selected",
                ) {
                    let ids: Vec<String> = self.multi.iter().cloned().collect();
                    self.set_done_many(&ids, true);
                    self.multi.clear();
                }
                if chip_button(
                    ui,
                    "↺ Reopen",
                    self.theme.fg,
                    rgba(self.theme.fg, 0.07),
                    "Reopen selected",
                ) {
                    let ids: Vec<String> = self.multi.iter().cloned().collect();
                    self.set_done_many(&ids, false);
                    self.multi.clear();
                }
                if chip_button(
                    ui,
                    "✕ Trash",
                    self.theme.danger,
                    rgba(self.theme.danger, 0.10),
                    "Move selected to trash",
                ) {
                    let ids: Vec<String> = self.multi.iter().cloned().collect();
                    self.delete_many(&ids);
                }
            }
            if chip_button(
                ui,
                "Clear",
                self.theme.dim,
                rgba(self.theme.fg, 0.04),
                "Clear selection (Esc)",
            ) {
                self.multi.clear();
            }
        });
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new("ctrl+click to add · esc to clear")
                .size(11.0)
                .color(rgba(self.theme.fg, 0.35)),
        );
    }

    fn ui_input(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        let (line, _) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), Sense::hover());
        ui.painter()
            .rect_filled(line, CornerRadius::ZERO, rgba(self.theme.fg, 0.05));
        ui.add_space(8.0);
        let mut enter = false;
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("❯")
                    .size(18.0)
                    .color(if self.input.starts_with('/') {
                        self.theme.accent2
                    } else {
                        self.theme.accent
                    })
                    .strong(),
            );
            let hint = if self.read_only {
                "state file is read-only — fix it and press Retry"
            } else if let Some(p) = &self.input_parent {
                let _ = p;
                "subtask title…    esc to cancel"
            } else if self.input.starts_with('/') {
                "filter…    #tag works too"
            } else {
                "add task…    / to filter    #tag"
            };
            let edit = egui::TextEdit::singleline(&mut self.input)
                .id(Id::unique("task-input"))
                .hint_text(hint)
                .font(FontId::new(15.0, FontFamily::Monospace))
                .text_color(self.theme.fg)
                .frame(egui::Frame::NONE)
                .desired_width(f32::INFINITY)
                .interactive(!self.read_only);
            let resp = ui.add_sized([ui.available_width() - 4.0, 28.0], edit);
            let underline = Rect::from_min_size(
                Pos2::new(resp.rect.left(), resp.rect.bottom() + 2.0),
                Vec2::new(resp.rect.width(), 1.5),
            );
            ui.painter().rect_filled(
                underline,
                CornerRadius::ZERO,
                rgba(
                    if self.input.starts_with('/') {
                        self.theme.accent2
                    } else {
                        self.theme.accent
                    },
                    if resp.has_focus() { 0.65 } else { 0.22 },
                ),
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
        if enter {
            if let Some(q) = self.input.strip_prefix('/') {
                self.search = q.trim().to_string();
                self.input.clear();
            } else if !self.input.trim().is_empty() && !self.read_only {
                let text = self.input.clone();
                let parent = self.input_parent.take().unwrap_or_default();
                self.input.clear();
                self.add(&text, &parent);
                self.focus_input = true;
            }
        }
        ui.horizontal(|ui| {
            let hint = if self.link_from.is_some() {
                "click a task to link — drag a node also works · esc cancels".to_string()
            } else if let Some(parent) = &self.input_parent {
                let title = self
                    .task(parent)
                    .map(|t| truncate(&t.title, 24))
                    .unwrap_or_default();
                format!("subtask of «{title}» · esc cancels")
            } else if self.search.is_empty() && self.tag_filter.is_none() {
                "↵ add   ·   / filter   ·   #tag   ·   ctrl+z undo   ·   esc hide".to_string()
            } else {
                "↵ keep filter   ·   esc clears   ·   click filter chips to remove".to_string()
            };
            ui.label(
                egui::RichText::new(hint)
                    .size(11.0)
                    .color(rgba(self.theme.fg, 0.36)),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let (dot, dresp) = ui.allocate_exact_size(Vec2::new(14.0, 14.0), Sense::hover());
                ui.painter().circle_filled(
                    dot.center(),
                    3.0,
                    if self.daemon_present {
                        rgba(self.theme.ok, 0.8)
                    } else {
                        rgba(self.theme.danger, 0.7)
                    },
                );
                dresp.on_hover_text(if self.daemon_present {
                    "Tray daemon"
                } else {
                    "Daemon not running"
                });
                if !self.search.is_empty() || self.tag_filter.is_some() {
                    if ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new("clear filter ✕")
                                    .size(11.0)
                                    .color(rgba(self.theme.fg, 0.55)),
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
                    egui::RichText::new("title")
                        .size(11.0)
                        .color(self.theme.dim),
                );
                let tr = ui.add_sized(
                    [ui.available_width(), 30.0],
                    egui::TextEdit::singleline(&mut title)
                        .font(FontId::new(15.0, FontFamily::Monospace)),
                );
                tr.request_focus();
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new("notes  ·  #tags are parsed automatically")
                        .size(11.0)
                        .color(self.theme.dim),
                );
                ui.add_sized(
                    [ui.available_width(), 100.0],
                    egui::TextEdit::multiline(&mut notes)
                        .font(FontId::new(13.0, FontFamily::Monospace)),
                );
                let tags = model::parse_tags(&format!("{title} {notes}"));
                if !tags.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        for tag in tags {
                            ui.label(
                                egui::RichText::new(format!("#{tag}"))
                                    .size(11.0)
                                    .color(self.theme.accent2),
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
                            .fill(rgba(self.theme.accent, 0.10)),
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
                                    .color(self.theme.dim),
                            )
                            .frame(false),
                        )
                        .clicked()
                    {
                        cancel = true;
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new("ctrl+↵ save · esc cancel")
                                .size(10.0)
                                .color(rgba(self.theme.fg, 0.32)),
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
        let Some((id, draft)) = self.due_edit.clone() else {
            return;
        };
        let mut open = true;
        let mut draft = draft;
        let mut apply: Option<Option<i64>> = None;
        let title = self.task(&id).map(|t| t.title.clone()).unwrap_or_default();
        egui::Window::new("Due date")
            .collapsible(false)
            .resizable(false)
            .default_width(420.0)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(
                    egui::RichText::new(truncate(&title, 42))
                        .size(12.0)
                        .color(self.theme.dim),
                );
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    for (label, val) in [
                        ("Today 18:00", "today 18:00"),
                        ("Tomorrow 9:00", "tomorrow 09:00"),
                        ("+1h", "+1h"),
                        ("+3d", "+3d"),
                    ] {
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new(label).size(11.0).color(self.theme.fg),
                                )
                                .fill(rgba(self.theme.surface, 0.95)),
                            )
                            .clicked()
                        {
                            draft = val.to_string();
                        }
                    }
                });
                ui.add_space(8.0);
                let resp = ui.add_sized(
                    [ui.available_width(), 30.0],
                    egui::TextEdit::singleline(&mut draft)
                        .hint_text("2026-10-27 18:00 · +2h · tomorrow 09:00"),
                );
                ui.add_space(2.0);
                if draft.trim().is_empty() {
                    ui.label(
                        egui::RichText::new("no due date")
                            .size(11.0)
                            .color(self.theme.dim),
                    );
                } else if let Some(ms) = parse_when(&draft) {
                    ui.label(
                        egui::RichText::new(format!(
                            "{}  ·  reminder 10 min before",
                            due_label(ms)
                        ))
                        .size(11.0)
                        .color(self.theme.accent),
                    );
                } else {
                    ui.label(
                        egui::RichText::new("couldn't parse that")
                            .size(11.0)
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
                            .frame(false),
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
                                    .color(self.theme.dim),
                            )
                            .frame(false),
                        )
                        .clicked()
                    {
                        self.due_edit = None;
                    }
                    if ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new("Save")
                                    .size(12.0)
                                    .color(self.theme.accent),
                            )
                            .fill(rgba(self.theme.accent, 0.10)),
                        )
                        .clicked()
                        || (resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                    {
                        match parse_when(&draft) {
                            Some(ms) => apply = Some(Some(ms)),
                            None => {}
                        }
                    }
                });
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    self.due_edit = None;
                }
            });
        if !open {
            self.due_edit = None;
        }
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
            if let Some((_, d)) = self.due_edit.as_mut() {
                *d = draft;
            }
        }
    }

    fn ui_toast(&mut self, ctx: &egui::Context) {
        let Some((msg, at)) = self.toast.clone() else {
            return;
        };
        if at.elapsed() > Duration::from_millis(2600) {
            self.toast = None;
            return;
        }
        egui::Area::new(Id::unique("toast"))
            .order(egui::Order::Foreground)
            .anchor(Align2::CENTER_BOTTOM, [0.0, -86.0])
            .interactable(false)
            .show(ctx, |ui| {
                egui::Frame::NONE
                    .fill(rgba(self.theme.surface, 0.98))
                    .stroke(Stroke::new(1.0, rgba(self.theme.accent, 0.25)))
                    .corner_radius(CornerRadius::same(6))
                    .inner_margin(egui::Margin::symmetric(12, 7))
                    .show(ui, |ui| {
                        ui.label(
                            egui::RichText::new(&msg)
                                .size(12.0)
                                .color(rgba(self.theme.fg, 0.9)),
                        );
                    });
            });
        ctx.request_repaint_after(Duration::from_millis(100));
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
        if ctx.input(|i| i.key_pressed(egui::Key::Escape))
            && self.edit.is_none()
            && self.due_edit.is_none()
        {
            if self.link_from.is_some() || self.link_drag.is_some() {
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
        let typing = ctx.egui_wants_keyboard_input();
        if !typing && ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::A)) {
            let ids: Vec<String> = self.visible().iter().map(|t| t.id.clone()).collect();
            self.multi = ids.into_iter().collect();
        }
        if (!typing || self.input.is_empty())
            && ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Z))
        {
            if ctx.input(|i| i.modifiers.shift) {
                self.redo();
            } else {
                self.undo();
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
            && ctx.input(|i| i.key_pressed(egui::Key::Slash))
            && !self.read_only
        {
            self.input = "/".into();
            self.focus_input = true;
        }
        if let Some(id) = self.selected.clone() {
            if !typing
                && self.filter != Filter::Trash
                && ui.input(|i| i.key_pressed(egui::Key::Delete))
            {
                self.soft_delete(&id);
            }
        }
        if self.dirty && self.last_save.elapsed() > Duration::from_millis(300) {
            self.save_now();
        }

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
            self.ui_active_filters(ui);
            self.ui_error_banner(ui);
            let reserved = if self.multi.is_empty() { 92.0 } else { 92.0 };
            let list_h = (ui.available_height() - reserved).max(90.0);
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .max_height(list_h)
                .show(ui, |ui| {
                    self.ui_list(ui);
                });
            if self.multi.is_empty() {
                self.ui_input(ui);
            } else {
                self.ui_batch(ui);
            }
        });
        self.ui_edit_window(&ctx);
        self.ui_due_window(&ctx);
        self.ui_toast(&ctx);

        if self.link_from.is_some() || self.link_drag.is_some() {
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

fn action_button(ui: &mut egui::Ui, rect: Rect, label: &str, color: Color32, tip: &str) -> bool {
    let resp = ui.interact(
        rect,
        Id::unique(("action", label, rect.min.y as i32)),
        Sense::click(),
    );
    let hovered = resp.hovered();
    ui.painter().rect_filled(
        rect,
        CornerRadius::same(4),
        rgba(color, if hovered { 0.16 } else { 0.06 }),
    );
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        FontId::new(11.0, FontFamily::Monospace),
        rgba(color, if hovered { 1.0 } else { 0.92 }),
    );
    resp.on_hover_text(tip).clicked()
}

fn chip_button(ui: &mut egui::Ui, label: &str, color: Color32, fill: Color32, tip: &str) -> bool {
    let w = text_width(ui, label, 11.0, false) + 14.0;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 21.0), Sense::click());
    ui.painter().rect_filled(
        rect,
        CornerRadius::same(4),
        if resp.hovered() {
            rgba(color, 0.20)
        } else {
            fill
        },
    );
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        FontId::new(11.0, FontFamily::Monospace),
        rgba(color, if resp.hovered() { 1.0 } else { 0.8 }),
    );
    resp.on_hover_text(tip).clicked()
}

fn chip_rect(ui: &mut egui::Ui, rect: Rect, label: &str, color: Color32, id: Id) -> bool {
    let resp = ui.interact(rect, id, Sense::click());
    let hovered = resp.hovered();
    ui.painter().rect_filled(
        rect,
        CornerRadius::same(4),
        rgba(color, if hovered { 0.22 } else { 0.11 }),
    );
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        FontId::new(10.0, FontFamily::Monospace),
        rgba(color, if hovered { 1.0 } else { 0.9 }),
    );
    resp.clicked()
}

fn icon_button(ui: &mut egui::Ui, size: Vec2, glyph: &str, color: Color32, tip: &str) -> bool {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    if resp.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(4), rgba(color, 0.10));
    }
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        glyph,
        FontId::new(16.0, FontFamily::Monospace),
        rgba(color, if resp.hovered() { 1.0 } else { 0.65 }),
    );
    resp.on_hover_text(tip).clicked()
}
