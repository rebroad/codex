use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

#[test]
fn screen_snapshot_keeps_visible_text_and_omits_cell_padding() {
    let mut buffer = Buffer::empty(Rect::new(0, 0, 8, 2));
    buffer.set_string(0, 0, "Codex", Style::default());
    buffer.set_string(0, 1, "你好", Style::default());

    assert_eq!(
        super::rows(&buffer),
        vec!["Codex".to_owned(), "你好".to_owned()]
    );
}

#[test]
fn printed_snapshot_leaves_the_cursor_after_the_frame_for_the_shell_prompt() {
    let mut output = Vec::new();
    super::print_to(&mut output, &["final TUI frame".to_owned()]).expect("print preserved frame");
    let output = String::from_utf8(output).expect("ANSI output is UTF-8");

    assert!(output.contains("final TUI frame"));
    assert!(output.ends_with("\r\n"));
}
