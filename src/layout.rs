//! Binary split tree of editor panes. Each split is half and half. `Ctrl` + an
//! arrow picks the open window that actually sits on that side of the screen.

use ratatui::layout::Rect;
use serde::{Deserialize, Serialize};

use crate::action::PaneDirection;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EditorId(pub u64);

/// One document inside an editor pane. A pane keeps its id; tabs come and go.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TabId(pub u64);

/// Default width of the explorer column.
pub const SIDEBAR_WIDTH: u16 = 30;

/// The sidebar never draws narrower than this, unless the terminal itself is smaller.
pub const SIDEBAR_MIN_WIDTH: u16 = 16;

/// Columns added or removed by one resize step.
pub const SIDEBAR_STEP: u16 = 4;

/// Columns the sidebar may use. It stays at most half the screen, so the editor keeps the rest.
pub fn clamp_sidebar_width(area_width: u16, preferred: u16) -> u16 {
    let max = area_width / 2;
    if max == 0 {
        return 0;
    }
    let min = SIDEBAR_MIN_WIDTH.min(max);
    preferred.clamp(min, max)
}

/// The width after one wider or narrower step, already clamped to this screen.
pub fn step_sidebar_width(area_width: u16, current: u16, wider: bool) -> u16 {
    let shown = clamp_sidebar_width(area_width, current);
    let max = area_width / 2;
    if max == 0 {
        return 0;
    }
    let min = SIDEBAR_MIN_WIDTH.min(max);
    if wider {
        shown.saturating_add(SIDEBAR_STEP).min(max)
    } else {
        shown.saturating_sub(SIDEBAR_STEP).max(min)
    }
}

/// Rows at the top of every editor pane reserved for its file name.
pub const EDITOR_TITLE_ROWS: u16 = 1;

/// How many rows the title actually takes in a pane this tall.
pub fn title_rows(pane_height: u16) -> u16 {
    EDITOR_TITLE_ROWS.min(pane_height)
}

/// Rows from the top of a pane to the first text row.
/// A pane that is only the title keeps the offset inside itself.
pub fn text_row_offset(pane_height: u16) -> u16 {
    EDITOR_TITLE_ROWS.min(pane_height.saturating_sub(1))
}

