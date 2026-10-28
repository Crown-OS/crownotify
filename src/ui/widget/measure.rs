//! Reports the laid-out size of whatever it wraps.

use crownui::{
    ext::{Kind, NodeId},
    prelude::{Cx, Element, Signal, Size, ViewSeq},
};

pub struct MeasuredKind<V> {
    content: V,
    size: Signal<Size>,
}

impl<V: ViewSeq> Kind for MeasuredKind<V> {
    const NAME: &'static str = "measured";

    fn mount(self, cx: &mut Cx, node: NodeId) {
        cx.set_children(node, self.content);
        let observed = cx.observe_size(node);
        let size = self.size;
        cx.effect(move |runtime| {
            let measured = observed.get(runtime);
            size.set(runtime, measured);
        });
    }
}

/// `content`, writing its size into `size` every time layout changes it.
pub fn measured<V: ViewSeq>(content: V, size: Signal<Size>) -> Element<MeasuredKind<V>> {
    Element::new(MeasuredKind { content, size })
}
