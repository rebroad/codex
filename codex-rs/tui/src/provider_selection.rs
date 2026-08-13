use crate::legacy_core::config::Config;
use crate::tui::Tui;
use crate::tui::TuiEvent;
use color_eyre::Result;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyEventKind;
use crossterm::event::KeyModifiers;
use ratatui::layout::Constraint;
use ratatui::layout::Layout;
use ratatui::prelude::Stylize;
use ratatui::style::Color;
use ratatui::text::Line;
use ratatui::widgets::Block;
use ratatui::widgets::Borders;
use ratatui::widgets::ListState;
use ratatui::widgets::Paragraph;
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
            TuiEvent::Key(key) => screen.handle_key(key),
            TuiEvent::Draw | TuiEvent::Resize(_) | TuiEvent::Resume => draw(tui, &screen)?,
            TuiEvent::Paste(_) => {}
        }
    }

    Ok(screen.selection())
}

fn draw(tui: &mut Tui, screen: &ProviderSelectionScreen) -> Result<()> {
    tui.draw(u16::MAX, |frame| {
        let [header, providers, footer] = Layout::vertical([
            Constraint::Length(4),
            Constraint::Min(1),
            Constraint::Length(2),
        ])
        .areas(frame.area());
        frame.render_widget_ref(
            &Paragraph::new(vec![
                Line::from("The saved session uses an unavailable model provider."),
                Line::from(format!(
                    "Missing provider: {}. Choose one available in the current configuration:",
                    screen.missing_provider
                )),
            ]),
            header,
        );
        let lines = screen
            .providers
            .iter()
            .enumerate()
            .map(|(index, provider)| {
                let line = Line::from(format!("{}. {provider}", index + 1));
                if screen.state.selected() == Some(index) {
                    line.bg(Color::Cyan).fg(Color::Black)
                } else {
                    line
                }
            })
            .collect::<Vec<_>>();
        frame.render_widget_ref(
            &Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title("Providers")),
            providers,
        );
        frame.render_widget_ref(
            &Paragraph::new("Up/Down or j/k to move • Enter to use • Esc to cancel"),
            footer,
        );
    })?;
    Ok(())
}

struct ProviderSelectionScreen {
    providers: Vec<String>,
    missing_provider: String,
    state: ListState,
    selected: Option<String>,
}

impl ProviderSelectionScreen {
    fn new(providers: Vec<String>, missing_provider: String) -> Self {
        let mut state = ListState::default();
        state.select(Some(0));
        Self {
            providers,
            missing_provider,
            state,
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
            KeyCode::Up | KeyCode::Char('k') => self.previous(),
            KeyCode::Down | KeyCode::Char('j') => self.next(),
            KeyCode::Enter => self.selected = self.current().cloned(),
            KeyCode::Esc => self.selected = Some(String::new()),
            KeyCode::Char(value) if value.is_ascii_digit() => {
                let index = value.to_digit(10).unwrap_or_default() as usize;
                if index > 0 && index <= self.providers.len() {
                    self.state.select(Some(index - 1));
                    self.selected = self.current().cloned();
                }
            }
            _ => {}
        }
    }

    fn previous(&mut self) {
        let index = self.state.selected().unwrap_or_default();
        self.state.select(Some(
            index.checked_sub(1).unwrap_or(self.providers.len() - 1),
        ));
    }

    fn next(&mut self) {
        let index = self.state.selected().unwrap_or_default();
        self.state.select(Some((index + 1) % self.providers.len()));
    }

    fn current(&self) -> Option<&String> {
        self.state
            .selected()
            .and_then(|index| self.providers.get(index))
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
    fn provider_selection_wraps_and_selects() {
        let mut screen = ProviderSelectionScreen::new(
            vec!["openai".to_string(), "amazon-bedrock".to_string()],
            "openai-custom".to_string(),
        );
        screen.handle_key(KeyEvent::from(KeyCode::Up));
        screen.handle_key(KeyEvent::from(KeyCode::Enter));
        assert_eq!(screen.selection(), Some("amazon-bedrock".to_string()));
    }
}
