//! Editing operations on a selection of widgets (indices into a page's widget list): align to the
//! selection's bounds, distribute evenly, same size as a reference, nudge, duplicate, delete.
//! Pure functions, used by the window (buttons, keyboard, mouse).

use crate::config::{SCREEN_H, SCREEN_W, Widget};

/// Edge or centre line to align on (of the selection's bounding box).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Left,
    CenterX,
    Right,
    Top,
    CenterY,
    Bottom,
}

/// Bounding box (x0, y0, x1, y1) of the selected widgets.
pub fn bounds(ws: &[Widget], sel: &[usize]) -> Option<(i32, i32, i32, i32)> {
    sel.iter().filter_map(|&i| ws.get(i)).fold(None, |b, w| {
        let (x0, y0, x1, y1) = (w.x, w.y, w.x + w.w, w.y + w.h);
        Some(match b {
            None => (x0, y0, x1, y1),
            Some((a, b, c, d)) => (a.min(x0), b.min(y0), c.max(x1), d.max(y1)),
        })
    })
}

pub fn align(ws: &mut [Widget], sel: &[usize], edge: Edge) {
    let Some((x0, y0, x1, y1)) = bounds(ws, sel) else {
        return;
    };
    for &i in sel {
        let Some(w) = ws.get_mut(i) else { continue };
        match edge {
            Edge::Left => w.x = x0,
            Edge::CenterX => w.x = (x0 + x1) / 2 - w.w / 2,
            Edge::Right => w.x = x1 - w.w,
            Edge::Top => w.y = y0,
            Edge::CenterY => w.y = (y0 + y1) / 2 - w.h / 2,
            Edge::Bottom => w.y = y1 - w.h,
        }
    }
}

/// Equal gaps between the selected widgets (at least 3), first and last staying in place.
pub fn distribute(ws: &mut [Widget], sel: &[usize], horizontal: bool) {
    let mut order: Vec<usize> = sel.iter().copied().filter(|&i| i < ws.len()).collect();
    if order.len() < 3 {
        return;
    }
    let pos = |w: &Widget| if horizontal { (w.x, w.w) } else { (w.y, w.h) };
    order.sort_by_key(|&i| pos(&ws[i]).0);
    let (first, last) = (pos(&ws[order[0]]), pos(&ws[*order.last().unwrap()]));
    let span = last.0 + last.1 - first.0;
    let total: i32 = order.iter().map(|&i| pos(&ws[i]).1).sum();
    let gap = (span - total) as f32 / (order.len() - 1) as f32;
    let mut at = first.0 as f32;
    for &i in &order {
        let w = &mut ws[i];
        if horizontal {
            w.x = at.round() as i32;
            at += w.w as f32 + gap;
        } else {
            w.y = at.round() as i32;
            at += w.h as f32 + gap;
        }
    }
}

/// Width (or height) of `reference` for every selected widget.
pub fn same_size(ws: &mut [Widget], sel: &[usize], reference: usize, width: bool) {
    let Some(r) = ws.get(reference).map(|w| (w.w, w.h)) else {
        return;
    };
    for &i in sel {
        if let Some(w) = ws.get_mut(i) {
            if width {
                w.w = r.0;
            } else {
                w.h = r.1;
            }
        }
    }
}

/// Move the selection by (dx, dy), keeping at least 8 px of each widget on the screen.
pub fn nudge(ws: &mut [Widget], sel: &[usize], dx: i32, dy: i32) {
    for &i in sel {
        if let Some(w) = ws.get_mut(i) {
            w.x = (w.x + dx).clamp(8 - w.w, SCREEN_W - 8);
            w.y = (w.y + dy).clamp(8 - w.h, SCREEN_H - 8);
        }
    }
}

/// Copies of the selected widgets, 16 px down-right, added on top; returns their indices.
pub fn duplicate(ws: &mut Vec<Widget>, sel: &[usize]) -> Vec<usize> {
    let mut sorted: Vec<usize> = sel.iter().copied().filter(|&i| i < ws.len()).collect();
    sorted.sort_unstable();
    sorted.dedup();
    let mut out = Vec::new();
    for i in sorted {
        let mut copy = ws[i].clone();
        copy.x = (copy.x + 16).min(SCREEN_W - 8);
        copy.y = (copy.y + 16).min(SCREEN_H - 8);
        ws.push(copy);
        out.push(ws.len() - 1);
    }
    out
}

pub fn delete(ws: &mut Vec<Widget>, sel: &[usize]) {
    let mut sorted: Vec<usize> = sel.iter().copied().filter(|&i| i < ws.len()).collect();
    sorted.sort_unstable();
    sorted.dedup();
    for i in sorted.into_iter().rev() {
        ws.remove(i);
    }
}

/// Widgets meeting the rectangle (x0, y0, x1, y1).
pub fn in_rect(ws: &[Widget], (x0, y0, x1, y1): (i32, i32, i32, i32)) -> Vec<usize> {
    ws.iter()
        .enumerate()
        .filter(|(_, w)| w.x < x1 && x0 < w.x + w.w && w.y < y1 && y0 < w.y + w.h)
        .map(|(i, _)| i)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::WidgetKind;

    fn w(x: i32, y: i32, ww: i32, h: i32) -> Widget {
        Widget {
            x,
            y,
            w: ww,
            h,
            frame: None,
            kind: WidgetKind::template("rect").unwrap(),
        }
    }

    #[test]
    fn align_on_the_selection_bounds() {
        let mut ws = vec![w(10, 10, 20, 20), w(50, 30, 40, 10), w(0, 0, 5, 5)];
        align(&mut ws, &[0, 1], Edge::Top);
        assert_eq!((ws[0].y, ws[1].y, ws[2].y), (10, 10, 0));
        align(&mut ws, &[0, 1], Edge::Right);
        assert_eq!((ws[0].x + ws[0].w, ws[1].x + ws[1].w), (90, 90));
        align(&mut ws, &[0, 1], Edge::CenterY);
        assert_eq!(ws[0].y + ws[0].h / 2, ws[1].y + ws[1].h / 2);
    }

    #[test]
    fn distribute_makes_equal_gaps() {
        let mut ws = vec![w(0, 0, 10, 10), w(17, 0, 10, 10), w(90, 0, 10, 10)];
        distribute(&mut ws, &[2, 0, 1], true);
        assert_eq!((ws[0].x, ws[1].x, ws[2].x), (0, 45, 90));
    }

    #[test]
    fn duplicate_delete_and_select_in_a_rectangle() {
        let mut ws = vec![w(0, 0, 10, 10), w(100, 100, 10, 10)];
        let copies = duplicate(&mut ws, &[1, 0]);
        assert_eq!(copies, vec![2, 3]);
        assert_eq!((ws[2].x, ws[3].x), (16, 116));
        assert_eq!(in_rect(&ws, (0, 0, 30, 30)), vec![0, 2]);
        delete(&mut ws, &[0, 2]);
        assert_eq!(ws.len(), 2);
        same_size(&mut ws, &[0, 1], 0, true);
        nudge(&mut ws, &[0], -1000, 0);
        assert_eq!(ws[0].x, 8 - ws[0].w);
    }
}
