use crate::session;
use serde_json::Value;

#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

fn hyprctl_json(args: &[&str]) -> Option<Value> {
    let out = session::command("hyprctl").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    serde_json::from_slice(&out.stdout).ok()
}

pub fn focused_monitor() -> Option<Rect> {
    let v = hyprctl_json(&["monitors", "-j"])?;
    let arr = v.as_array()?;
    let m = arr
        .iter()
        .find(|m| m["focused"].as_bool().unwrap_or(false))
        .or_else(|| arr.first())?;
    Some(Rect {
        x: m["x"].as_f64()? as f32,
        y: m["y"].as_f64()? as f32,
        w: m["width"].as_f64()? as f32,
        h: m["height"].as_f64()? as f32,
    })
}

pub fn client_geometry() -> Option<Rect> {
    let v = hyprctl_json(&["clients", "-j"])?;
    let c = v.as_array()?.iter().find(|c| {
        c["class"].as_str() == Some("tasks-rs") || c["initialClass"].as_str() == Some("tasks-rs")
    })?;
    let at = c["at"].as_array()?;
    let size = c["size"].as_array()?;
    Some(Rect {
        x: at.first()?.as_f64()? as f32,
        y: at.get(1)?.as_f64()? as f32,
        w: size.first()?.as_f64()? as f32,
        h: size.get(1)?.as_f64()? as f32,
    })
}

pub fn top_right(w: f32) -> Option<(f32, f32)> {
    let m = focused_monitor()?;
    Some((m.x + m.w - w - 24.0, m.y + 24.0))
}

pub fn focus_client() -> bool {
    let Some(v) = hyprctl_json(&["clients", "-j"]) else {
        return false;
    };
    let Some(arr) = v.as_array() else {
        return false;
    };
    let Some(c) = arr.iter().find(|c| {
        c["class"].as_str() == Some("tasks-rs") || c["initialClass"].as_str() == Some("tasks-rs")
    }) else {
        return false;
    };
    let Some(addr) = c["address"].as_str() else {
        return false;
    };
    let arg = format!("hl.dsp.focus({{window=\"address:{addr}\"}})");
    session::command("hyprctl")
        .args(["dispatch", &arg])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn move_client(x: f32, y: f32) -> bool {
    let Some(v) = hyprctl_json(&["clients", "-j"]) else {
        return false;
    };
    let Some(arr) = v.as_array() else {
        return false;
    };
    let Some(c) = arr.iter().find(|c| {
        c["class"].as_str() == Some("tasks-rs") || c["initialClass"].as_str() == Some("tasks-rs")
    }) else {
        return false;
    };
    let Some(addr) = c["address"].as_str() else {
        return false;
    };
    let arg = format!(
        "hl.dsp.window.move({{x={},y={},window=\"address:{}\"}})",
        x.round() as i32,
        y.round() as i32,
        addr
    );
    session::command("hyprctl")
        .args(["dispatch", &arg])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
