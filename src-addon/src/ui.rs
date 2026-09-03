//! Drawing. Everything here reads the model and writes pixels; nothing here
//! decides anything.
//!
//! Sizes are in **points**, and one point is `scale` pixels. The panel is
//! 640×480 because that is the screen this runs on, and at 1.5× that leaves a
//! 427×320 point layout — the same figures the archived front end settled on
//! after being drawn too large three times and read wrong each time.

use moose_rack::layout::Rect;
use moose_sdl::gfx::Gfx;
use moose_sdl::text::{Painter, Spec};

use crate::model::{App, Kind, Overlay, Row, Tab};
use crate::sync::{Action, Review, Stage};

pub const PANEL: (u32, u32) = (640, 480);

/// Points to pixels. Text at 12pt on a 640×480 handheld held at arm's length
/// is not readable at 1:1.
pub const SCALE: f32 = 1.5;

/// One place for the palette, so a row that is "changed" is the same colour
/// everywhere it appears — in the list, in the counter and in the confirmation.
pub mod ink {
    use moose_sdl::gfx::Rgba;
    pub const BACKDROP: Rgba = Rgba(0.055, 0.063, 0.086, 1.0);
    pub const BAR: Rgba = Rgba(0.094, 0.106, 0.141, 1.0);
    pub const LINE: Rgba = Rgba(1.0, 1.0, 1.0, 0.08);
    pub const TEXT: Rgba = Rgba(0.88, 0.90, 0.94, 1.0);
    pub const DIM: Rgba = Rgba(0.55, 0.58, 0.65, 1.0);
    pub const FAINT: Rgba = Rgba(0.38, 0.41, 0.48, 1.0);
    pub const PICKED: Rgba = Rgba(0.20, 0.24, 0.34, 1.0);
    pub const ACCENT: Rgba = Rgba(0.55, 0.78, 0.45, 1.0);
    /// Queued but not run. Deliberately not the accent colour: "chosen" and
    /// "actually true on the device" must never look the same.
    pub const QUEUED: Rgba = Rgba(0.95, 0.72, 0.32, 1.0);
    pub const SHADE: Rgba = Rgba(0.0, 0.0, 0.0, 0.72);
}

/// All in points.
mod size {
    pub const NAME_BAR: f32 = 20.0;
    pub const TAB_BAR: f32 = 20.0;
    pub const HELP: f32 = 16.0;
    pub const ROW: f32 = 22.0;
    pub const PAD: f32 = 8.0;
    pub const TAB_WIDTH: f32 = 68.0;
    pub const TEXT: f32 = 11.0;
    pub const SMALL: f32 = 9.0;
}

pub struct Ui {
    pub scale: f32,
    /// Which row is at the top of the visible window. Kept here rather than in
    /// the model because it is a fact about the screen, not about the patches.
    pub first: usize,
}

impl Default for Ui {
    fn default() -> Self {
        Ui { scale: SCALE, first: 0 }
    }
}

impl Ui {
    /// Points to pixels.
    fn px(&self, points: f32) -> f32 {
        points * self.scale
    }

    fn rect(&self, x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::new(self.px(x), self.px(y), self.px(w), self.px(h))
    }

    fn spec(&self, text: &str, points: f32) -> Spec {
        Spec::new(text, points, self.scale)
    }

    /// The panel in points rather than pixels.
    fn panel_size(&self) -> (f32, f32) {
        (PANEL.0 as f32 / self.scale, PANEL.1 as f32 / self.scale)
    }

    pub fn draw(&mut self, gfx: &Gfx, painter: &mut Painter, app: &App) {
        let (w, h) = self.panel_size();
        gfx.clear(ink::BACKDROP);

        self.name_bar(gfx, painter, w);
        self.tab_bar(gfx, painter, w, app);

        let top = size::NAME_BAR + size::TAB_BAR;
        self.rows(gfx, painter, (top, h - top - size::HELP), w, app);
        self.help(gfx, painter, (h - size::HELP, w), app);

        match &app.overlay {
            Overlay::None => {}
            Overlay::Detail => self.detail(gfx, painter, app),
            Overlay::ConfirmApply => self.confirm_apply(gfx, painter, app),
            Overlay::ConfirmDiscard => self.confirm_discard(gfx, painter, app),
            Overlay::ConfirmAction { title } => self.confirm_action(gfx, painter, app, title),
            Overlay::Applying { done, total } => self.applying(gfx, painter, *done, *total),
        }
    }

