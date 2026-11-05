//! Remembers where an element was laid out on its window.

use crownui::{
    ext::{Kind, NodeId},
    prelude::{Cx, Element, Rect, Signal},
};

/// Refers to the element [`tracked`] mounted, for a handler that has to tell
/// the compositor where something is on screen.
#[derive(Clone, Copy)]
pub struct NodeRef(Signal<Option<NodeId>>);

impl NodeRef {
    pub fn new(cx: &mut Cx) -> Self {
        Self(cx.signal(None))
    }

    /// The element's rectangle in window coordinates, once it is mounted.
    pub fn window_rect(self, cx: &Cx) -> Option<Rect> {
        let node = self.0.with_untracked(cx, |node| *node)?;
        cx.tree().get(node).map(|node| node.geometry().window_rect)
    }
}

pub struct TrackedKind(NodeRef);

impl Kind for TrackedKind {
    const NAME: &'static str = "tracked";

    fn mount(self, cx: &mut Cx, node: NodeId) {
        self.0.0.set(cx, Some(node));
    }
}

/// An empty element `node` refers to. Laid over another element, it reports
/// that element's place on the window.
pub fn tracked(node: NodeRef) -> Element<TrackedKind> {
    Element::new(TrackedKind(node))
}
