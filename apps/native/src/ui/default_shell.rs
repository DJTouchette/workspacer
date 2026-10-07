//! Settings → Workspace → Default terminal shell. Agent terminals are started
//! by the hub (`terminals.create`), so the choice is the hub's shared
//! `terminal.shell` (desktop's Settings → Terminal edits the same key) and the
//! offered shells are the ones installed on the hub's host
//! (`terminals.shells`), not on this machine. Read and verified like the
//! other hub settings; the controller reads it fresh for each new shell.
use super::*;
use wks_native::{features::Request, terminal::ShellChoice};

impl Workspace {
    /// Read the hub's setting and its host's shells for Settings.
    pub(super) fn load_terminal_shell(&mut self, cx: &mut Context<Self>) {
        if self.demo
            || !self.view.connected
            || self
                .view
                .requests
                .get("terminal-shell")
                .is_some_and(|s| s.loading)
        {
            return;
        }
        self.request(Request::TerminalShell { set: None }, cx);
    }

    fn terminal_shell_writable(&self) -> bool {
        self.view.connected
            && !self.demo
            && self.extras.terminal_shell.is_some()
            && !self
                .view
                .requests
                .get("terminal-shell")
                .is_some_and(|s| s.loading)
    }

    pub(super) fn choose_terminal_shell(&mut self, path: String, cx: &mut Context<Self>) {
        let current = self
            .extras
            .terminal_shell
            .as_ref()
            .and_then(|v| v["shell"].as_str());
        if !self.terminal_shell_writable() || current == Some(path.as_str()) {
            return;
        }
        self.request(Request::TerminalShell { set: Some(path) }, cx);
    }

    fn terminal_shell_status(&self, choices: &[ShellChoice]) -> (String, chrome::Tone) {
        let state = self.view.requests.get("terminal-shell");
        let error = state.filter(|s| !s.loading).and_then(|s| {
            let writing = matches!(s.request, Request::TerminalShell { set: Some(_) });
            s.error.clone().map(|e| (e, writing))
        });
        let value = self.extras.terminal_shell.as_ref();
        match (value, error) {
            (_, Some((error, true))) => (
                format!("Couldn't change the hub's shell: {error}"),
                chrome::Tone::Error,
            ),
            (_, Some((error, false))) => (
                format!("Couldn't reach the hub's setting: {error}"),
                chrome::Tone::Error,
            ),
            (None, None) if self.view.connected && !self.demo => {
                ("Reading the hub's shells…".into(), chrome::Tone::Loading)
            }
            (None, None) => (
                "Connect to a hub to change this.".into(),
                chrome::Tone::Info,
            ),
            (Some(value), None) => {
                if let Some(error) = value["listError"].as_str() {
                    return (
                        format!(
                            "This hub can't list its shells ({error}); its default and the configured shell are shown."
                        ),
                        chrome::Tone::Warning,
                    );
                }
                let shell = value["shell"].as_str().unwrap_or_default();
                let label = choices
                    .iter()
                    .find(|c| c.path == shell)
                    .map(|c| c.label.as_str())
                    .unwrap_or("the hub's default");
                (
                    format!(
                        "New terminals start with {label}. Shells already open keep theirs until restarted."
                    ),
                    chrome::Tone::Info,
                )
            }
        }
    }

    pub(super) fn render_terminal_shell(&self, cx: &mut Context<Self>) -> Div {
        let p = self.appearance.palette();
        let writable = self.terminal_shell_writable();
        let value = self.extras.terminal_shell.as_ref();
        let choices = value
            .map(wks_native::terminal::shell_choices)
            .unwrap_or_default();
        let selected = value
            .and_then(|v| v["shell"].as_str())
            .unwrap_or_default()
            .to_owned();
        let code = self
            .fonts
            .resolved(&self.settings.code_font, "JetBrains Mono");
        let status = self.terminal_shell_status(&choices);
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .debug_selector(|| "terminal-shell-picker".into())
                    .flex()
                    .flex_col()
                    .gap_1()
                    .children(choices.into_iter().enumerate().map(|(ix, choice)| {
                        let active = choice.path == selected;
                        let path = choice.path.clone();
                        chrome::interactive_control(div().id(("terminal-shell", ix)), p, writable)
                            .debug_selector(move || format!("terminal-shell-{ix}"))
                            .px_3()
                            .py_2()
                            .rounded(px(p.control_radius))
                            .flex()
                            .items_center()
                            .gap_3()
                            .when(active, |d| d.bg(rgb(p.selected)))
                            .child(div().w(px(14.)).flex_shrink_0().when(active, |d| {
                                d.child(
                                    Icon::new(IconName::Check)
                                        .size(px(14.))
                                        .text_color(rgb(p.accent)),
                                )
                            }))
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(rgb(match (writable, active) {
                                        (false, false) => p.muted,
                                        _ => p.text,
                                    }))
                                    .child(choice.label),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .font_family(code.clone())
                                    .text_size(px(12.))
                                    .text_color(rgb(p.muted))
                                    .child(choice.detail),
                            )
                            .when(writable, |d| {
                                d.hover(move |s| s.bg(rgb(p.selected)))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.choose_terminal_shell(path.clone(), cx)
                                    }))
                            })
                    })),
            )
            .child(
                div()
                    .debug_selector(|| "terminal-shell-status".into())
                    .child(chrome::notice_line(
                        status.0,
                        status.1,
                        p,
                        "terminal-shell-status",
                    )),
            )
    }
}
