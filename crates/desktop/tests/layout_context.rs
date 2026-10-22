use std::{cell::Cell, rc::Rc};

use taffy::{Style, TaffyTree};

struct ContextDrop(Rc<Cell<usize>>);

impl Drop for ContextDrop {
    fn drop(&mut self) {
        self.0.set(self.0.get() + 1);
    }
}

#[test]
fn clearing_layout_tree_releases_all_measurement_contexts() {
    let dropped = Rc::new(Cell::new(0));
    let mut tree = TaffyTree::new();
    for _ in 0..8 {
        tree.new_leaf_with_context(Style::default(), ContextDrop(dropped.clone()))
            .unwrap();
    }

    tree.clear();

    assert_eq!(dropped.get(), 8);
    tree.new_leaf(Style::default()).unwrap();
    assert_eq!(dropped.get(), 8);
}

#[test]
fn removing_layout_node_releases_its_measurement_context() {
    let dropped = Rc::new(Cell::new(0));
    let mut tree = TaffyTree::new();
    let node = tree
        .new_leaf_with_context(Style::default(), ContextDrop(dropped.clone()))
        .unwrap();

    tree.remove(node).unwrap();

    assert_eq!(dropped.get(), 1);
    tree.new_leaf(Style::default()).unwrap();
    assert_eq!(dropped.get(), 1);
}