    fn name_bar(&self, gfx: &Gfx, painter: &mut Painter, w: f32) {
        gfx.fill(self.rect(0.0, 0.0, w, size::NAME_BAR), ink::BAR);
        painter.put(
            gfx,
            &self.spec("moose-patch", size::TEXT),
            self.rect(size::PAD, 5.0, w - size::PAD * 2.0, size::NAME_BAR),
            ink::TEXT,
        );
    }

    fn tab_bar(&self, gfx: &Gfx, painter: &mut Painter, w: f32, app: &App) {
        let y = size::NAME_BAR;
        gfx.fill(self.rect(0.0, y, w, size::TAB_BAR), ink::BACKDROP);
        gfx.fill(self.rect(0.0, y + size::TAB_BAR - 1.0, w, 1.0), ink::LINE);

        for (i, tab) in Tab::ALL.iter().enumerate() {
            let x = size::PAD + i as f32 * size::TAB_WIDTH;
            let here = *tab == app.tab;
            if here {
                // The underline, not a filled block: a selected tab should
                // read as part of the page below it rather than a button.
                gfx.fill(
                    self.rect(x, y + size::TAB_BAR - 2.0, size::TAB_WIDTH - 12.0, 2.0),
                    ink::ACCENT,
                );
            }
            painter.put(
                gfx,
                &self.spec(tab.title(), size::SMALL),
                self.rect(x, y + 6.0, size::TAB_WIDTH, size::TAB_BAR),
                if here { ink::TEXT } else { ink::FAINT },
            );
        }
    }

    fn rows(&mut self, gfx: &Gfx, painter: &mut Painter, body: (f32, f32), w: f32, app: &App) {
        let (top, height) = body;
        let page = app.page();
        let fits = ((height - size::PAD) / size::ROW).floor().max(1.0) as usize;

        // Keep the cursor on screen without ever jumping further than it has
        // to — a list that recentres on every step is hard to read.
        if page.cursor < self.first {
            self.first = page.cursor;
        } else if page.cursor >= self.first + fits {
            self.first = page.cursor + 1 - fits;
        }
        let last = (self.first + fits).min(page.rows.len());

        for (n, row) in page.rows[self.first..last].iter().enumerate() {
            let y = top + size::PAD * 0.5 + n as f32 * size::ROW;
            let at = self.rect(size::PAD, y, w - size::PAD * 2.0, size::ROW);
            self.row(gfx, painter, at, row, self.first + n == page.cursor);
        }

        if last < page.rows.len() {
            painter.put_right(
                gfx,
                &self.spec("more below", size::SMALL),
                self.rect(0.0, top + height - 11.0, w - size::PAD, 10.0),
                ink::FAINT,
            );
        }
    }

    fn row(&self, gfx: &Gfx, painter: &mut Painter, at: Rect, row: &Row, here: bool) {
        if here {
            gfx.rounded(self.px(3.0), || gfx.fill(at, ink::PICKED));
        }

        let is_fact = matches!(row.kind, Kind::Fact { .. });
        let title_ink = if here && !is_fact { ink::TEXT } else { ink::DIM };
        painter.put(
            gfx,
            &self.spec(&row.title, size::TEXT),
            Rect::new(at.x + self.px(7.0), at.y + self.px(6.0), at.w * 0.55, at.h),
            title_ink,
        );

        // The right-hand column. A choice gets arrows so it is obvious it is
        // a dial rather than a switch; a fact gets none, because there is
        // nothing to turn.
        let value_ink = if row.pending() {
            ink::QUEUED
        } else if is_fact {
            ink::FAINT
        } else {
            ink::TEXT
        };
        let shown = match &row.kind {
            Kind::Choice { .. } if here => format!("‹ {} ›", row.value()),
            Kind::Choice { .. } => format!("  {}  ", row.value()),
            _ => row.value().to_string(),
        };
        painter.put_right(
            gfx,
            &self.spec(&shown, size::TEXT),
            Rect::new(at.x, at.y + self.px(6.0), at.w - self.px(7.0), at.h),
            value_ink,
        );

        // What it is now, when it is about to become something else. Without
        // this the queue is a list of destinations with no origins.
        if let Some(was) = row.was() {
            painter.put_right(
                gfx,
                &self.spec(&format!("was {was}"), size::SMALL),
                Rect::new(at.x, at.y - self.px(1.0), at.w - self.px(7.0), self.px(9.0)),
                ink::FAINT,
            );
        }
    }

