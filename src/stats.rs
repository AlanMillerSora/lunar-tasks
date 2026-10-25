use chrono::{Local, TimeZone};

use crate::model::{Doc, Task};

/// Completed tasks per local calendar day, oldest first.
///
/// The result always has exactly `days` entries and the last one is the day
/// containing `now`. Deleted tasks and tasks without a completion time are
/// ignored.
pub fn activity_counts(doc: &Doc, days: usize, now: i64) -> Vec<u32> {
    let mut counts = vec![0u32; days];
    if days == 0 {
        return counts;
    }
    let Some(today) = Local.timestamp_millis_opt(now).single() else {
        return counts;
    };
    let today = today.date_naive();
    for task in doc.items.iter().filter(|t| t.deleted_at == 0) {
        if task.completed_at <= 0 {
            continue;
        }
        let Some(done) = Local.timestamp_millis_opt(task.completed_at).single() else {
            continue;
        };
        let offset = (today - done.date_naive()).num_days();
        if offset < 0 || offset >= days as i64 {
            continue;
        }
        let idx = days - 1 - offset as usize;
        counts[idx] = counts[idx].saturating_add(1);
    }
    counts
}

/// Export the visible task tree as a markdown checklist.
///
/// Roots and children keep document order. Deleted tasks are skipped; if a
/// task's parent was deleted, the task is promoted to a root so its subtree
/// is not lost.
pub fn doc_to_markdown(doc: &Doc) -> String {
    let visible: Vec<&Task> = doc.items.iter().filter(|t| t.deleted_at == 0).collect();
    let mut visited = vec![false; visible.len()];
    let mut out = String::new();
    for (idx, task) in visible.iter().enumerate() {
        let is_root = task.parent.is_empty() || !visible.iter().any(|p| p.id == task.parent);
        if is_root {
            render_task(idx, 0, &visible, &mut visited, &mut out);
        }
    }
    out
}

fn render_task(idx: usize, depth: usize, all: &[&Task], visited: &mut [bool], out: &mut String) {
    if visited[idx] {
        return;
    }
    visited[idx] = true;
    let task = all[idx];
    let indent = "  ".repeat(depth);

    out.push_str(&indent);
    out.push_str(if task.done { "- [x] " } else { "- [ ] " });
    out.push_str(&task.title);
    out.push('\n');

    let metadata = metadata_line(task);
    if !metadata.is_empty() {
        out.push_str(&indent);
        out.push_str("  ");
        out.push_str(&metadata);
        out.push('\n');
    }

    for line in task.notes.lines() {
        let line = line.trim_end();
        out.push_str(&indent);
        out.push_str("  >");
        if !line.is_empty() {
            out.push(' ');
            out.push_str(line);
        }
        out.push('\n');
    }

    for (child_idx, child) in all.iter().enumerate() {
        if child.parent == task.id {
            render_task(child_idx, depth + 1, all, visited, out);
        }
    }
}

fn metadata_line(task: &Task) -> String {
    let mut parts: Vec<String> = Vec::new();
    if task.due > 0 {
        if let Some(due) = Local.timestamp_millis_opt(task.due).single() {
            parts.push(due.format("%Y-%m-%d %H:%M").to_string());
        }
    }
    if let Some(priority) = priority_label(task.priority) {
        parts.push(priority.to_string());
    }
    let repeat = task.repeat.trim();
    if !repeat.is_empty() {
        parts.push(repeat.to_string());
    }
    let tags = task.tags();
    if !tags.is_empty() {
        parts.push(
            tags.iter()
                .map(|tag| format!("#{tag}"))
                .collect::<Vec<_>>()
                .join(" "),
        );
    }
    parts.join(" · ")
}

