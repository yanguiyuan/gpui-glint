//! Native presentation for engine-independent plugin page snapshots.
use super::{Button, DataTable, TableCell, TableColumn, TableRow};
use crate::{
    plugins::{Node, Page, Result},
    theme::ThemePreset,
};
use gpui::{
    AppContext, Context, Entity, EventEmitter, InteractiveElement, IntoElement, ParentElement,
    Render, SharedString, StatefulInteractiveElement, Styled, Window, div, px, rgb,
};
use std::collections::HashMap;
#[derive(Clone, Debug)]
pub struct PluginPageEvent {
    pub action: String,
    pub data: serde_json::Value,
}
/// Hosts route actions to their plugin runtime and commit validated snapshots with `set_page`.
/// No JavaScript executes on the rendering path.
pub struct PluginPage {
    page: Page,
    theme: ThemePreset,
    tables: HashMap<String, (Vec<String>, Entity<DataTable>)>,
}
impl EventEmitter<PluginPageEvent> for PluginPage {}
impl PluginPage {
    pub fn new(page: Page, cx: &mut Context<Self>) -> Result<Self> {
        page.validate()?;
        let mut this = Self {
            page,
            theme: ThemePreset::default(),
            tables: HashMap::new(),
        };
        this.sync_tables(cx);
        Ok(this)
    }
    pub fn set_page(&mut self, page: Page, cx: &mut Context<Self>) -> Result<()> {
        page.validate()?;
        self.page = page;
        self.sync_tables(cx);
        cx.notify();
        Ok(())
    }
    pub fn set_theme(&mut self, theme: ThemePreset, cx: &mut Context<Self>) {
        self.theme = theme;
        for (_, table) in self.tables.values() {
            table.update(cx, |t, cx| t.set_theme(theme, cx));
        }
        cx.notify();
    }
    fn sync_tables(&mut self, cx: &mut Context<Self>) {
        fn collect(node: &Node, tables: &mut Vec<(String, Vec<String>, Vec<Vec<String>>)>) {
            match node {
                Node::Table { id, columns, rows } => {
                    tables.push((id.clone(), columns.clone(), rows.clone()))
                }
                Node::Column { children } | Node::Row { children } => {
                    for child in children {
                        collect(child, tables)
                    }
                }
                _ => {}
            }
        }
        let mut definitions = vec![];
        collect(&self.page.body, &mut definitions);
        self.tables
            .retain(|id, _| definitions.iter().any(|(current, _, _)| current == id));
        for (id, columns, rows) in definitions {
            let rows: Vec<_> = rows
                .into_iter()
                .enumerate()
                .map(|(i, row)| {
                    TableRow::new(format!("{id}-{i}"), row.into_iter().map(TableCell::text))
                })
                .collect();
            if let Some((old, table)) = self.tables.get(&id).filter(|(old, _)| *old == columns) {
                let _ = old;
                table.update(cx, |table, cx| table.set_rows(rows, cx));
            } else {
                let theme = self.theme;
                let table = cx.new(|cx| {
                    DataTable::new(
                        cx,
                        columns
                            .iter()
                            .map(|c| TableColumn::new(c.clone(), 120.).flexible()),
                        rows,
                    )
                    .theme(theme)
                });
                self.tables.insert(id, (columns, table));
            }
        }
    }
    fn node(&self, node: &Node, cx: &mut Context<Self>) -> gpui::AnyElement {
        match node {
            Node::Text { text } => div()
                .text_size(px(13.))
                .child(text.clone())
                .into_any_element(),
            Node::Column { children } | Node::Row { children } => {
                let mut el = div().min_w_0().flex().gap_3();
                if matches!(node, Node::Column { .. }) {
                    el = el.flex_col();
                }
                for child in children {
                    el = el.child(self.node(child, cx));
                }
                el.into_any_element()
            }
            Node::Button {
                id,
                label,
                action,
                data,
            } => {
                let action = action.clone();
                let data = data.clone();
                div()
                    .id(SharedString::from(id.clone()))
                    .debug_selector(|| id.clone())
                    .on_click(cx.listener(move |_, _, _, cx| {
                        cx.emit(PluginPageEvent {
                            action: action.clone(),
                            data: data.clone(),
                        })
                    }))
                    .child(Button::new(label.clone()).theme(self.theme).render())
                    .into_any_element()
            }
            Node::Table { id, .. } => div()
                .w_full()
                .h(px(280.))
                .child(self.tables[id].1.clone())
                .into_any_element(),
        }
    }
}
impl Render for PluginPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("glint-plugin-page")
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_3()
            .p_4()
            .text_color(rgb(self.theme.palette().text))
            .child(div().text_size(px(18.)).child(self.page.title.clone()))
            .child(self.node(&self.page.body, cx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[gpui::test]
    fn actions_emit_data_and_invalid_updates_preserve_page(cx: &mut gpui::TestAppContext) {
        let page = Page {
            title: "Database plugin".into(),
            body: Node::Button {
                id: "refresh".into(),
                label: "Refresh".into(),
                action: "database.refresh".into(),
                data: serde_json::json!({"table":"notes"}),
            },
        };
        let (view, window) = cx.add_window_view(|_, cx| PluginPage::new(page, cx).unwrap());
        let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let received = events.clone();
        let _subscription = window.update(|_, cx| {
            cx.subscribe(&view, move |_, event, _| {
                received.borrow_mut().push(event.clone())
            })
        });
        let bounds = window.debug_bounds("refresh").unwrap();
        window.simulate_click(bounds.center(), gpui::Modifiers::default());
        assert_eq!(events.borrow()[0].action, "database.refresh");
        assert_eq!(
            events.borrow()[0].data,
            serde_json::json!({"table":"notes"})
        );
        view.update(window, |view, cx| {
            let invalid = Page {
                title: "Rejected".into(),
                body: Node::Table {
                    id: "table".into(),
                    columns: vec!["Name".into()],
                    rows: vec![vec![]],
                },
            };
            assert!(view.set_page(invalid, cx).is_err());
            assert_eq!(view.page.title, "Database plugin");
        });
    }
}
