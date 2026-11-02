//! Offline reusable component demo: cargo run --example git_graph
use gpui::{App, AppContext, Bounds, WindowBounds, WindowOptions, px, size};
use gpui_glint::components::{
    GitCommit, GitComparison, GitGraph, GitGraphData, GitGraphEvent, GlintAssets,
    bind_text_entry_keys,
};

fn data(target: &str) -> GitGraphData {
    let commit = |id: &str, parents: &[&str], summary: &str, refs: &str| GitCommit {
        id: id.into(),
        parents: parents.iter().map(|s| s.to_string()).collect(),
        summary: summary.into(),
        author: "Glint".into(),
        date: "2026-11-02".into(),
        references: refs.into(),
    };
    let commits = vec![
        commit(
            "aaaaaaaa",
            &["bbbbbbbb", "cccccccc"],
            "Merge feature",
            "HEAD -> feature",
        ),
        commit("bbbbbbbb", &["dddddddd"], "Polish branch picker", ""),
        commit(
            "cccccccc",
            &["dddddddd"],
            "Add GitGraph component",
            "master",
        ),
        commit("dddddddd", &[], "Initial commit", ""),
    ];
    GitGraphData {
        current_branch: "feature".into(),
        branches: vec!["master".into(), "feature".into()],
        compare_branch: target.into(),
        comparison: Some(GitComparison {
            current_count: if target == "master" { 2 } else { 0 },
            target_count: 0,
            current_only: if target == "master" {
                commits[..2].to_vec()
            } else {
                vec![]
            },
            target_only: vec![],
        }),
        commits,
        ..Default::default()
    }
}
fn main() {
    gpui_platform::application().with_assets(GlintAssets).run(|cx: &mut App| {
        bind_text_entry_keys(cx);
        cx.open_window(WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(480.), px(760.)), cx))),
            ..Default::default()
        }, |_, cx| {
            let graph = cx.new(|cx| {
                let mut graph = GitGraph::new(cx); graph.set_data(data("master"), cx); graph
            });
            cx.subscribe(&graph, |graph, event: &GitGraphEvent, cx| {
                graph.update(cx, |graph, cx| match event {
                    GitGraphEvent::CompareBranch(branch) => graph.set_data(data(branch), cx),
                    GitGraphEvent::SelectCommit(id) => graph.set_details(id, format!("commit {id}\nAuthor: Glint\n\nOffline fixture: the host supplies commit details."), cx),
                    GitGraphEvent::Refresh => graph.set_data(data("master"), cx),
                    GitGraphEvent::CloseRequested => cx.quit(),
                    GitGraphEvent::OpenFile { .. } => {},
                });
            }).detach();
            graph
        }).unwrap();
        cx.activate(true);
    });
}
