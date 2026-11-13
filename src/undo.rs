//! Undo and redo. One step is one gesture or one action: a drag, a stroke across matrix cells, a
//! split, a hide… It records the connections it made and removed (by port name, so a step still
//! applies after the graph was re-read) and the canvas as it was before (positions, pins,
//! splits, hidden and expanded clients, names; not the view).

use crate::layout::Layout;

#[derive(Clone, Default)]
pub struct Step {
    pub made: Vec<(String, String)>,
    pub removed: Vec<(String, String)>,
    /// The canvas before the step (None: the step didn't change it).
    pub canvas: Option<Layout>,
}

#[derive(Default)]
pub struct History {
    undo: Vec<Step>,
    redo: Vec<Step>,
    /// The step being recorded, and how deep the begin/end nesting is.
    open: Option<Step>,
    depth: u32,
    /// Replaying a step: what it changes is not recorded.
    pub replaying: bool,
}

const LIMIT: usize = 200;

/// Do two layouts hold the same canvas (the view aside)?
pub fn same_canvas(a: &Layout, b: &Layout) -> bool {
    a.nodes == b.nodes && a.matrices == b.matrices && a.pinned == b.pinned && a.split == b.split && a.hidden == b.hidden && a.expanded == b.expanded && a.names == b.names
}

impl History {
    /// Start recording (nested calls join the outer step).
    pub fn begin(&mut self, canvas: &Layout) {
        if self.replaying {
            return;
        }
        if self.depth == 0 {
            self.open = Some(Step { canvas: Some(canvas.clone()), ..Default::default() });
        }
        self.depth += 1;
        if std::env::var_os("PATCHBOX_UNDO_TRACE").is_some() {
            eprintln!("[undo] begin depth={}", self.depth);
        }
    }

    /// Finish recording; keep the step if it changed anything.
    pub fn end(&mut self, canvas: &Layout) {
        if self.replaying || self.depth == 0 {
            return;
        }
        self.depth -= 1;
        if std::env::var_os("PATCHBOX_UNDO_TRACE").is_some() {
            eprintln!("[undo] end depth={} open={:?}", self.depth, self.open.as_ref().map(|s| (s.made.len(), s.removed.len())));
        }
        if self.depth > 0 {
            return;
        }
        let Some(mut s) = self.open.take() else { return };
        if s.canvas.as_ref().is_some_and(|c| same_canvas(c, canvas)) {
            s.canvas = None;
        }
        if s.made.is_empty() && s.removed.is_empty() && s.canvas.is_none() {
            return;
        }
        self.undo.push(s);
        if self.undo.len() > LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    pub fn connected(&mut self, o: &str, i: &str) {
        if let Some(s) = &mut self.open {
            s.made.push((o.into(), i.into()));
        }
    }

    pub fn disconnected(&mut self, o: &str, i: &str) {
        if let Some(s) = &mut self.open {
            s.removed.push((o.into(), i.into()));
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// The step to undo; the caller applies its inverse and gives back the canvas it replaced.
    pub fn pop_undo(&mut self) -> Option<Step> {
        self.undo.pop()
    }
    pub fn pop_redo(&mut self) -> Option<Step> {
        self.redo.pop()
    }
    pub fn push_redo(&mut self, s: Step) {
        self.redo.push(s);
    }
    pub fn push_undo(&mut self, s: Step) {
        self.undo.push(s);
    }
}
