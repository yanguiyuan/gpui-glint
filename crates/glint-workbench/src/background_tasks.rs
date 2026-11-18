use super::*;
use gpui_glint::components::{BackgroundTask, BackgroundTasksMenu};

pub(super) struct Tasks {
    api: gpui::WeakEntity<glint_api::Workbench>,
    agent: gpui::WeakEntity<glint_agent::AgentWorkspace>,
    owner: gpui::WeakEntity<Workbench>,
    open: bool,
    _subscriptions: Vec<Subscription>,
}
impl Tasks {
    pub(super) fn new(
        api: Entity<glint_api::Workbench>,
        agent: Entity<glint_agent::AgentWorkspace>,
        owner: gpui::WeakEntity<Workbench>,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscriptions = vec![
            cx.observe(&api, |_, _, cx| cx.notify()),
            cx.observe(&agent, |_, _, cx| cx.notify()),
        ];
        Self {
            api: api.downgrade(),
            agent: agent.downgrade(),
            owner,
            open: false,
            _subscriptions: subscriptions,
        }
    }
}
impl Render for Tasks {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let api = self
            .api
            .upgrade()
            .expect("task menu outlived API workspace");
        let agent = self
            .agent
            .upgrade()
            .expect("task menu outlived Agent workspace");
        let mut tasks: Vec<BackgroundTask> = agent
            .read(cx)
            .background_tasks(cx)
            .into_iter()
            .map(|mut task| {
                task.id = format!("agent/{}", task.id);
                task
            })
            .collect();
        tasks.extend(
            api.read(cx)
                .background_tasks(cx)
                .into_iter()
                .map(|mut task| {
                    task.id = format!("api/{}", task.id);
                    task
                }),
        );
        let owner = self.owner.clone();
        let weak = cx.entity().downgrade();
        BackgroundTasksMenu::new(tasks)
            .theme(api.read(cx).host_theme())
            .open(self.open)
            .on_open_change(move |open, _, cx| {
                let _ = weak.update(cx, |this, cx| {
                    this.open = open;
                    cx.notify();
                });
            })
            .on_select(move |id, window, cx| {
                let _ = owner.update(cx, |this, cx| {
                    if let Some(id) = id.strip_prefix("api/") {
                        this.dispatch(
                            Action::Navigate(Route::Api(glint_api::Page::Requests)),
                            window,
                            cx,
                        );
                        this.api
                            .update(cx, |api, cx| api.focus_background_task(id, window, cx));
                        this.route = Route::Api(this.api.read(cx).page());
                    } else if let Some(id) = id.strip_prefix("agent/") {
                        if matches!(this.route, Route::Api(_)) {
                            this.api.update(cx, |api, cx| api.deactivate(window, cx));
                        }
                        this.agent
                            .update(cx, |agent, cx| agent.focus_background_task(id, window, cx));
                        this.route = Route::Agent(this.agent.read(cx).page());
                    }
                    this.sync_navigation(cx);
                    cx.notify();
                });
            })
            .render()
    }
}