    fn help(&self, gfx: &Gfx, painter: &mut Painter, at: (f32, f32), app: &App) {
        let (y, w) = at;
        gfx.fill(self.rect(0.0, y, w, size::HELP), ink::BAR);
        // Tab-aware, because A means different things on the two tabs: on
        // patches it applies everything queued, on sync it runs the one row
        // the cursor is on.
        let keys = match (&app.overlay, app.tab) {
            (Overlay::None, Tab::Patches) => "←→ change   A apply   B back   X what it does",
            (Overlay::None, Tab::Sync) => "A run this   B back   X what it does   L/R tabs",
            (Overlay::Detail, _) => "B close",
            (Overlay::ConfirmApply, _) => "A confirm   B cancel",
            (Overlay::ConfirmDiscard, _) => "A discard   B stay",
            (Overlay::ConfirmAction { .. }, _) => match &app.stage {
                Stage::Ready(r) if !r.is_empty() => "A do all this   B cancel",
                _ => "A run it   B cancel",
            },
            (Overlay::Applying { .. }, _) => "working…",
        };
        painter.put(
            gfx,
            &self.spec(keys, size::SMALL),
            self.rect(size::PAD, y + 4.0, w, size::HELP),
            ink::FAINT,
        );

        let queued = app.queue().len();
        if queued > 0 {
            let word = if queued == 1 { "change" } else { "changes" };
            painter.put_right(
                gfx,
                &self.spec(&format!("{queued} {word} not applied"), size::SMALL),
                self.rect(0.0, y + 4.0, w - size::PAD, size::HELP),
                ink::QUEUED,
            );
        }
    }

    /// The shared frame behind every overlay. Returns its rect, in pixels.
    fn panel(&self, gfx: &Gfx, title: &str, painter: &mut Painter, lines: usize) -> Rect {
        let (w, h) = self.panel_size();
        gfx.fill(self.rect(0.0, 0.0, w, h), ink::SHADE);
        let ph = (52.0 + lines as f32 * 13.0).min(h - 26.0);
        let at = self.rect(26.0, (h - ph) * 0.5, w - 52.0, ph);
        gfx.rounded(self.px(4.0), || gfx.fill(at, ink::BAR));
        gfx.outline(at, self.px(1.0), ink::LINE);
        painter.put(
            gfx,
            &self.spec(title, size::TEXT),
            Rect::new(at.x + self.px(10.0), at.y + self.px(9.0), at.w, self.px(14.0)),
            ink::TEXT,
        );
        at
    }

    fn detail(&self, gfx: &Gfx, painter: &mut Painter, app: &App) {
        let Some(row) = app.page().selected() else { return };
        let at = self.panel(gfx, &row.title, painter, 6);
        let width_points = at.w / self.scale - 20.0;
        let body = self.spec(&row.detail, size::SMALL).wrapped(width_points, 9);
        painter.put(
            gfx,
            &body,
            Rect::new(
                at.x + self.px(10.0),
                at.y + self.px(28.0),
                at.w - self.px(20.0),
                at.h - self.px(38.0),
            ),
            ink::DIM,
        );
    }

    fn confirm_apply(&self, gfx: &Gfx, painter: &mut Painter, app: &App) {
        let queue = app.queue();
        let at = self.panel(gfx, "Apply these changes?", painter, queue.len() + 1);
        for (i, row) in queue.iter().enumerate() {
            let y = at.y + self.px(28.0 + i as f32 * 13.0);
            if y > at.bottom() - self.px(13.0) {
                break;
            }
            painter.put(
                gfx,
                &self.spec(&row.title, size::SMALL),
                Rect::new(at.x + self.px(10.0), y, at.w * 0.6, self.px(11.0)),
                ink::DIM,
            );
            let change = match row.was() {
                Some(was) => format!("{was}  →  {}", row.value()),
                None => row.value().to_string(),
            };
            painter.put_right(
                gfx,
                &self.spec(&change, size::SMALL),
                Rect::new(at.x, y, at.w - self.px(10.0), self.px(11.0)),
                ink::QUEUED,
            );
        }
    }

