//! Optional split layout for the chat area. One pane is the ordinary app;
//! splitting divides the focused pane in two, like a terminal multiplexer.

use smaller_talk_domain::{PaneAxis, PaneLayout, PaneNode};
use std::collections::{HashMap, HashSet};

use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct PaneId(u64);

impl PaneId {
    pub fn index(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Axis {
    /// Panes sit side by side.
    Row,
    /// Panes are stacked.
    Column,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Layout {
    Pane(PaneId),
    Split(Split),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Split {
    pub id: u64,
    pub axis: Axis,
    /// Share of the split given to `first`, between 0 and 1.
    pub ratio: f32,
    pub first: Box<Layout>,
    pub second: Box<Layout>,
}

pub(crate) struct Panes {
    root: Layout,
    focused: PaneId,
    /// A chat is shown in at most one pane; panes without an entry are empty.
    chats: HashMap<PaneId, Uuid>,
    next: u64,
}

impl Panes {
    pub fn snapshot(&self) -> PaneLayout {
        fn node(layout: &Layout, chats: &HashMap<PaneId, Uuid>) -> PaneNode {
            match layout {
                Layout::Pane(pane) => PaneNode::Pane {
                    id: pane.0,
                    chat: chats.get(pane).copied(),
                },
                Layout::Split(split) => PaneNode::Split {
                    id: split.id,
                    axis: match split.axis {
                        Axis::Row => PaneAxis::Row,
                        Axis::Column => PaneAxis::Column,
                    },
                    ratio: split.ratio,
                    first: Box::new(node(&split.first, chats)),
                    second: Box::new(node(&split.second, chats)),
                },
            }
        }
        PaneLayout {
            root: node(&self.root, &self.chats),
            focused: self.focused.0,
        }
    }

    pub fn restore(saved: &PaneLayout, open: &[Uuid]) -> Option<Self> {
        fn node(
            saved: &PaneNode,
            depth: usize,
            ids: &mut HashSet<u64>,
            chats: &mut HashMap<PaneId, Uuid>,
            open: &[Uuid],
        ) -> Option<Layout> {
            if depth > 8 || ids.len() >= 63 {
                return None;
            }
            match saved {
                PaneNode::Pane { id, chat } => {
                    if !ids.insert(*id) {
                        return None;
                    }
                    let pane = PaneId(*id);
                    if let Some(chat) = chat.filter(|chat| {
                        open.contains(chat) && !chats.values().any(|shown| shown == chat)
                    }) {
                        chats.insert(pane, chat);
                    }
                    Some(Layout::Pane(pane))
                }
                PaneNode::Split {
                    id,
                    axis,
                    ratio,
                    first,
                    second,
                } => {
                    if !ids.insert(*id) || !ratio.is_finite() {
                        return None;
                    }
                    Some(Layout::Split(Split {
                        id: *id,
                        axis: match axis {
                            PaneAxis::Row => Axis::Row,
                            PaneAxis::Column => Axis::Column,
                        },
                        ratio: ratio.clamp(0.1, 0.9),
                        first: Box::new(node(first, depth + 1, ids, chats, open)?),
                        second: Box::new(node(second, depth + 1, ids, chats, open)?),
                    }))
                }
            }
        }
        let mut ids = HashSet::new();
        let mut chats = HashMap::new();
        let root = node(&saved.root, 0, &mut ids, &mut chats, open)?;
        let next = ids.iter().max()?.checked_add(1)?;
        let mut panes = Self {
            focused: PaneId(saved.focused),
            root,
            chats,
            next,
        };
        if !panes.panes().contains(&panes.focused) {
            panes.focused = first_pane(&panes.root);
        }
        Some(panes)
    }

    pub fn new(chat: Option<Uuid>) -> Self {
        let pane = PaneId(0);
        Self {
            root: Layout::Pane(pane),
            focused: pane,
            chats: chat.map(|chat| (pane, chat)).into_iter().collect(),
            next: 1,
        }
    }

    pub fn root(&self) -> &Layout {
        &self.root
    }

    pub fn focused(&self) -> PaneId {
        self.focused
    }

    pub fn is_split(&self) -> bool {
        matches!(self.root, Layout::Split(_))
    }

    pub fn chat(&self, pane: PaneId) -> Option<Uuid> {
        self.chats.get(&pane).copied()
    }

    /// The chat in the focused pane.
    pub fn active(&self) -> Option<Uuid> {
        self.chat(self.focused)
    }

    pub fn pane_of(&self, chat: Uuid) -> Option<PaneId> {
        self.chats
            .iter()
            .find_map(|(pane, shown)| (*shown == chat).then_some(*pane))
    }

    /// Whether `chat` is visible in a pane other than the focused one.
    pub fn shown_elsewhere(&self, chat: Uuid) -> bool {
        self.pane_of(chat).is_some_and(|pane| pane != self.focused)
    }

    pub fn panes(&self) -> Vec<PaneId> {
        let mut panes = Vec::new();
        collect(&self.root, &mut panes);
        panes
    }

    /// Returns whether focus moved.
    pub fn focus(&mut self, pane: PaneId) -> bool {
        if pane == self.focused || !self.panes().contains(&pane) {
            return false;
        }
        self.focused = pane;
        true
    }

    /// Show `chat` in the focused pane, or focus the pane already showing it.
    pub fn show(&mut self, chat: Option<Uuid>) {
        match chat {
            Some(chat) => match self.pane_of(chat) {
                Some(pane) => self.focused = pane,
                None => {
                    self.chats.insert(self.focused, chat);
                }
            },
            None => {
                self.chats.remove(&self.focused);
            }
        }
    }

    /// Remove `chat` from whichever pane shows it, leaving that pane empty.
    pub fn clear(&mut self, chat: Uuid) {
        self.chats.retain(|_, shown| *shown != chat);
    }

    /// Split the focused pane. The new, empty pane takes focus.
    pub fn split(&mut self, axis: Axis) -> PaneId {
        let pane = PaneId(self.next);
        let split = self.next + 1;
        self.next += 2;
        let target = self.focused;
        replace(&mut self.root, target, &mut |old| {
            Layout::Split(Split {
                id: split,
                axis,
                ratio: 0.5,
                first: Box::new(old),
                second: Box::new(Layout::Pane(pane)),
            })
        });
        self.focused = pane;
        pane
    }

    /// Close `pane`; its sibling takes its space. The last pane cannot close.
    /// Returns whether the pane closed.
    pub fn close(&mut self, pane: PaneId) -> bool {
        let Some(sibling) = remove(&mut self.root, pane) else {
            return false;
        };
        self.chats.remove(&pane);
        if self.focused == pane {
            self.focused = sibling;
        }
        true
    }

    pub fn resize(&mut self, split: u64, ratio: f32) -> bool {
        if !ratio.is_finite() {
            return false;
        }
        let ratio = ratio.clamp(0.1, 0.9);
        match find_split(&mut self.root, split) {
            Some(node) if node.ratio != ratio => {
                node.ratio = ratio;
                true
            }
            _ => false,
        }
    }

    pub fn even_out(&mut self) {
        even_out(&mut self.root);
    }
}

fn collect(layout: &Layout, panes: &mut Vec<PaneId>) {
    match layout {
        Layout::Pane(pane) => panes.push(*pane),
        Layout::Split(split) => {
            collect(&split.first, panes);
            collect(&split.second, panes);
        }
    }
}

fn first_pane(layout: &Layout) -> PaneId {
    match layout {
        Layout::Pane(pane) => *pane,
        Layout::Split(split) => first_pane(&split.first),
    }
}

fn replace(layout: &mut Layout, target: PaneId, build: &mut dyn FnMut(Layout) -> Layout) -> bool {
    match layout {
        Layout::Pane(pane) if *pane == target => {
            let old = std::mem::replace(layout, Layout::Pane(target));
            *layout = build(old);
            true
        }
        Layout::Pane(_) => false,
        Layout::Split(split) => {
            replace(&mut split.first, target, build) || replace(&mut split.second, target, build)
        }
    }
}

/// Remove `target` and return the pane that should take focus in its place.
fn remove(layout: &mut Layout, target: PaneId) -> Option<PaneId> {
    let Layout::Split(split) = layout else {
        return None;
    };
    let survivor = if *split.first == Layout::Pane(target) {
        Some(std::mem::replace(&mut *split.second, Layout::Pane(target)))
    } else if *split.second == Layout::Pane(target) {
        Some(std::mem::replace(&mut *split.first, Layout::Pane(target)))
    } else {
        None
    };
    match survivor {
        Some(survivor) => {
            let focus = first_pane(&survivor);
            *layout = survivor;
            Some(focus)
        }
        None => remove(&mut split.first, target).or_else(|| remove(&mut split.second, target)),
    }
}

fn find_split(layout: &mut Layout, id: u64) -> Option<&mut Split> {
    let Layout::Split(split) = layout else {
        return None;
    };
    if split.id == id {
        return Some(split);
    }
    find_split(&mut split.first, id).or_else(|| find_split(&mut split.second, id))
}

fn even_out(layout: &mut Layout) {
    if let Layout::Split(split) = layout {
        split.ratio = 0.5;
        even_out(&mut split.first);
        even_out(&mut split.second);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_round_trip_keeps_nested_splits_focus_ratios_and_chat_ownership() {
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        let mut panes = Panes::new(Some(a));
        panes.split(Axis::Row);
        panes.show(Some(b));
        let empty = panes.split(Axis::Column);
        let Layout::Split(split) = panes.root() else {
            panic!("missing split");
        };
        panes.resize(split.id, 0.35);
        let saved: PaneLayout =
            serde_json::from_str(&serde_json::to_string(&panes.snapshot()).unwrap()).unwrap();
        let mut restored = Panes::restore(&saved, &[a, b]).unwrap();
        assert_eq!(restored.snapshot(), saved);
        assert_eq!(restored.focused(), empty);
        assert_eq!(restored.active(), None);
        assert!(restored.close(empty));
        assert_eq!(restored.active(), Some(b));
        let next = restored.split(Axis::Row);
        assert!(!panes.panes().contains(&next));
    }

    #[test]
    fn restore_clears_missing_chats_and_rejects_ambiguous_ids() {
        let chat = Uuid::new_v4();
        let mut panes = Panes::new(Some(chat));
        panes.split(Axis::Row);
        let mut saved = panes.snapshot();
        let restored = Panes::restore(&saved, &[]).unwrap();
        assert_eq!(restored.pane_of(chat), None);
        if let PaneNode::Split { first, second, .. } = &mut saved.root {
            *second = first.clone();
        }
        assert!(Panes::restore(&saved, &[chat]).is_none());
    }

    #[test]
    fn single_pane_is_the_unsplit_app() {
        let chat = Uuid::new_v4();
        let panes = Panes::new(Some(chat));
        assert!(!panes.is_split());
        assert_eq!(panes.active(), Some(chat));
        assert_eq!(panes.panes().len(), 1);
    }

    #[test]
    fn split_focuses_a_new_empty_pane_and_keeps_the_original_chat() {
        let chat = Uuid::new_v4();
        let mut panes = Panes::new(Some(chat));
        let first = panes.focused();
        let second = panes.split(Axis::Row);
        assert!(panes.is_split());
        assert_eq!(panes.focused(), second);
        assert_eq!(panes.active(), None);
        assert_eq!(panes.chat(first), Some(chat));
        assert_eq!(panes.panes(), vec![first, second]);
    }

    #[test]
    fn showing_a_visible_chat_focuses_its_pane_instead_of_duplicating_it() {
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        let mut panes = Panes::new(Some(a));
        let first = panes.focused();
        panes.split(Axis::Column);
        panes.show(Some(b));
        assert!(panes.shown_elsewhere(a));
        panes.show(Some(a));
        assert_eq!(panes.focused(), first);
        assert_eq!(
            panes.panes().iter().filter_map(|p| panes.chat(*p)).count(),
            2
        );
    }

    #[test]
    fn nested_splits_close_back_to_their_sibling() {
        let mut panes = Panes::new(None);
        let first = panes.focused();
        let second = panes.split(Axis::Row);
        let third = panes.split(Axis::Column);
        assert_eq!(panes.panes(), vec![first, second, third]);
        assert!(panes.close(second));
        assert_eq!(panes.panes(), vec![first, third]);
        assert_eq!(panes.focused(), third);
        assert!(panes.close(third));
        assert_eq!(panes.focused(), first);
        assert!(!panes.is_split());
        assert!(!panes.close(first));
    }

    #[test]
    fn closing_the_focused_pane_moves_focus_into_the_sibling_subtree() {
        let mut panes = Panes::new(None);
        let first = panes.focused();
        panes.split(Axis::Row);
        panes.focus(first);
        let below = panes.split(Axis::Column);
        panes.focus(first);
        assert!(panes.close(first));
        assert_eq!(panes.focused(), below);
    }

    #[test]
    fn clearing_a_chat_empties_its_pane_and_resize_targets_one_split() {
        let chat = Uuid::new_v4();
        let mut panes = Panes::new(Some(chat));
        panes.split(Axis::Row);
        let Layout::Split(split) = panes.root().clone() else {
            panic!("expected a split");
        };
        assert!(panes.resize(split.id, 0.3));
        assert!(!panes.resize(split.id + 100, 0.3));
        panes.clear(chat);
        assert_eq!(panes.pane_of(chat), None);
        panes.even_out();
        let Layout::Split(split) = panes.root() else {
            panic!("expected a split");
        };
        assert_eq!(split.ratio, 0.5);
    }
}
