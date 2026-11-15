//! Modal dialogs: a title, rows of labelled controls, buttons. Immediate mode like the rest of the
//! chrome: drawn each frame from the rows, clicks matched against the rectangles of the last
//! frame. Used by the JACK server settings now, and by the session dialogs next.

use crate::gfx::{Gfx, Rect, alpha};
use crate::text::Face;
use crate::theme::*;

#[derive(Clone, Debug)]
pub enum Control {
    Header,
    /// Read-only text.
    Info(String),
    Toggle(bool),
    /// One of `options` (value, label). `editable`: other values may be typed too.
    Choice {
        value: String,
        options: Vec<(String, String)>,
        editable: bool,
    },
    /// An integer with − / + and a typed value.
    Number {
        value: i64,
        min: i64,
        max: i64,
    },
    Text(String),
    /// A row that does something when clicked (open this session, add this application); the
    /// text is shown on the right.
    Link(String),
    /// A Link with a × at its right end: Event::Set(id, "remove").
    LinkRemove(String),
}

#[derive(Clone, Debug)]
pub struct Row {
    /// What the owner of the dialog knows the row by (e.g. a jackdbus parameter path).
    pub id: String,
    pub label: String,
    pub help: String,
    pub control: Control,
    /// Set away from its default (offers a reset).
    pub modified: bool,
}

