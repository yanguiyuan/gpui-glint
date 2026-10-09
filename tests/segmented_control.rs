use std::{cell::RefCell, rc::Rc};

use gpui::{
    Bounds, Context, IntoElement, Modifiers, ParentElement, Pixels, Render, Styled, TestAppContext,
    Window, div,
};
use gpui_glint::components::{SegmentOption, SegmentedControl};

#[derive(Default)]
struct Controls {
    explicit_ids: bool,
    clicks: Rc<RefCell<Vec<(usize, usize)>>>,
    bounds: Rc<RefCell<Vec<Vec<Bounds<Pixels>>>>>,
}

impl Render for Controls {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let mut root = div().flex().flex_col().gap_3();
        for (control, mut segments) in [
            SegmentedControl::new([SegmentOption::new("排版"), SegmentOption::new("源码")]),
            SegmentedControl::new([
                SegmentOption::new("全部"),
                SegmentOption::new("工作区"),
                SegmentOption::new("外部"),
            ]),
        ]
        .into_iter()
        .enumerate()
        {
            if self.explicit_ids {
                segments = segments.id(("test-segmented", control));
            }
            let clicks = self.clicks.clone();
            let bounds = self.bounds.clone();
            root = root.child(
                segments
                    .on_change(move |index, _, _| clicks.borrow_mut().push((control, index)))
                    .render()
                    .on_children_prepainted(move |children, _, _| {
                        let mut bounds = bounds.borrow_mut();
                        bounds.resize_with(2, Vec::new);
                        bounds[control] = children;
                    }),
            );
        }
        root
    }
}

#[gpui::test]
fn clicking_each_segment_only_calls_its_own_handler(cx: &mut TestAppContext) {
    check_clicks(cx, false);
}

#[gpui::test]
fn explicit_control_ids_keep_clicks_independent(cx: &mut TestAppContext) {
    check_clicks(cx, true);
}

fn check_clicks(cx: &mut TestAppContext, explicit_ids: bool) {
    let controls = Controls {
        explicit_ids,
        ..Default::default()
    };
    let clicks = controls.clicks.clone();
    let bounds = controls.bounds.clone();
    let (_, cx) = cx.add_window_view(|_, _| controls);
    for control in [1, 0, 1] {
        let children = bounds.borrow()[control].clone();
        for (index, child) in children.iter().enumerate() {
            clicks.borrow_mut().clear();
            cx.simulate_click(child.center(), Modifiers::default());
            assert_eq!(*clicks.borrow(), vec![(control, index)]);
        }
    }
}
