//! Pressed shortcuts shown over the views, for screen recordings.

use std::time::{Duration, Instant};

use egui::text::{LayoutJob, TextFormat};
use egui::{Align2, Color32, FontId, Stroke, pos2, vec2};
use serde::{Deserialize, Serialize};

use crate::app::MapCraftApp;
use crate::chrome::{ACCENT, HEADER, LINE, TEXT, TEXT_WEAK};

/// How long a press shows at full strength, then how long it takes to fade out.
const HOLD: Duration = Duration::from_millis(1500);
const FADE: Duration = Duration::from_millis(400);
const MAX_SHOWN: usize = 4;

/// The corner of the views pressed keys show in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl Corner {
    pub const ALL: [Corner; 4] = [Corner::TopLeft, Corner::TopRight, Corner::BottomLeft, Corner::BottomRight];

    pub fn label(self) -> &'static str {
        match self {
            Corner::TopLeft => "Top Left",
            Corner::TopRight => "Top Right",
            Corner::BottomLeft => "Bottom Left",
            Corner::BottomRight => "Bottom Right",
        }
    }

    fn align(self) -> Align2 {
        match self {
            Corner::TopLeft => Align2::LEFT_TOP,
            Corner::TopRight => Align2::RIGHT_TOP,
            Corner::BottomLeft => Align2::LEFT_BOTTOM,
            Corner::BottomRight => Align2::RIGHT_BOTTOM,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Press {
    pub chord: String,
    pub label: &'static str,
    pub count: u32,
    pub at: Instant,
}

/// The last few presses, oldest first.
#[derive(Default)]
pub(crate) struct Presses(Vec<Press>);

impl Presses {
    /// Adds a press, or counts it on the newest one when it repeats that.
    pub(crate) fn push(&mut self, chord: String, label: &'static str, now: Instant) {
        match self.0.last_mut() {
            Some(p) if p.chord == chord && p.label == label => {
                p.count = p.count.saturating_add(1);
                p.at = now;
            }
            _ => self.0.push(Press { chord, label, count: 1, at: now }),
        }
        if self.0.len() > MAX_SHOWN {
            self.0.remove(0);
        }
    }

    /// Drops faded presses and says when the next frame is needed: now while one fades, `None` once all are gone.
    pub(crate) fn expire(&mut self, now: Instant) -> Option<Duration> {
        self.0.retain(|p| now.saturating_duration_since(p.at) < HOLD + FADE);
        self.0.iter().map(|p| HOLD.saturating_sub(now.saturating_duration_since(p.at))).min()
    }

    /// The presses newest first, each with its opacity.
    pub(crate) fn rows(&self, now: Instant) -> impl Iterator<Item = (&Press, f32)> {
        self.0.iter().rev().map(move |p| {
            let left = (HOLD + FADE).saturating_sub(now.saturating_duration_since(p.at));
            (p, (left.as_secs_f32() / FADE.as_secs_f32()).min(1.0))
        })
    }
}

impl MapCraftApp {
    /// Notes a shortcut press for the pressed keys display, when it is on.
    pub(crate) fn note_key(&mut self, chord: String, label: &'static str) {
        if self.prefs.get().view.show_keys.is_some() && !chord.is_empty() {
            self.keys.push(chord, label, Instant::now());
        }
    }

    /// Stacks the last shortcuts pressed in the chosen corner of the views, newest nearest it.
    pub(crate) fn draw_keys(&mut self, ui: &egui::Ui) {
        let Some(corner) = self.prefs.get().view.show_keys else { return };
        let now = Instant::now();
        if let Some(wait) = self.keys.expire(now) {
            ui.ctx().request_repaint_after(wait);
        }
        let full = ui.max_rect();
        let align = corner.align();
        let (left, top) = (align.x() == egui::Align::Min, align.y() == egui::Align::Min);
        let pane = self.maximized.unwrap_or(match corner {
            Corner::TopLeft => 0,
            Corner::TopRight => 1,
            Corner::BottomLeft => 2,
            Corner::BottomRight => 3,
        });
        // The 3D view's axis widget fills its bottom left corner.
        let gizmo = corner == Corner::BottomLeft && self.panes.get(pane).is_some_and(|p| p.uv.is_none() && !p.camera.is_2d());
        let margin = 10.0;
        let x = if left { full.left() + margin + if gizmo { 64.0 } else { 0.0 } } else { full.right() - margin };
        // Below the views' 22 pixel title bars.
        let mut y = if top { full.top() + 22.0 + margin } else { full.bottom() - margin };
        let painter = ui.painter_at(full);
        for (press, alpha) in self.keys.rows(now) {
            let format = |size: f32, color: Color32| TextFormat::simple(FontId::proportional(size), color.gamma_multiply(alpha));
            let mut job = LayoutJob::default();
            job.append(&press.chord, 0.0, format(15.0, TEXT));
            job.append(press.label, 10.0, format(13.0, TEXT_WEAK));
            if press.count > 1 {
                job.append(&format!("x{}", press.count), 8.0, format(13.0, ACCENT));
            }
            let galley = painter.layout_job(job);
            let rect = align.anchor_size(pos2(x, y), galley.size() + vec2(20.0, 10.0));
            painter.rect(rect, 6.0, HEADER.gamma_multiply(alpha), Stroke::new(1.0, LINE.gamma_multiply(alpha)), egui::StrokeKind::Inside);
            painter.galley(rect.min + vec2(10.0, 5.0), galley, TEXT);
            y += if top { rect.height() + 4.0 } else { -rect.height() - 4.0 };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeats_collapse_and_presses_fade_out() {
        let t = Instant::now();
        let mut keys = Presses::default();
        for chord in ["B", "Q", "Q", "Q"] {
            keys.push(chord.into(), "Tool", t);
        }
        let rows: Vec<_> = keys.rows(t).map(|(p, a)| (p.chord.clone(), p.count, a)).collect();
        assert_eq!(rows, vec![("Q".into(), 3, 1.0), ("B".into(), 1, 1.0)], "newest first, repeats counted");
        for chord in ["1", "2", "3", "4"] {
            keys.push(chord.into(), "Sculpt", t);
        }
        assert_eq!(keys.rows(t).map(|(p, _)| p.chord.as_str()).collect::<Vec<_>>(), ["4", "3", "2", "1"]);
        assert_eq!(keys.expire(t), Some(HOLD), "idle until the fade starts");
        let fading = t + HOLD + FADE / 2;
        assert_eq!(keys.expire(fading), Some(Duration::ZERO));
        assert!(keys.rows(fading).all(|(_, a)| a > 0.0 && a < 1.0));
        assert_eq!(keys.expire(t + HOLD + FADE), None, "gone once faded, so no more repaints");
        assert_eq!(keys.rows(t).count(), 0);
    }
}