impl Row {
    pub fn header(label: impl Into<String>) -> Row {
        Row { id: String::new(), label: label.into(), help: String::new(), control: Control::Header, modified: false }
    }
    pub fn info(label: impl Into<String>, text: impl Into<String>) -> Row {
        Row { id: String::new(), label: label.into(), help: String::new(), control: Control::Info(text.into()), modified: false }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    Server,
    NewSession,
    OpenSession,
    AddApp,
    /// One command line (the sidebar's Executable).
    AddExe,
    /// Confirm deleting a session (`tag`: its name).
    DeleteSession,
    /// Confirm removing a running client (`tag`: its id).
    RemoveClient,
    /// Confirm deleting a removed client with its files (`tag`: its id).
    DeleteClient,
    /// Confirm stopping a client with unsaved changes (`tag`: its id).
    StopClient,
    /// Confirm closing the session without saving.
    AbortSession,
    /// Session and client templates, to use or delete.
    Templates,
    /// Confirm restarting an older daemon that runs the open session (`tag`: its name).
    UpdateDaemon,
    /// Confirm deleting a template (`tag`: "s:<name>" or "c:<name>").
    DeleteTemplate,
    Rename,
    Duplicate,
    Notes,
    Snapshot,
    Snapshots,
    SaveTemplate,
    /// Rename a node or a port; `Dialog::tag` holds the layout key.
    RenameItem,
    Views,
    /// PipeWire's settings (the settings page's second tab).
    PipeWire,
    /// The event log and jackdbus's log.
    Log,
    /// The name for a JACK preset.
    SavePreset,
    /// Connection rules.
    Rules,
    /// Group ports: a name and how many.
    PortGroup,
    /// A new name for a port group.
    GroupName,
    /// A session client's properties.
    ClientProps,
    /// The name for a client template.
    ClientTemplate,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Part {
    Toggle,
    Choice,
    Minus,
    Plus,
    Edit,
    Reset,
    Link,
    Remove,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Hit {
    Row(usize, Part),
    Button(usize),
    Tab(usize),
    Close,
}

/// What a click in the dialog asks the owner to do.
#[derive(Clone, Debug)]
pub enum Event {
    /// Row `id` gets this value (text form; the owner parses it).
    Set(String, String),
    Reset(String),
    /// Open the options of a choice row under this rectangle.
    Options(usize, Rect),
    Button(usize),
    /// A tab of the page's title was clicked.
    Tab(usize),
    Close,
}

pub struct Dialog {
    pub kind: Kind,
    /// The owner's data (what the dialog is about).
    pub tag: String,
    pub title: String,
    pub rows: Vec<Row>,
    pub buttons: Vec<String>,
    /// A paragraph above the rows (wrapped): what a confirmation is about.
    pub message: String,
    pub note: String,
    /// Tabs after the title (pages that are one of several), and the one shown.
    pub tabs: Vec<String>,
    pub tab: usize,
    pub scroll: f32,
    /// The row whose text is being edited, and the text.
    pub editing: Option<(usize, String)>,
    /// The edit's text is the prefilled value, all "selected": the first key replaces it.
    fresh: bool,
    hits: Vec<(Rect, Hit)>,
    pub panel: Rect,
    content: Rect,
    content_h: f32,
    pub hover_help: Option<(String, [f32; 2])>,
}

impl Dialog {
    pub fn new(kind: Kind, title: impl Into<String>) -> Dialog {
        Dialog {
            kind,
            tag: String::new(),
            title: title.into(),
            rows: Vec::new(),
            buttons: Vec::new(),
            message: String::new(),
            note: String::new(),
            tabs: Vec::new(),
            tab: 0,
            scroll: 0.0,
            editing: None,
            fresh: false,
            hits: Vec::new(),
            panel: Rect::default(),
            content: Rect::default(),
            content_h: 0.0,
            hover_help: None,
        }
    }

    /// Replace the rows, keeping the scroll position and an edit in progress on the same row.
    pub fn set_rows(&mut self, rows: Vec<Row>) {
        if let Some((i, _)) = &self.editing {
            let id = self.rows.get(*i).map(|r| r.id.clone());
            self.editing = self.editing.take().and_then(|(_, t)| Some((rows.iter().position(|r| Some(&r.id) == id.as_ref())?, t)));
        }
        self.rows = rows;
    }

    pub fn scroll_by(&mut self, d: f32) {
        self.scroll = (self.scroll + d).clamp(0.0, (self.content_h - self.content.h).max(0.0));
    }

    /// Finish a text edit: the value goes to its row (text rows keep it themselves too).
    pub fn commit(&mut self) -> Option<Event> {
        let (i, text) = self.editing.take()?;
        let row = self.rows.get_mut(i)?;
        if let Control::Text(v) = &mut row.control {
            *v = text.clone();
        }
        Some(Event::Set(row.id.clone(), text))
    }

    /// Edit the text row `id` right away (a dialog that asks for a name).
    pub fn edit(&mut self, id: &str) {
        if let Some(i) = self.rows.iter().position(|r| r.id == id)
            && let Control::Text(v) = &self.rows[i].control
        {
            self.editing = Some((i, v.clone()));
            self.fresh = !v.is_empty();
        }
    }

    /// The current value of row `id` (text being edited included).
    pub fn value(&self, id: &str) -> Option<String> {
        let i = self.rows.iter().position(|r| r.id == id)?;
        if let Some((e, t)) = &self.editing
            && *e == i
        {
            return Some(t.clone());
        }
        match &self.rows[i].control {
            Control::Text(v) | Control::Info(v) | Control::Link(v) | Control::LinkRemove(v) => Some(v.clone()),
            Control::Choice { value, .. } => Some(value.clone()),
            Control::Toggle(b) => Some(if *b { "on" } else { "off" }.into()),
            Control::Number { value, .. } => Some(value.to_string()),
            Control::Header => None,
        }
    }

    /// Set a local row's value (dialogs whose rows are a form, not remote state).
    pub fn set_local(&mut self, id: &str, v: &str) {
        let Some(row) = self.rows.iter_mut().find(|r| r.id == id) else { return };
        match &mut row.control {
            Control::Text(t) => *t = v.to_string(),
            Control::Toggle(b) => *b = v == "on",
            Control::Choice { value, .. } => *value = v.to_string(),
            Control::Number { value, .. } => *value = v.parse().unwrap_or(*value),
            _ => {}
        }
    }

    pub fn text_input(&mut self, s: &str) {
        if let Some((_, t)) = &mut self.editing {
            if std::mem::take(&mut self.fresh) {
                t.clear();
            }
            t.push_str(s);
        }
    }

    pub fn backspace(&mut self) {
        if let Some((_, t)) = &mut self.editing {
            if std::mem::take(&mut self.fresh) {
                t.clear();
            } else {
                t.pop();
            }
        }
    }

    /// A click: what it asks of the owner (an edit in progress is committed first).
    pub fn click(&mut self, x: f32, y: f32) -> Vec<Event> {
        let hit = self.hits.iter().rev().find(|(r, _)| r.contains(x, y)).map(|(r, h)| (*r, *h));
        let mut out = Vec::new();
        // A click anywhere but the field being edited commits the edit first, then does what it
        // does there (another field starts editing, a button or a link acts): one click moves
        // from field to field, as in any form. A click on nothing only commits.
        if self.editing.is_some() && !matches!(hit, Some((_, Hit::Row(i, Part::Edit))) if Some(i) == self.editing.as_ref().map(|e| e.0)) {
            out.extend(self.commit());
            if hit.is_none() {
                return out;
            }
        }
        out.extend(self.click_hit(hit));
        out
    }

    fn click_hit(&mut self, hit: Option<(Rect, Hit)>) -> Option<Event> {
        let (r, h) = hit?;
        match h {
            Hit::Close => Some(Event::Close),
            Hit::Button(b) => Some(Event::Button(b)),
            Hit::Tab(t) => Some(Event::Tab(t)),
            Hit::Row(i, part) => {
                let row = self.rows.get(i)?;
                let id = row.id.clone();
                match (&row.control, part) {
                    (_, Part::Reset) => Some(Event::Reset(id)),
                    (Control::LinkRemove(_), Part::Remove) => Some(Event::Set(id, "remove".into())),
                    (Control::Link(_) | Control::LinkRemove(_), _) => Some(Event::Set(id, "activate".into())),
                    (Control::Toggle(on), _) => Some(Event::Set(id, if *on { "off" } else { "on" }.into())),
                    (Control::Choice { .. }, Part::Choice) => Some(Event::Options(i, r)),
                    (Control::Number { value, min, .. }, Part::Minus) => Some(Event::Set(id, (*value - 1).max(*min).to_string())),
                    (Control::Number { value, max, .. }, Part::Plus) => Some(Event::Set(id, (*value + 1).min(*max).to_string())),
                    (c, Part::Edit) => {
                        let text = match c {
                            Control::Number { value, .. } => value.to_string(),
                            Control::Choice { value, .. } | Control::Text(value) => value.clone(),
                            _ => return None,
                        };
                        // Selected, like a field you click into elsewhere: the first key replaces it.
                        self.fresh = !text.is_empty();
                        self.editing = Some((i, text));
                        None
                    }
                    _ => None,
                }
            }
        }
    }

    /// Test points: a row's control (by label), or a button (by label).
    pub fn point(&self, what: &str) -> Option<[f32; 2]> {
        let center = |r: &Rect| [r.cx(), r.cy()];
        if let Some(t) = what.strip_prefix("tab:") {
            let ti = self.tabs.iter().position(|l| l == t)?;
            return self.hits.iter().find(|(_, h)| *h == Hit::Tab(ti)).map(|(r, _)| center(r));
        }
        if let Some(b) = what.strip_prefix("button:") {
            let bi = self.buttons.iter().position(|l| l == b)?;
            return self.hits.iter().find(|(_, h)| *h == Hit::Button(bi)).map(|(r, _)| center(r));
        }
        // "choice:<label>": a row's ▾ (its list of values).
        if let Some(l) = what.strip_prefix("choice:") {
            let i = self.rows.iter().position(|r| r.label == l)?;
            return self.hits.iter().find(|(_, h)| *h == Hit::Row(i, Part::Choice)).map(|(r, _)| center(r));
        }
        // "remove:<label>": a removable row's ×.
        if let Some(l) = what.strip_prefix("remove:") {
            let i = self.rows.iter().position(|r| r.label == l)?;
            return self.hits.iter().find(|(_, h)| *h == Hit::Row(i, Part::Remove)).map(|(r, _)| center(r));
        }
        let i = self.rows.iter().position(|r| r.label == what)?;
        self.hits.iter().find(|(_, h)| matches!(h, Hit::Row(j, p) if *j == i && *p != Part::Reset)).map(|(r, _)| center(r))
    }

    /// Pages (settings, lists) fill the canvas; prompts are a card over it.
    pub fn is_page(&self) -> bool {
        matches!(self.kind, Kind::Server | Kind::PipeWire | Kind::Log | Kind::Rules | Kind::AddApp | Kind::OpenSession | Kind::Snapshots | Kind::Views | Kind::Templates)
    }

    /// Draw over `area` (the canvas): a page fills it, a prompt is a card in its middle. The
    /// window's chrome around stays usable.
    pub fn draw(&mut self, g: &mut Gfx, area: Rect, s: f32, mouse: [f32; 2]) {
        self.hits.clear();
        self.hover_help = None;
        let page = self.is_page();
        let px = FONT * s;
        let lh = g.line_height(Face::Regular, px);
        let row_h = (30.0 * s).round();
        let title_h = (if page { 48.0 } else { 40.0 } * s).round();
        let foot_h = (52.0 * s).round();
        let pad = 14.0 * s;
        // A page's rows use most of the canvas's width (up to a width still easy to read across).
        let page_col = |w: f32| (w - 2.0 * pad).min(1400.0 * s);
        // The message, wrapped to the width the rows get.
        let msg_w = if page { page_col(area.w) } else { (560.0 * s).min(area.w - 40.0 * s) } - 2.0 * pad;
        let msg_lines = if self.message.is_empty() { Vec::new() } else { crate::ui::wrap(g, px, &self.message, msg_w) };
        let msg_h = if msg_lines.is_empty() { 0.0 } else { msg_lines.len() as f32 * (lh + 4.0 * s) + pad };
        self.content_h = self.rows.iter().map(|r| if matches!(r.control, Control::Header) { row_h * 1.2 } else { row_h }).sum::<f32>() + pad + msg_h;
        let panel = if page {
            // The whole canvas; the graph is behind it, not shown.
            g.rect(area, PANEL_BG);
            area
        } else {
            g.rect(area, [0.0, 0.0, 0.0, 0.45]);
            let w = (560.0 * s).min(area.w - 40.0 * s);
            let h = (title_h + self.content_h + foot_h).min(area.h - 40.0 * s);
            let p = Rect::new((area.cx() - w / 2.0).round(), (area.cy() - h / 2.0).round(), w.round(), h.round());
            g.rrect(Rect::new(p.x + 4.0 * s, p.y + 6.0 * s, p.w, p.h), 6.0 * s, [0.0, 0.0, 0.0, 0.4]);
            g.rrect(p, 6.0 * s, PANEL_BG);
            g.rrect_stroke(p, 6.0 * s, s.max(1.0), LINE_SOFT);
            p
        };
        self.panel = panel;
        let col_w = if page { page_col(panel.w) } else { panel.w };
        let col_x = if page { (panel.cx() - col_w / 2.0).round() } else { panel.x };
        let w = col_w;
        // A page goes back to the graph with the arrow before its title (the toolbar button that
        // opened it closes it too); a prompt has its × at the right.
        let mut title_x = col_x + pad;
        if page {
            let b = (28.0 * s).round();
            let back = Rect::new(col_x + pad * 0.5, (panel.y + (title_h - b) / 2.0).round(), b, b);
            let on = back.contains(mouse[0], mouse[1]);
            if on {
                g.rrect(back, 4.0 * s, BUTTON_HOVER);
            }
            let (c, k) = ([back.cx() + 1.5 * s, back.cy()], 5.5 * s);
            let col = if on { HILIGHT } else { TEXT };
            g.line([c[0] + k * 0.5, c[1] - k], [c[0] - k * 0.5, c[1]], 1.8 * s, col);
            g.line([c[0] - k * 0.5, c[1]], [c[0] + k * 0.5, c[1] + k], 1.8 * s, col);
            self.hits.push((back, Hit::Close));
            if on {
                self.hover_help = Some(("Back to the graph".into(), mouse));
            }
            title_x = back.right() + 8.0 * s;
        }
        // Title.
        let tpx = if page { px * 1.3 } else { px * 1.1 };
        let tw = g.text(Face::Bold, tpx, title_x, panel.y + (title_h - lh * tpx / px) / 2.0, TEXT_BRIGHT, &self.title);
        // Tabs: Luluda buttons after the title, the shown one lit.
        let mut tx = title_x + tw + 18.0 * s;
        let th = (lh + 12.0 * s).round();
        for (i, t) in self.tabs.iter().enumerate() {
            let w = g.measure(Face::Regular, px, t) + 24.0 * s;
            let r = Rect::new(tx, (panel.y + (title_h - th) / 2.0).round(), w, th);
            let on = i == self.tab;
            let hv = !on && r.contains(mouse[0], mouse[1]);
            crate::ui::lbutton(g, r, s, TINT_BLUE, hv, on, true);
            g.text_center(Face::Regular, px, r, crate::ui::lbutton_text(TINT_BLUE, hv, on, true), t);
            if !on {
                self.hits.push((r, Hit::Tab(i)));
            }
            tx += w + 6.0 * s;
        }
        if !page {
            let close = Rect::new(panel.right() - title_h, panel.y, title_h, title_h);
            let c = [close.cx(), close.cy()];
            let k = 5.0 * s;
            let ccol = if close.contains(mouse[0], mouse[1]) { HILIGHT } else { TEXT };
            g.line([c[0] - k, c[1] - k], [c[0] + k, c[1] + k], 1.6 * s, ccol);
            g.line([c[0] - k, c[1] + k], [c[0] + k, c[1] - k], 1.6 * s, ccol);
            self.hits.push((close, Hit::Close));
        }
        g.rect(Rect::new(panel.x, panel.y + title_h, panel.w, s.max(1.0)), PANEL_LINE);

        // Rows.
        let content = Rect::new(panel.x, panel.y + title_h, panel.w, panel.h - title_h - foot_h);
        self.content = content;
        self.scroll = self.scroll.clamp(0.0, (self.content_h - content.h).max(0.0));
        g.push_clip(content);
        let mut y = content.y + pad * 0.5 - self.scroll;
        if !msg_lines.is_empty() {
            y += pad * 0.5;
            for l in &msg_lines {
                g.text(Face::Regular, px, col_x + pad, y, TEXT_BRIGHT, l);
                y += lh + 4.0 * s;
            }
            y += pad * 0.5;
        }
        // The labels' column: enough for the longest usual label, no more (a wide page's controls
        // stay near their labels)
        let label_w = if page { (w * 0.36).clamp(220.0 * s, 460.0 * s) } else { w * 0.42 };
        let ctl_x = col_x + pad + label_w;
        let ctl_w = col_x + col_w - pad - ctl_x;
        for i in 0..self.rows.len() {
            let row = self.rows[i].clone();
            let rh = if matches!(row.control, Control::Header) { row_h * 1.2 } else { row_h };
            let rr = Rect::new(col_x, y, col_w, rh);
            y += rh;
            if rr.bottom() < content.y || rr.y > content.bottom() {
                continue;
            }
            if let Control::Header = row.control {
                g.text(Face::Bold, px, col_x + pad, rr.bottom() - lh - 6.0 * s, HILIGHT, &row.label);
                g.rect(Rect::new(col_x + pad, rr.bottom() - 2.0 * s, col_w - 2.0 * pad, s.max(1.0)), alpha(HILIGHT, 0.4));
                continue;
            }
            let label_r = Rect::new(col_x + pad, rr.y, label_w - 8.0 * s, rr.h);
            g.text_fit(Face::Regular, px, label_r.x, rr.cy() - lh / 2.0, label_r.w, TEXT, &row.label);
            if !row.help.is_empty() && label_r.contains(mouse[0], mouse[1]) && content.contains(mouse[0], mouse[1]) {
                self.hover_help = Some((row.help.clone(), mouse));
            }
            let reset_w = if row.modified { 24.0 * s } else { 0.0 };
            let cr = Rect::new(ctl_x, rr.y + 3.0 * s, ctl_w - reset_w, rr.h - 6.0 * s);
            let hov = |r: &Rect| r.contains(mouse[0], mouse[1]) && content.contains(mouse[0], mouse[1]);
            let editing = self.editing.as_ref().filter(|(e, _)| *e == i).map(|(_, t)| t.clone());
            let selected = editing.is_some() && self.fresh;
            let field = |g: &mut Gfx, r: Rect, text: &str, active: bool| {
                g.rrect(r, 3.0 * s, if active { BUTTON_HOVER } else { BUTTON });
                g.rrect_stroke(r, 3.0 * s, s.max(1.0), if active { HILIGHT } else { LINE_SOFT });
                // An edit shows its caret (a bar: the font has no ▏) after the text.
                let (shown, caret) = match text.strip_suffix('▏') {
                    Some(t) => (t, true),
                    None => (text, false),
                };
                // Selected (the first key replaces it): the text on a highlight.
                if caret && selected && !shown.is_empty() {
                    let sw = g.measure(Face::Regular, px, shown).min(r.w - 16.0 * s);
                    g.rect(Rect::new(r.x + 7.0 * s, r.cy() - lh / 2.0 - 1.0 * s, sw + 2.0 * s, lh + 2.0 * s), alpha(BORDER, 0.45));
                }
                let w = g.text_fit(Face::Regular, px, r.x + 8.0 * s, r.cy() - lh / 2.0, r.w - 16.0 * s, TEXT_BRIGHT, shown);
                if caret {
                    let cx = (r.x + 8.0 * s + w.min(r.w - 16.0 * s) + 1.0).round();
                    g.rect(Rect::new(cx, r.cy() - lh / 2.0, (1.5 * s).max(1.0), lh), HILIGHT);
                }
            };
            match &row.control {
                Control::Header => {}
                Control::Info(t) => {
                    g.text_fit(Face::Regular, px, cr.x, rr.cy() - lh / 2.0, cr.w, TEXT_BRIGHT, t);
                }
                Control::Link(t) | Control::LinkRemove(t) => {
                    let removable = matches!(row.control, Control::LinkRemove(_));
                    let whole = Rect::new(col_x + pad / 2.0, rr.y + 1.0, col_w - pad, rr.h - 2.0);
                    let xr = Rect::new(whole.right() - whole.h, whole.y, whole.h, whole.h);
                    let text_w = if removable { cr.w - xr.w - 4.0 * s } else { cr.w };
                    if hov(&whole) {
                        g.rrect(whole, 3.0 * s, BUTTON_HOVER);
                        g.text_fit(Face::Regular, px, label_r.x, rr.cy() - lh / 2.0, label_r.w, HILIGHT, &row.label);
                    }
                    g.text_fit(Face::Regular, FONT_SMALL * s, cr.x, rr.cy() - g.line_height(Face::Regular, FONT_SMALL * s) / 2.0, text_w, TEXT_DIM, t);
                    self.hits.push((whole, Hit::Row(i, Part::Link)));
                    if removable {
                        // The ×: dim, red under the pointer.
                        let on = hov(&xr);
                        if on {
                            g.rrect(xr, 3.0 * s, BUTTON);
                            self.hover_help = Some(("Delete (asks first)".into(), mouse));
                        }
                        let (c, k) = ([xr.cx(), xr.cy()], 4.5 * s);
                        let col = if on { BAD } else if hov(&whole) { TEXT } else { TEXT_DIM };
                        g.line([c[0] - k, c[1] - k], [c[0] + k, c[1] + k], 1.5 * s, col);
                        g.line([c[0] - k, c[1] + k], [c[0] + k, c[1] - k], 1.5 * s, col);
                        self.hits.push((xr, Hit::Row(i, Part::Remove)));
                    }
                }
                Control::Toggle(on) => {
                    let tw = 40.0 * s;
                    let tr = Rect::new(cr.x, cr.cy() - 9.0 * s, tw, 18.0 * s);
                    g.rrect(tr, 9.0 * s, if *on { alpha(TOGGLE, 0.55) } else { BUTTON });
                    g.rrect_stroke(tr, 9.0 * s, s.max(1.0), if hov(&tr) { HILIGHT } else { LINE_SOFT });
                    let kx = if *on { tr.right() - 9.0 * s } else { tr.x + 9.0 * s };
                    g.circle([kx, tr.cy()], 6.5 * s, if *on { TEXT_BRIGHT } else { TEXT });
                    g.text(Face::Regular, px, tr.right() + 10.0 * s, rr.cy() - lh / 2.0, TEXT, if *on { "on" } else { "off" });
                    self.hits.push((tr, Hit::Row(i, Part::Toggle)));
                }
                Control::Choice { value, options, editable } => {
                    let arrow_w = 28.0 * s;
                    let label = options.iter().find(|(v, _)| v == value).map(|(_, l)| l.clone()).unwrap_or_else(|| value.clone());
                    match (&editing, editable) {
                        (Some(t), _) => field(g, Rect::new(cr.x, cr.y, cr.w - arrow_w - 4.0 * s, cr.h), &format!("{t}▏"), true),
                        (None, true) => {
                            let er = Rect::new(cr.x, cr.y, cr.w - arrow_w - 4.0 * s, cr.h);
                            field(g, er, &label, hov(&er));
                            self.hits.push((er, Hit::Row(i, Part::Edit)));
                        }
                        (None, false) => {}
                    }
                    let ar = if *editable || editing.is_some() { Rect::new(cr.right() - arrow_w, cr.y, arrow_w, cr.h) } else { cr };
                    if !*editable && editing.is_none() {
                        field(g, ar, &label, hov(&ar));
                        g.text(Face::Regular, px, ar.right() - 18.0 * s, ar.cy() - lh / 2.0, TEXT, "▼");
                    } else {
                        g.rrect(ar, 3.0 * s, if hov(&ar) { BUTTON_HOVER } else { BUTTON });
                        g.rrect_stroke(ar, 3.0 * s, s.max(1.0), LINE_SOFT);
                        g.text_center(Face::Regular, px, ar, TEXT, "▼");
                    }
                    self.hits.push((ar, Hit::Row(i, Part::Choice)));
                }
                Control::Number { value, .. } => {
                    let bw = cr.h;
                    let mr = Rect::new(cr.x, cr.y, bw, cr.h);
                    let pr = Rect::new(cr.right() - bw, cr.y, bw, cr.h);
                    let er = Rect::new(mr.right() + 4.0 * s, cr.y, cr.w - 2.0 * bw - 8.0 * s, cr.h);
                    for (r, t, part) in [(mr, "−", Part::Minus), (pr, "+", Part::Plus)] {
                        g.rrect(r, 3.0 * s, if hov(&r) { BUTTON_HOVER } else { BUTTON });
                        g.rrect_stroke(r, 3.0 * s, s.max(1.0), LINE_SOFT);
                        g.text_center(Face::Regular, px, r, TEXT_BRIGHT, t);
                        self.hits.push((r, Hit::Row(i, part)));
                    }
                    match &editing {
                        Some(t) => field(g, er, &format!("{t}▏"), true),
                        None => field(g, er, &value.to_string(), hov(&er)),
                    }
                    self.hits.push((er, Hit::Row(i, Part::Edit)));
                }
                Control::Text(v) => {
                    match &editing {
                        Some(t) => field(g, cr, &format!("{t}▏"), true),
                        None => field(g, cr, if v.is_empty() { "—" } else { v }, hov(&cr)),
                    }
                    self.hits.push((cr, Hit::Row(i, Part::Edit)));
                }
            }
            if row.modified {
                let rr2 = Rect::new(cr.right() + 4.0 * s, cr.y, reset_w - 4.0 * s, cr.h);
                // Reset to default: a circular arrow (the font has no ↺).
                let col = if hov(&rr2) { HILIGHT } else { TEXT_DIM };
                let (c, rad) = ([rr2.cx(), rr2.cy()], 5.5 * s);
                g.arc(c, rad, -2.2, 2.4, 1.5 * s, col);
                let a = -2.2f32;
                let tip = [c[0] + rad * a.cos(), c[1] + rad * a.sin()];
                g.pie(tip, 3.2 * s, -0.4, 0.9, col);
                self.hits.push((rr2, Hit::Row(i, Part::Reset)));
            }
        }
        g.pop_clip();
        // Clicks outside the visible content don't reach rows.
        self.hits.retain(|(r, h)| !matches!(h, Hit::Row(..)) || content.overlaps(r));

        // Footer.
        let fy = panel.bottom() - foot_h;
        g.rect(Rect::new(panel.x, fy, panel.w, s.max(1.0)), PANEL_LINE);
        let bh = 30.0 * s;
        let mut bx = col_x + col_w - pad;
        for (bi, label) in self.buttons.iter().enumerate().rev() {
            let bw = g.measure(Face::Regular, px, label) + 28.0 * s;
            bx -= bw;
            let br = Rect::new(bx, fy + (foot_h - bh) / 2.0, bw, bh);
            let hv = br.contains(mouse[0], mouse[1]);
            let tint = crate::ui::tint_for(label);
            crate::ui::lbutton(g, br, s, tint, hv, false, true);
            g.text_center(Face::Regular, px, br, crate::ui::lbutton_text(tint, hv, false, true), label);
            self.hits.push((br, Hit::Button(bi)));
            bx -= 8.0 * s;
        }
        if !self.note.is_empty() {
            let spx = FONT_SMALL * s;
            let slh = g.line_height(Face::Regular, spx);
            g.text_fit(Face::Regular, spx, col_x + pad, fy + (foot_h - slh) / 2.0, bx - col_x - 2.0 * pad, TEXT_DIM, &self.note);
        }
    }
}