fn priority_label(priority: u8) -> Option<&'static str> {
    match priority {
        1 => Some("P1"),
        2 => Some("P2"),
        3 => Some("P3"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::new_task;

    fn local_ms(y: i32, m: u32, d: u32, h: u32, min: u32) -> i64 {
        Local
            .with_ymd_and_hms(y, m, d, h, min, 0)
            .unwrap()
            .timestamp_millis()
    }

    #[test]
    fn activity_counts_buckets_by_local_day() {
        let now = local_ms(2026, 6, 15, 12, 0);
        let today = local_ms(2026, 6, 15, 9, 0);
        let yesterday = local_ms(2026, 6, 14, 20, 0);
        let two_days_ago = local_ms(2026, 6, 13, 18, 0);
        let outside = local_ms(2026, 6, 12, 23, 0);

        let mut doc = Doc::default();
        let mut a = new_task("today", "");
        a.completed_at = today;
        let mut b = new_task("two days ago", "");
        b.completed_at = two_days_ago;
        let mut c = new_task("outside window", "");
        c.completed_at = outside;
        let mut d = new_task("deleted today", "");
        d.completed_at = today;
        d.deleted_at = now;
        let mut e = new_task("never completed", "");
        e.completed_at = 0;
        let mut f = new_task("yesterday", "");
        f.completed_at = yesterday;
        doc.items = vec![a, b, c, d, e, f];

        let counts = activity_counts(&doc, 3, now);
        assert_eq!(counts.len(), 3);
        assert_eq!(counts, vec![1, 1, 1]);
    }

    #[test]
    fn activity_counts_handles_empty_and_zero_days() {
        let now = local_ms(2026, 6, 15, 12, 0);
        assert!(activity_counts(&Doc::default(), 0, now).is_empty());
        assert_eq!(activity_counts(&Doc::default(), 4, now), vec![0; 4]);
    }

    #[test]
    fn activity_counts_accepts_multiple_tasks_per_day() {
        let now = local_ms(2026, 6, 15, 12, 0);
        let mut doc = Doc::default();
        for h in [8, 9, 10] {
            let mut t = new_task("done", "");
            t.completed_at = local_ms(2026, 6, 15, h, 0);
            doc.items.push(t);
        }
        assert_eq!(activity_counts(&doc, 2, now), vec![0, 3]);
    }

    #[test]
    fn markdown_exports_tree_with_metadata_and_notes() {
        let mut doc = Doc::default();

        let mut root = new_task("Root #work", "");
        root.id = "root".into();
        root.due = local_ms(2026, 5, 1, 9, 30);
        root.priority = 1;
        root.repeat = "daily".into();
        root.notes = "first line\nsecond #idea".into();

        let mut child = new_task("Child", "root");
        child.id = "child".into();
        child.done = true;

        let mut grandchild = new_task("Grandchild", "child");
        grandchild.id = "grandchild".into();

        let mut other = new_task("Other", "");
        other.id = "other".into();

        let mut gone = new_task("Gone", "");
        gone.deleted_at = 1;

        doc.items = vec![root, child, grandchild, other, gone];

        let got = doc_to_markdown(&doc);
        let want = "\
- [ ] Root #work
  2026-05-01 09:30 · P1 · daily · #work #idea
  > first line
  > second #idea
  - [x] Child
    - [ ] Grandchild
- [ ] Other
";
        assert_eq!(got, want);
        assert!(!got.contains("Gone"));
    }

    #[test]
    fn markdown_promotes_children_of_deleted_parents() {
        let mut doc = Doc::default();
        let mut gone = new_task("Gone", "");
        gone.id = "gone".into();
        gone.deleted_at = 7;
        let mut orphan = new_task("Orphan #keep", "gone");
        orphan.id = "orphan".into();
        doc.items = vec![gone, orphan];

        let got = doc_to_markdown(&doc);
        assert_eq!(got, "- [ ] Orphan #keep\n  #keep\n");
    }

    #[test]
    fn markdown_without_metadata_has_no_extra_line() {
        let mut doc = Doc::default();
        let mut t = new_task("Plain", "");
        t.id = "plain".into();
        doc.items.push(t);
        assert_eq!(doc_to_markdown(&doc), "- [ ] Plain\n");

        doc.items[0].done = true;
        doc.items[0].completed_at = 1;
        assert_eq!(doc_to_markdown(&doc), "- [x] Plain\n");
    }

    #[test]
    fn markdown_priority_labels() {
        let mut doc = Doc::default();
        for (i, priority) in [0u8, 1, 2, 3, 9].iter().enumerate() {
            let mut t = new_task(&format!("task{i}"), "");
            t.id = format!("p{i}");
            t.priority = *priority;
            doc.items.push(t);
        }
        let got = doc_to_markdown(&doc);
        assert!(got.contains("- [ ] task0\n"));
        assert!(got.contains("- [ ] task1\n  P1\n"));
        assert!(got.contains("- [ ] task2\n  P2\n"));
        assert!(got.contains("- [ ] task3\n  P3\n"));
        assert!(got.contains("- [ ] task4\n"));
    }
}
