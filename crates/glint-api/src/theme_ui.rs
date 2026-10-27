//! Theme switching and propagation to workbench controls.

use super::*;

impl Workbench {
    pub(super) fn toggle_theme(&mut self, cx: &mut Context<Self>) {
        self.set_theme(
            if self.theme.is_dark() {
                ThemePreset::Moonstone
            } else {
                ThemePreset::DarkObsidian
            },
            cx,
        );
    }

    pub(super) fn set_theme(&mut self, theme: ThemePreset, cx: &mut Context<Self>) {
        self.mcp_history_search
            .update(cx, |entry, cx| entry.set_theme(theme, cx));
        self.mcp_history_detail
            .update(cx, |editor, cx| editor.set_theme(theme, cx));
        self.ws_decoder_select
            .update(cx, |select, cx| select.set_theme(theme, cx));
        self.ws_format_select
            .update(cx, |select, cx| select.set_theme(theme, cx));
        self.ws_output
            .update(cx, |output, cx| output.set_theme(theme, cx));
        self.theme = theme;
        if self.directory_settings.is_some() {
            self.sync_reference_inputs(cx);
        }
        self.curl_input
            .update(cx, |editor, cx| editor.set_theme(theme, cx));
        self.toasts.update(cx, |host, cx| host.set_theme(theme, cx));
        self.sync_groups(cx);
        self.sync_pair_tables(cx);
        self.response_headers_table
            .update(cx, |table, cx| table.set_theme(theme, cx));
        for entry in self.response_header_entries.iter().flatten() {
            entry.update(cx, |entry, cx| entry.set_theme(theme, cx));
        }
        for entry in [
            &self.filter,
            &self.history_search,
            &self.response_search,
            &self.group_name,
            &self.request_name,
            &self.version_tag_input,
            &self.workspace_name_input,
            &self.workspace_transfer_name,
        ] {
            entry.update(cx, |e, cx| e.set_theme(theme, cx));
        }
        self.variable_search
            .update(cx, |entry, cx| entry.set_theme(theme, cx));
        for row in self.variable_rows.iter().flatten() {
            row.name.update(cx, |entry, cx| entry.set_theme(theme, cx));
            row.value.update(cx, |entry, cx| entry.set_theme(theme, cx));
            row.kind
                .update(cx, |select, cx| select.set_theme(theme, cx));
        }
        for input in &self.proxy_inputs {
            input
                .select
                .update(cx, |select, cx| select.set_theme(theme, cx));
        }
        for proxy in &self.profile_inputs {
            proxy
                .table
                .update(cx, |table, cx| table.set_theme(theme, cx));
            proxy.url.update(cx, |entry, cx| entry.set_theme(theme, cx));
            proxy
                .listener_port
                .update(cx, |entry, cx| entry.set_theme(theme, cx));
            proxy
                .search
                .update(cx, |entry, cx| entry.set_theme(theme, cx));
            for row in &proxy.mappings {
                row.name.update(cx, |entry, cx| entry.set_theme(theme, cx));
                row.value.update(cx, |entry, cx| entry.set_theme(theme, cx));
            }
        }
        for index in 0..self.profile_inputs.len() {
            self.sync_proxy_table(index, cx);
        }
        for name in &self.profile_names {
            name.update(cx, |entry, cx| entry.set_theme(theme, cx));
        }
        self.sync_variable_table(cx);
        for row in self
            .params
            .iter()
            .chain(self.headers.iter())
            .chain(self.cookies.iter())
            .chain(self.form.iter())
            .chain(self.extractors.iter())
        {
            for entry in [&row.key, &row.value, &row.note] {
                entry.update(cx, |e, cx| e.set_theme(theme, cx));
            }
        }
        for editor in [&self.body, &self.response, &self.request_preview] {
            editor.update(cx, |e, cx| e.set_theme(theme, cx));
        }
        self.endpoint.update(cx, |field, cx| {
            field.set_theme(theme, cx);
            field.set_options(
                [
                    "GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS", "WS",
                ]
                .map(|s| SelectOption::new(s, s).color(theme.color(method_color(s)))),
                cx,
            );
        });
        for field in self
            .oauth
            .fields
            .iter()
            .chain([&self.oauth.secret, &self.oauth.callback])
        {
            field.update(cx, |entry, cx| entry.set_theme(theme, cx));
        }
        self.authentication
            .update(cx, |field, cx| field.set_theme(theme, cx));
        self.environment.update(cx, |e, cx| e.set_theme(theme, cx));
        self.commands.update(cx, |e, cx| e.set_theme(theme, cx));
        cx.notify();
    }
}