/// A window that can take focus. The sidebar is included only while it is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    Sidebar,
    Editor(EditorId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Axis {
    /// Left | right.
    Horizontal,
    /// Top / bottom.
    Vertical,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Layout {
    Leaf(EditorId),
    Split {
        axis: Axis,
        first: Box<Layout>,
        second: Box<Layout>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Divider {
    pub area: Rect,
    /// A vertical bar (`│`) between left and right panes. Otherwise a horizontal rule (`─`).
    pub vertical: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placed {
    pub panes: Vec<(EditorId, Rect)>,
    pub dividers: Vec<Divider>,
}

impl Layout {
    pub fn split(self, target: EditorId, split_axis: Axis, new_id: EditorId) -> Self {
        match self {
            Layout::Leaf(id) if id == target => Layout::Split {
                axis: split_axis,
                first: Box::new(Layout::Leaf(id)),
                second: Box::new(Layout::Leaf(new_id)),
            },
            Layout::Leaf(_) => self,
            Layout::Split {
                axis,
                first,
                second,
            } => Layout::Split {
                axis,
                first: Box::new((*first).split(target, split_axis, new_id)),
                second: Box::new((*second).split(target, split_axis, new_id)),
            },
        }
    }

    /// Drops `target` and promotes its sibling. The last remaining pane is returned as `Err`
    /// so the caller still has the tree.
    pub fn close(self, target: EditorId) -> Result<Self, Self> {
        if matches!(self, Layout::Leaf(_)) {
            return Err(self);
        }
        Ok(remove(self, target))
    }

    /// The leaf that should take focus when `id` closes: the near edge of its sibling.
    pub fn focus_after_close(&self, id: EditorId) -> Option<EditorId> {
        let mut path = Vec::new();
        if !find_path(self, id, &mut path) {
            return None;
        }
        let step = *path.last()?;
        let direction = match (step.axis, step.in_second) {
            (Axis::Horizontal, false) => PaneDirection::Right,
            (Axis::Horizontal, true) => PaneDirection::Left,
            (Axis::Vertical, false) => PaneDirection::Down,
            (Axis::Vertical, true) => PaneDirection::Up,
        };
        Some(edge_leaf(sibling(self, &path), direction))
    }

    pub fn adjacent(&self, id: EditorId, direction: PaneDirection) -> Option<EditorId> {
        let mut path = Vec::new();
        if !find_path(self, id, &mut path) {
            return None;
        }
        for end in (0..path.len()).rev() {
            let step = path[end];
            let aligned = match direction {
                PaneDirection::Left | PaneDirection::Right => step.axis == Axis::Horizontal,
                PaneDirection::Up | PaneDirection::Down => step.axis == Axis::Vertical,
            };
            let crosses = match direction {
                PaneDirection::Left | PaneDirection::Up => step.in_second,
                PaneDirection::Right | PaneDirection::Down => !step.in_second,
            };
            if aligned && crosses {
                let sibling = sibling(self, &path[..=end]);
                return Some(edge_leaf(sibling, direction));
            }
        }
        None
    }

    /// Assigns each leaf a rectangle inside `area`, with a one-cell gap between siblings.
    pub fn place(&self, area: Rect) -> Placed {
        let mut placed = Placed {
            panes: Vec::new(),
            dividers: Vec::new(),
        };
        place_into(self, area, &mut placed);
        placed
    }
}

#[derive(Clone, Copy)]
struct Step {
    axis: Axis,
    in_second: bool,
}

fn find_path(layout: &Layout, id: EditorId, path: &mut Vec<Step>) -> bool {
    match layout {
        Layout::Leaf(leaf) => *leaf == id,
        Layout::Split {
            axis,
            first,
            second,
        } => {
            path.push(Step {
                axis: *axis,
                in_second: false,
            });
            if find_path(first, id, path) {
                return true;
            }
            path.pop();
            path.push(Step {
                axis: *axis,
                in_second: true,
            });
            if find_path(second, id, path) {
                return true;
            }
            path.pop();
            false
        }
    }
}

fn node_at<'a>(layout: &'a Layout, path: &[Step]) -> &'a Layout {
    let mut node = layout;
    for step in path {
        let Layout::Split { first, second, .. } = node else {
            return node;
        };
        node = if step.in_second { second } else { first };
    }
    node
}

fn sibling<'a>(layout: &'a Layout, path_including_self: &[Step]) -> &'a Layout {
    let parent = node_at(
        layout,
        &path_including_self[..path_including_self.len() - 1],
    );
    let Layout::Split { first, second, .. } = parent else {
        return parent;
    };
    if path_including_self
        .last()
        .is_some_and(|step| step.in_second)
    {
        first
    } else {
        second
    }
}

/// The leaf on the edge of `layout` that faces `direction`.
fn edge_leaf(layout: &Layout, direction: PaneDirection) -> EditorId {
    match layout {
        Layout::Leaf(id) => *id,
        Layout::Split {
            axis,
            first,
            second,
        } => {
            let parallel = match direction {
                PaneDirection::Left | PaneDirection::Right => *axis == Axis::Horizontal,
                PaneDirection::Up | PaneDirection::Down => *axis == Axis::Vertical,
            };
            let child = if parallel {
                let near_is_first = matches!(direction, PaneDirection::Right | PaneDirection::Down);
                if near_is_first { first } else { second }
            } else {
                first
            };
            edge_leaf(child, direction)
        }
    }
}

fn remove(layout: Layout, target: EditorId) -> Layout {
    match layout {
        Layout::Leaf(_) => layout,
        Layout::Split {
            axis,
            first,
            second,
        } => match remove_child(*first, target) {
            Child::Removed => *second,
            Child::Kept(first) => match remove_child(*second, target) {
                Child::Removed => first,
                Child::Kept(second) => Layout::Split {
                    axis,
                    first: Box::new(first),
                    second: Box::new(second),
                },
            },
        },
    }
}

enum Child {
    Removed,
    Kept(Layout),
}

fn remove_child(layout: Layout, target: EditorId) -> Child {
    match layout {
        Layout::Leaf(id) if id == target => Child::Removed,
        Layout::Leaf(_) => Child::Kept(layout),
        Layout::Split {
            axis,
            first,
            second,
        } => match remove_child(*first, target) {
            Child::Removed => Child::Kept(*second),
            Child::Kept(first) => match remove_child(*second, target) {
                Child::Removed => Child::Kept(first),
                Child::Kept(second) => Child::Kept(Layout::Split {
                    axis,
                    first: Box::new(first),
                    second: Box::new(second),
                }),
            },
        },
    }
}

fn place_into(layout: &Layout, area: Rect, out: &mut Placed) {
    let Layout::Split {
        axis,
        first,
        second,
    } = layout
    else {
        out.panes.push((leaf_id(layout), area));
        return;
    };
    let (span, vertical) = match axis {
        Axis::Horizontal => (area.width, true),
        Axis::Vertical => (area.height, false),
    };
    // A divider needs a cell of its own. Below that, the two panes share the space with no gap.
    let gap = u16::from(span >= 3);
    let inner = span - gap;
    let first_span = inner / 2;
    let second_span = inner - first_span;
    let (first_area, gap_area, second_area) = match axis {
        Axis::Horizontal => (
            Rect::new(area.x, area.y, first_span, area.height),
            Rect::new(area.x + first_span, area.y, gap, area.height),
            Rect::new(area.x + first_span + gap, area.y, second_span, area.height),
        ),
        Axis::Vertical => (
            Rect::new(area.x, area.y, area.width, first_span),
            Rect::new(area.x, area.y + first_span, area.width, gap),
            Rect::new(area.x, area.y + first_span + gap, area.width, second_span),
        ),
    };
    place_into(first, first_area, out);
    if gap == 1 {
        out.dividers.push(Divider {
            area: gap_area,
            vertical,
        });
    }
    place_into(second, second_area, out);
}

/// Sidebar column and the rectangle left for editor panes, under a one-row status bar.
pub fn body_rects(area: Rect, sidebar_open: bool, preferred: u16) -> (Rect, Rect) {
    split_above(area, sidebar_open, 1, preferred)
}

/// Sidebar column and the editor rectangle, leaving `bottom` rows free.
pub fn split_above(area: Rect, sidebar_open: bool, bottom: u16, preferred: u16) -> (Rect, Rect) {
    let body_height = area.height.saturating_sub(bottom.min(area.height));
    let sidebar_width = if sidebar_open {
        clamp_sidebar_width(area.width, preferred)
    } else {
        0
    };
    (
        Rect::new(area.x, area.y, sidebar_width, body_height),
        Rect::new(
            area.x + sidebar_width,
            area.y,
            area.width - sidebar_width,
            body_height,
        ),
    )
}

/// The open window in `direction` from `from`, on a straight line through `anchor`.
///
/// `anchor` is a cell inside the source window (the cursor, or the middle of the
/// sidebar). The nearest window on that side wins, and it has to contain the
/// anchor's row or column. A wider neighbor does not steal the move when the
/// anchor lines up with a narrower one.
pub fn adjacent_surface(
    surfaces: &[(Surface, Rect)],
    from: Surface,
    anchor: (u16, u16),
    direction: PaneDirection,
) -> Option<Surface> {
    let origin = surfaces.iter().find(|(surface, _)| *surface == from)?.1;
    let anchor = clamp_anchor(origin, anchor);
    let mut best: Option<(u16, u16, i32, u16, Surface)> = None;
    for &(surface, rect) in surfaces {
        if surface == from || rect.width == 0 || rect.height == 0 {
            continue;
        }
        let Some((gap, miss, overlap, start)) = approach(origin, rect, anchor, direction) else {
            continue;
        };
        let rank = (gap, miss, -(overlap as i32), start);
        if best
            .as_ref()
            .is_none_or(|(best_gap, best_miss, best_overlap, best_start, _)| {
                rank < (*best_gap, *best_miss, *best_overlap, *best_start)
            })
        {
            best = Some((gap, miss, -(overlap as i32), start, surface));
        }
    }
    best.map(|(_, _, _, _, surface)| surface)
}

fn clamp_anchor(origin: Rect, anchor: (u16, u16)) -> (u16, u16) {
    let x = anchor.0.clamp(
        origin.x,
        origin.x.saturating_add(origin.width.saturating_sub(1)),
    );
    let y = anchor.1.clamp(
        origin.y,
        origin.y.saturating_add(origin.height.saturating_sub(1)),
    );
    (x, y)
}

/// `(gap, how far the anchor misses the target, shared edge, tie-break start)`.
fn approach(
    origin: Rect,
    target: Rect,
    anchor: (u16, u16),
    direction: PaneDirection,
) -> Option<(u16, u16, u16, u16)> {
    let origin_right = origin.x.saturating_add(origin.width);
    let origin_bottom = origin.y.saturating_add(origin.height);
    let target_right = target.x.saturating_add(target.width);
    let target_bottom = target.y.saturating_add(target.height);
    let x_overlap = overlap(origin.x, origin_right, target.x, target_right);
    let y_overlap = overlap(origin.y, origin_bottom, target.y, target_bottom);
    match direction {
        PaneDirection::Left if target_right <= origin.x && y_overlap > 0 => Some((
            origin.x - target_right,
            miss(anchor.1, target.y, target_bottom),
            y_overlap,
            target.y,
        )),
        PaneDirection::Right if target.x >= origin_right && y_overlap > 0 => Some((
            target.x - origin_right,
            miss(anchor.1, target.y, target_bottom),
            y_overlap,
            target.y,
        )),
        PaneDirection::Up if target_bottom <= origin.y && x_overlap > 0 => Some((
            origin.y - target_bottom,
            miss(anchor.0, target.x, target_right),
            x_overlap,
            target.x,
        )),
        PaneDirection::Down if target.y >= origin_bottom && x_overlap > 0 => Some((
            target.y - origin_bottom,
            miss(anchor.0, target.x, target_right),
            x_overlap,
            target.x,
        )),
        _ => None,
    }
}

/// Zero when `point` lies in `[start, end)`.
fn miss(point: u16, start: u16, end: u16) -> u16 {
    if point < start {
        start - point
    } else if point >= end {
        point - end + 1
    } else {
        0
    }
}

fn overlap(a0: u16, a1: u16, b0: u16, b1: u16) -> u16 {
    a1.min(b1).saturating_sub(a0.max(b0))
}

fn leaf_id(layout: &Layout) -> EditorId {
    match layout {
        Layout::Leaf(id) => *id,
        Layout::Split { .. } => unreachable!("caller matched a leaf"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u64) -> EditorId {
        EditorId(n)
    }

    fn diagram() -> Layout {
        // A | (B / C)
        Layout::Leaf(id(0))
            .split(id(0), Axis::Horizontal, id(1))
            .split(id(1), Axis::Vertical, id(2))
    }

    #[test]
    fn arrows_walk_to_the_near_edge_of_the_neighbor() {
        let layout = diagram();
        assert_eq!(
            layout.adjacent(id(0), PaneDirection::Right),
            Some(id(1)),
            "entering a vertical stack from the left lands on the top pane"
        );
        assert_eq!(layout.adjacent(id(1), PaneDirection::Down), Some(id(2)));
        assert_eq!(layout.adjacent(id(2), PaneDirection::Up), Some(id(1)));
        assert_eq!(layout.adjacent(id(2), PaneDirection::Left), Some(id(0)));
        assert_eq!(layout.adjacent(id(1), PaneDirection::Left), Some(id(0)));
        assert_eq!(layout.adjacent(id(0), PaneDirection::Left), None);
        assert_eq!(layout.adjacent(id(0), PaneDirection::Down), None);
        assert_eq!(layout.adjacent(id(2), PaneDirection::Right), None);
        assert_eq!(layout.focus_after_close(id(1)), Some(id(2)));
        assert_eq!(layout.focus_after_close(id(0)), Some(id(1)));
        assert_eq!(Layout::Leaf(id(0)).focus_after_close(id(0)), None);
    }

    #[test]
    fn closing_the_middle_pane_promotes_its_sibling() {
        let layout = diagram();
        let closed = layout.close(id(1)).unwrap();
        assert_eq!(
            closed,
            Layout::Split {
                axis: Axis::Horizontal,
                first: Box::new(Layout::Leaf(id(0))),
                second: Box::new(Layout::Leaf(id(2))),
            }
        );
        assert_eq!(Layout::Leaf(id(0)).close(id(0)), Err(Layout::Leaf(id(0))));
    }

    #[test]
    fn panes_and_dividers_cover_the_area_without_overlap() {
        let layout = diagram();
        let area = Rect::new(2, 3, 20, 11);
        let placed = layout.place(area);
        let mut cells = vec![0u8; (area.width as usize) * (area.height as usize)];
        let mark = |rect: Rect, cells: &mut [u8]| {
            for y in rect.y..rect.y + rect.height {
                for x in rect.x..rect.x + rect.width {
                    let index = (y - area.y) as usize * area.width as usize + (x - area.x) as usize;
                    cells[index] += 1;
                }
            }
        };
        for (_, pane) in &placed.panes {
            mark(*pane, &mut cells);
        }
        for divider in &placed.dividers {
            mark(divider.area, &mut cells);
        }
        assert_eq!(placed.panes.len(), 3);
        assert!(cells.iter().all(|count| *count == 1));
        assert!(placed.dividers.iter().any(|divider| divider.vertical));
        assert!(placed.dividers.iter().any(|divider| !divider.vertical));
    }

    /// Top is `A | B`. Bottom is `(C | D) | E`. Down from B is E, the pane under it.
    fn screenshot_layout() -> (Layout, Vec<(Surface, Rect)>) {
        let layout = Layout::Leaf(id(0))
            .split(id(0), Axis::Vertical, id(1))
            .split(id(0), Axis::Horizontal, id(2))
            .split(id(1), Axis::Horizontal, id(3))
            .split(id(1), Axis::Horizontal, id(4));
        let (_, editors) = body_rects(Rect::new(0, 0, 120, 40), false, SIDEBAR_WIDTH);
        let surfaces = layout
            .place(editors)
            .panes
            .into_iter()
            .map(|(editor, rect)| (Surface::Editor(editor), rect))
            .collect();
        (layout, surfaces)
    }

    fn cell(surfaces: &[(Surface, Rect)], surface: Surface) -> (u16, u16) {
        let rect = surfaces
            .iter()
            .find(|(candidate, _)| *candidate == surface)
            .unwrap()
            .1;
        (rect.x, rect.y)
    }

    fn go(
        surfaces: &[(Surface, Rect)],
        from: Surface,
        direction: PaneDirection,
    ) -> Option<Surface> {
        adjacent_surface(surfaces, from, cell(surfaces, from), direction)
    }

    #[test]
    fn arrow_follows_the_pane_on_that_side_of_the_screen() {
        let (_, surfaces) = screenshot_layout();
        let editor = Surface::Editor;
        assert_eq!(
            go(&surfaces, editor(id(2)), PaneDirection::Down),
            Some(editor(id(3))),
            "the top-right pane moves down into the pane beneath it"
        );
        assert_eq!(
            go(&surfaces, editor(id(3)), PaneDirection::Up),
            Some(editor(id(2)))
        );
        assert_eq!(
            go(&surfaces, editor(id(0)), PaneDirection::Down),
            Some(editor(id(1)))
        );
        assert_eq!(
            go(&surfaces, editor(id(0)), PaneDirection::Right),
            Some(editor(id(2)))
        );
        assert_eq!(
            go(&surfaces, editor(id(4)), PaneDirection::Left),
            Some(editor(id(1)))
        );
        assert_eq!(go(&surfaces, editor(id(0)), PaneDirection::Left), None);
    }

    #[test]
    fn down_follows_the_cursor_column_not_the_wider_pane() {
        let (layout, _) = screenshot_layout();
        let (sidebar, editors) = body_rects(Rect::new(0, 0, 120, 40), true, SIDEBAR_WIDTH);
        let surfaces: Vec<_> = layout
            .place(editors)
            .panes
            .into_iter()
            .map(|(editor, rect)| (Surface::Editor(editor), rect))
            .chain(std::iter::once((Surface::Sidebar, sidebar)))
            .collect();
        let top_left = surfaces
            .iter()
            .find(|(surface, _)| *surface == Surface::Editor(id(0)))
            .unwrap()
            .1;
        let left_column = (top_left.x, top_left.y);
        let right_column = (top_left.x + top_left.width.saturating_sub(1), top_left.y);
        assert_eq!(
            adjacent_surface(
                &surfaces,
                Surface::Editor(id(0)),
                left_column,
                PaneDirection::Down
            ),
            Some(Surface::Editor(id(1))),
            "a cursor on the left stays in the left pane"
        );
        assert_eq!(
            adjacent_surface(
                &surfaces,
                Surface::Editor(id(0)),
                right_column,
                PaneDirection::Down
            ),
            Some(Surface::Editor(id(4))),
            "a cursor on the right of the same pane moves into the pane beneath that column"
        );
    }

    #[test]
    fn closed_sidebar_is_not_a_destination_and_an_open_one_is() {
        let (layout, mut surfaces) = screenshot_layout();
        assert_eq!(
            go(&surfaces, Surface::Editor(id(0)), PaneDirection::Left),
            None
        );
        let (sidebar, editors) = body_rects(Rect::new(0, 0, 120, 40), true, SIDEBAR_WIDTH);
        surfaces = layout
            .place(editors)
            .panes
            .into_iter()
            .map(|(editor, rect)| (Surface::Editor(editor), rect))
            .collect();
        surfaces.push((Surface::Sidebar, sidebar));
        assert_eq!(
            go(&surfaces, Surface::Editor(id(0)), PaneDirection::Left),
            Some(Surface::Sidebar)
        );
        assert_eq!(
            go(&surfaces, Surface::Editor(id(2)), PaneDirection::Left),
            Some(Surface::Editor(id(0))),
            "a pane with an editor on its left does not jump to the sidebar"
        );
        assert_eq!(
            go(&surfaces, Surface::Sidebar, PaneDirection::Right),
            Some(Surface::Editor(id(0)))
        );
    }

    #[test]
    fn sidebar_width_steps_and_stays_within_half_the_screen() {
        assert_eq!(clamp_sidebar_width(120, 30), 30);
        assert_eq!(clamp_sidebar_width(120, 80), 60);
        assert_eq!(clamp_sidebar_width(20, 30), 10);
        assert_eq!(clamp_sidebar_width(0, 30), 0);
        assert_eq!(step_sidebar_width(120, 30, true), 34);
        assert_eq!(step_sidebar_width(120, 58, true), 60);
        assert_eq!(step_sidebar_width(120, 60, true), 60);
        assert_eq!(step_sidebar_width(120, 18, false), 16);
        assert_eq!(step_sidebar_width(120, 16, false), 16);
        let (sidebar, editors) = body_rects(Rect::new(0, 0, 120, 40), true, 44);
        assert_eq!(sidebar.width, 44);
        assert_eq!(editors.x, 44);
        assert_eq!(editors.width, 76);
    }
}
