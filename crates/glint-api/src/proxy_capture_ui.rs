use super::proxy_service::{CaptureStore, CaptureSummary};
use super::*;
use std::sync::Arc;

pub(super) struct CapturePanel {
    store: Arc<CaptureStore>,
    revision: u64,
    rows: Vec<CaptureSummary>,
    table: Entity<DataTable>,
    search: Entity<TextEntry>,
    selected: Option<u64>,
    detail: Entity<CodeCard>,
    detail_tab: usize,
    theme: ThemePreset,
    profile: String,
    _subscriptions: Vec<Subscription>,
    _poll: Task<()>,
}
impl CapturePanel {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let table = cx.new(|cx| {
            DataTable::new(
                cx,
                [
                    TableColumn::new("时间", 106.),
                    TableColumn::new("方法", 70.),
                    TableColumn::new("URL", 180.).flexible(),
                    TableColumn::new("状态", 80.),
                    TableColumn::new("耗时", 80.),
                    TableColumn::new("大小", 80.),
                ],
                [],
            )
            .compact()
            .max_height(300.)
            .page_size(1000)
            .empty_row("暂无抓包记录。启动代理后，通过端口或浏览器发送请求。")
        });
        let search = cx.new(|cx| TextEntry::new(cx, "搜索 URL、方法或状态", ""));
        let detail = cx.new(|cx| {
            CodeCard::new(cx, "http", "选择一条记录查看请求与响应")
                .read_only(true)
                .show_header(false)
                .viewport()
        });
        let subscriptions = vec![
            cx.subscribe(&table, |this, _, event, cx| {
                if let TableEvent::SelectionChanged(ids) = event {
                    this.selected = ids.first().and_then(|id| id.parse().ok());
                    this.update_detail(cx);
                }
            }),
            cx.observe(&search, |this, search, cx| {
                let query = search.read(cx).value().to_owned();
                this.table
                    .update(cx, |table, cx| table.set_search_query(query, cx));
            }),
        ];
        let poll = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(250))
                    .await;
                if this.update(cx, |this, cx| this.refresh(cx)).is_err() {
                    break;
                }
            }
        });
        Self {
            store: Arc::default(),
            revision: u64::MAX,
            rows: vec![],
            table,
            search,
            selected: None,
            detail,
            detail_tab: 0,
            theme: ThemePreset::Moonstone,
            profile: String::new(),
            _subscriptions: subscriptions,
            _poll: poll,
        }
    }
    pub fn bind_profile(
        &mut self,
        profile: &str,
        store: Option<Arc<CaptureStore>>,
        cx: &mut Context<Self>,
    ) {
        if self.profile != profile {
            self.profile = profile.into();
            self.search.update(cx, |entry, cx| entry.set_value("", cx));
            self.set_store(store.unwrap_or_default(), cx);
        } else if let Some(store) = store {
            self.set_store(store, cx);
        }
        self.refresh(cx);
    }
    pub fn set_store(&mut self, store: Arc<CaptureStore>, cx: &mut Context<Self>) {
        if Arc::ptr_eq(&self.store, &store) {
            return;
        }
        self.store = store;
        self.revision = u64::MAX;
        self.selected = None;
        self.refresh(cx);
    }
    pub fn set_theme(&mut self, theme: ThemePreset, cx: &mut Context<Self>) {
        if self.theme == theme {
            return;
        }
        self.theme = theme;
        self.table
            .update(cx, |table, cx| table.set_theme(theme, cx));
        self.search
            .update(cx, |entry, cx| entry.set_theme(theme, cx));
        self.detail.update(cx, |card, cx| card.set_theme(theme, cx));
        cx.notify();
    }
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let revision = self.store.revision();
        if self.revision == revision {
            return;
        }
        self.revision = revision;
        self.rows = self.store.summaries();
        let rows = self
            .rows
            .iter()
            .map(|r| {
                TableRow::new(
                    r.id.to_string(),
                    [
                        TableCell::mono(r.time.clone()),
                        TableCell::mono(r.method.clone()),
                        TableCell::text(r.url.clone()),
                        TableCell::text(
                            r.status
                                .map_or_else(|| r.state.clone(), |status| status.to_string()),
                        ),
                        TableCell::mono(format!("{} ms", r.elapsed_ms)),
                        TableCell::mono(format!("{} B", r.bytes)),
                    ],
                )
            })
            .collect::<Vec<_>>();
        self.table.update(cx, |table, cx| table.set_rows(rows, cx));
        if self
            .selected
            .is_some_and(|id| !self.rows.iter().any(|r| r.id == id))
        {
            self.selected = None;
        }
        self.update_detail(cx);
        cx.notify();
    }
    fn update_detail(&mut self, cx: &mut Context<Self>) {
        let text = self
            .selected
            .and_then(|id| self.store.record(id))
            .map(|r| {
                let (heading, headers, body) = if self.detail_tab == 0 {
                    (
                        format!("{} {}", r.method, r.url),
                        &r.request_headers,
                        &r.request,
                    )
                } else {
                    (
                        format!(
                            "HTTP {} · {} · {} ms",
                            r.status.map_or("—".into(), |s| s.to_string()),
                            r.state,
                            r.elapsed_ms
                        ),
                        &r.response_headers,
                        &r.response,
                    )
                };
                format!(
                    "{heading}\n客户端：{}\n{}\n\n{}{}",
                    r.peer,
                    headers
                        .iter()
                        .map(|(k, v)| format!("{k}: {v}"))
                        .collect::<Vec<_>>()
                        .join("\n"),
                    body.display(),
                    r.error.map_or(String::new(), |e| format!("\n\n错误：{e}"))
                )
            })
            .unwrap_or_else(|| "选择一条记录查看请求与响应".into());
        self.detail
            .update(cx, |card, cx| card.set_content(text, cx));
        cx.notify();
    }
}
impl Render for CapturePanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        div().id("proxy-capture-page").debug_selector(|| "proxy-capture-page".into())
            .size_full().min_h_0().overflow_y_scroll().flex().flex_col().gap_3()
            .child(div().flex().items_center().gap_3()
                .child(div().font_weight(FontWeight::SEMIBOLD).child(format!("代理抓包 · {} / 1000 条", self.rows.len())))
                .child(SearchInput::new("搜索 URL、方法或状态").control(self.search.clone()).theme(theme).render())
                .child(div().flex_1())
                .child(Button::new("清空记录").kind(ButtonKind::Plain).size(ButtonSize::Small).theme(theme).render()
                    .debug_selector(|| "proxy-capture-clear".into())
                    .on_click(cx.listener(|this, _, _, cx| { this.store.clear(); this.refresh(cx); }))))
            .child(div().text_size(px(11.)).text_color(rgb(theme.palette().text_muted))
                .child("仅保留当前会话最近 1000 条记录；请求和响应各预览前 32 KiB。停止代理后仍可查看，关闭应用后清除。"))
            .child(self.table.clone())
            .child(SegmentedControl::new([SegmentOption::new("请求"), SegmentOption::new("响应")])
                .selected(self.detail_tab).theme(theme)
                .on_change({ let owner = cx.entity().downgrade(); move |tab, _, cx| { let _ = owner.update(cx, |this, cx| { this.detail_tab = tab; this.update_detail(cx); }); } }).render())
            .child(div().flex_1().min_h(px(200.)).child(self.detail.clone()))
    }
}
