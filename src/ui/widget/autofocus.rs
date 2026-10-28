//! Takes keyboard focus for the field it wraps as soon as it is shown.

use crownui::{
    ext::{Kind, NodeId},
    prelude::{Cx, Element, View},
};

pub struct AutofocusKind<V> {
    content: V,
}

impl<V: View> Kind for AutofocusKind<V> {
    const NAME: &'static str = "autofocus";

    fn mount(self, cx: &mut Cx, node: NodeId) {
        let content = cx.build(self.content);
        cx.adopt_children(node, vec![content]);
        cx.request_focus(content);
    }
}

pub fn autofocus<V: View>(content: V) -> Element<AutofocusKind<V>> {
    Element::new(AutofocusKind { content })
}