    fn confirm_discard(&self, gfx: &Gfx, painter: &mut Painter, app: &App) {
        let n = app.queue().len();
        let at = self.panel(gfx, "Leave without applying?", painter, 3);
        let word = if n == 1 { "change" } else { "changes" };
        let width_points = at.w / self.scale - 20.0;
        painter.put(
            gfx,
            &self
                .spec(
                    &format!("{n} {word} would be thrown away. Nothing has been written yet."),
                    size::SMALL,
                )
                .wrapped(width_points, 3),
            Rect::new(
                at.x + self.px(10.0),
                at.y + self.px(29.0),
                at.w - self.px(20.0),
                self.px(28.0),
            ),
            ink::DIM,
        );
    }

    fn confirm_action(&self, gfx: &Gfx, painter: &mut Painter, app: &App, title: &str) {
        // With a plan in hand, the prompt *is* the plan. Showing the row's
        // description instead would be asking someone to accept something
        // they have not been shown, which is the one thing this flow exists
        // to avoid.
        if let Stage::Ready(review) = &app.stage
            && !review.is_empty()
        {
            return self.plan(gfx, painter, review);
        }
        let at = self.panel(gfx, title, painter, 4);
        let detail = app
            .page()
            .selected()
            .map(|r| r.detail.clone())
            .unwrap_or_default();
        let width_points = at.w / self.scale - 20.0;
        painter.put(
            gfx,
            &self.spec(&detail, size::SMALL).wrapped(width_points, 4),
            Rect::new(
                at.x + self.px(10.0),
                at.y + self.px(28.0),
                at.w - self.px(20.0),
                self.px(40.0),
            ),
            ink::DIM,
        );
    }

    /// What the server said it would do, line by line.
    fn plan(&self, gfx: &Gfx, painter: &mut Painter, review: &Review) {
        let rows = review.lines.len().min(9);
        let at = self.panel(gfx, &review.headline(), painter, rows + 1);
        for (i, line) in review.lines.iter().take(rows).enumerate() {
            let y = at.y + self.px(28.0 + i as f32 * 13.0);
            if y > at.bottom() - self.px(13.0) {
                break;
            }
            // The verb on the left, coloured by what it costs you: a conflict
            // needs a decision, a pull overwrites something here.
            let ink = match line.action {
                Action::Conflict => ink::QUEUED,
                Action::Download => ink::TEXT,
                Action::Upload => ink::DIM,
            };
            painter.put(
                gfx,
                &self.spec(line.action.label(), size::SMALL),
                Rect::new(at.x + self.px(10.0), y, at.w * 0.25, self.px(11.0)),
                ink,
            );
            painter.put(
                gfx,
                &self.spec(&line.title, size::SMALL),
                Rect::new(at.x + self.px(48.0), y, at.w - self.px(58.0), self.px(11.0)),
                ink::DIM,
            );
        }
        if review.lines.len() > rows {
            painter.put_right(
                gfx,
                &self.spec(
                    &format!("and {} more", review.lines.len() - rows),
                    size::SMALL,
                ),
                Rect::new(at.x, at.bottom() - self.px(14.0), at.w - self.px(10.0), self.px(11.0)),
                ink::FAINT,
            );
        }
    }

    fn applying(&self, gfx: &Gfx, painter: &mut Painter, done: usize, total: usize) {
        let at = self.panel(gfx, "Applying", painter, 2);
        painter.put(
            gfx,
            &self.spec(&format!("{done} of {total}"), size::SMALL),
            Rect::new(at.x + self.px(10.0), at.y + self.px(30.0), at.w, self.px(11.0)),
            ink::DIM,
        );
        let bar = Rect::new(
            at.x + self.px(10.0),
            at.y + self.px(46.0),
            at.w - self.px(20.0),
            self.px(4.0),
        );
        gfx.fill(bar, ink::LINE);
        let width = if total == 0 {
            0.0
        } else {
            bar.w * (done as f32 / total as f32)
        };
        gfx.fill(Rect::new(bar.x, bar.y, width, bar.h), ink::ACCENT);
    }
}
