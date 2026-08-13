use crate::legacy_core::config::Config;
use crate::tui::Tui;
use crate::tui::TuiEvent;
use codex_protocol::protocol::SessionMetaLine;
use codex_rollout::RolloutItem;
use color_eyre::Result;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyEventKind;
use crossterm::event::KeyModifiers;
use ratatui::layout::Constraint;
use ratatui::layout::Layout;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use std::path::Path;
use tokio_stream::StreamExt;

pub(crate) async fn select_model_provider(
    tui: &mut Tui,
    config: &Config,
    missing_provider: &str,
) -> Result<Option<String>> {
    let mut providers: Vec<_> = config.model_providers.keys().cloned().collect();
    providers.sort_unstable();
    if providers.is_empty() {
        return Ok(None);
    }

    let mut screen = ProviderSelectionScreen::new(providers, missing_provider.to_string());
    draw(tui, &screen)?;
    let events = tui.event_stream();
    tokio::pin!(events);

    while !screen.is_done() {
        let Some(event) = events.next().await else {
            break;
        };
        tui.screen_size_for_event(&event)?;
        match event {
            TuiEvent::Key(key) => {
                screen.handle_key(key);
                draw(tui, &screen)?;
            }
            TuiEvent::Draw | TuiEvent::Resize(_) | TuiEvent::Resume => draw(tui, &screen)?,
            TuiEvent::Paste(_) => {}
        }
    }

    Ok(screen.selection())
}

pub(crate) async fn persist_model_provider(
    rollout_path: Option<&Path>,
    provider: &str,
) -> Result<()> {
    let Some(rollout_path) = rollout_path else {
        color_eyre::eyre::bail!(
            "cannot remember the selected provider because the session has no local rollout path"
        );
    };
    let mut session_meta = codex_rollout::read_session_meta_line(rollout_path).await?;
    session_meta.meta.model_provider = Some(provider.to_string());
    codex_rollout::append_rollout_item_to_path(
        rollout_path,
        &RolloutItem::SessionMeta(SessionMetaLine {
            meta: session_meta.meta,
            git: session_meta.git,
        }),
    )
    .await?;
    Ok(())
}

fn draw(tui: &mut Tui, screen: &ProviderSelectionScreen) -> Result<()> {
    tui.draw(u16::MAX, |frame| {
        let [message, providers, footer] = Layout::vertical([
            Constraint::Length(4),
            Constraint::Min(1),
            Constraint::Length(2),
        ])
        .areas(frame.area());
        frame.render_widget_ref(
            &Paragraph::new(vec![
                Line::from("The saved session uses an unavailable model provider."),
                Line::from(format!(
                    "Missing provider: {}. Press the number of the provider to use:",
                    screen.missing_provider
                )),
            ]),
            message,
        );
        let provider_lines = screen
            .providers
            .iter()
            .enumerate()
            .map(|(index, provider)| Line::from(format!("{}: {provider}", index + 1)))
            .collect::<Vec<_>>();
        frame.render_widget_ref(&Paragraph::new(provider_lines), providers);
        frame.render_widget_ref(&Paragraph::new("Press Esc or Ctrl-C to cancel."), footer);
    })?;
    Ok(())
}

struct ProviderSelectionScreen {
    providers: Vec<String>,
    missing_provider: String,
    selected: Option<String>,
}

impl ProviderSelectionScreen {
    fn new(providers: Vec<String>, missing_provider: String) -> Self {
        Self {
            providers,
            missing_provider,
            selected: None,
        }
    }

    fn handle_key(&mut self, key: KeyEvent) {
        if key.kind == KeyEventKind::Release {
            return;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('d'))
        {
            self.selected = Some(String::new());
            return;
        }
        match key.code {
            KeyCode::Char(value) if value.is_ascii_digit() => {
                let index = value.to_digit(10).unwrap_or_default() as usize;
                if index > 0 {
                    self.selected = self.providers.get(index - 1).cloned();
                }
            }
            KeyCode::Esc => self.selected = Some(String::new()),
            _ => {}
        }
    }

    fn is_done(&self) -> bool {
        self.selected.is_some()
    }

    fn selection(&self) -> Option<String> {
        self.selected.clone().filter(|value| !value.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_selection_chooses_provider() {
        let mut screen = ProviderSelectionScreen::new(
            vec!["openai".to_string(), "amazon-bedrock".to_string()],
            "openai-custom".to_string(),
        );
        screen.handle_key(KeyEvent::from(KeyCode::Char('2')));
        assert_eq!(screen.selection(), Some("amazon-bedrock".to_string()));
    }

    #[test]
    fn invalid_numeric_selection_keeps_prompt_open() {
        let mut screen =
            ProviderSelectionScreen::new(vec!["openai".to_string()], "openai-custom".to_string());
        screen.handle_key(KeyEvent::from(KeyCode::Char('2')));
        assert!(!screen.is_done());
    }
}
