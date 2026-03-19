/// Pick a representative preview line from a user prompt.
///
/// The returned line skips common scaffolding headers that appear in copied IDE
/// context blocks and falls back to the first non-empty line.
pub fn prompt_preview_line(prompt: &str) -> String {
    let mut first_non_empty: Option<&str> = None;
    for line in prompt
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        if first_non_empty.is_none() {
            first_non_empty = Some(line);
        }
        if line.starts_with('#')
            || line.starts_with("```")
            || line.starts_with('-')
            || line.eq_ignore_ascii_case("Context from my IDE setup:")
            || line.eq_ignore_ascii_case("Active file:")
            || line.eq_ignore_ascii_case("Open tabs:")
        {
            continue;
        }
        return line.to_string();
    }
    first_non_empty.unwrap_or("<empty prompt>").to_string()
}
