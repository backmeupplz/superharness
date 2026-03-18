/// Clean raw tmux capture-pane output from opencode/claude/codex TUI.
///
/// Removes box-drawing characters, ANSI escape sequences, status bar lines,
/// spinner characters, and collapses blank lines.
pub fn clean_output(raw: &str) -> String {
    let lines: Vec<&str> = raw.lines().collect();
    let mut cleaned: Vec<String> = Vec::with_capacity(lines.len());
    let mut blank_run = 0usize;
    let mut in_thinking = false;

    for line in lines {
        // 1. Strip ANSI escape sequences
        let line = strip_ansi(line);

        // 2. Strip box-drawing and block element characters only
        let line = strip_box_chars(&line);

        // 3. Trim trailing whitespace only — preserve leading spaces so that
        // indented code output is not destroyed. A little TUI padding surviving
        // on the left is acceptable; losing indentation is not.
        let line = line.trim_end().to_string();

        // 4. Drop known status bar / UI chrome lines
        if is_ui_chrome(&line) {
            continue;
        }

        // 5. Handle thinking blocks
        if line.starts_with("Thinking:") || line == "Thinking" {
            if !in_thinking {
                in_thinking = true;
                // Reset blank run so [thinking...] marker isn't suppressed
                blank_run = 0;
                cleaned.push("[thinking...]".to_string());
            }
            // Skip subsequent thinking lines — they're already collapsed
            continue;
        } else {
            in_thinking = false;
        }

        // 6. Collapse consecutive blank lines
        if line.is_empty() {
            blank_run += 1;
            if blank_run <= 1 {
                cleaned.push(String::new());
            }
        } else {
            blank_run = 0;
            cleaned.push(line);
        }
    }

    // Trim leading/trailing blank lines from the whole output
    let result = cleaned.join("\n");
    result.trim().to_string()
}

// ── helpers ──────────────────────────────────────────────────────────────────

/// Remove ANSI CSI escape sequences: ESC [ ... letter
fn strip_ansi(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b {
            // ESC — look ahead for '['
            if i + 1 < bytes.len() && bytes[i + 1] == b'[' {
                i += 2;
                // Skip parameter bytes (0x30–0x3F) and intermediate bytes (0x20–0x2F)
                while i < bytes.len()
                    && ((bytes[i] >= 0x20 && bytes[i] <= 0x2F)
                        || (bytes[i] >= 0x30 && bytes[i] <= 0x3F))
                {
                    i += 1;
                }
                // Skip final byte (0x40–0x7E)
                if i < bytes.len() && bytes[i] >= 0x40 && bytes[i] <= 0x7E {
                    i += 1;
                }
            } else if i + 1 < bytes.len() && bytes[i + 1] == b']' {
                // OSC sequence: ESC ] ... ST (BEL or ESC \)
                i += 2;
                while i < bytes.len() {
                    if bytes[i] == 0x07 {
                        i += 1;
                        break;
                    }
                    if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'\\' {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
            } else {
                // Other ESC sequences — skip ESC + one char
                i += 2.min(bytes.len() - i);
            }
        } else {
            // Gather a contiguous ASCII-safe or valid UTF-8 slice
            // Push char by char to avoid re-encoding issues
            let ch = s[i..].chars().next().unwrap_or('\0');
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

/// Remove only Unicode box-drawing (U+2500–U+257F) and block elements (U+2580–U+259F).
/// These are TUI frame characters like ┃ ━ ┏ ┓ ▀ ▄ █ that are never legitimate output.
///
/// NOT stripped (may appear in legitimate output):
/// - Geometric Shapes: U+25A0–U+25FF
/// - Braille Patterns: U+2800–U+28FF (spinners — needed by heartbeat.rs for busy detection)
/// - Miscellaneous Symbols: U+2600–U+26FF
/// - Dingbats: U+2700–U+27BF
fn strip_box_chars(s: &str) -> String {
    s.chars()
        .filter(|&c| {
            let cp = c as u32;
            // Box Drawing
            if (0x2500..=0x257F).contains(&cp) {
                return false;
            }
            // Block Elements
            if (0x2580..=0x259F).contains(&cp) {
                return false;
            }
            // Keep everything else (geometric shapes, braille, symbols, dingbats, etc.)
            true
        })
        .collect()
}

/// Return true for lines that are pure UI chrome and should be dropped entirely.
fn is_ui_chrome(line: &str) -> bool {
    if line.is_empty() {
        return false; // blank lines handled separately
    }

    // Lines that are purely spinner characters after stripping
    if line.chars().all(|c| "⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏|/-\\".contains(c)) {
        return true;
    }

    // opencode build-mode indicator: "Build  Claude Sonnet 4.6 Anthropic · high"
    if (line.contains("Anthropic") || line.contains("OpenAI") || line.contains("Fireworks"))
        && (line.starts_with("Build") || line.starts_with("Plan"))
    {
        return true;
    }

    // Bottom bar variants
    if line.contains("ctrl+t variants") || line.contains("tab agents") {
        return true;
    }
    if line.contains("ctrl+p commands") && line.contains("OpenCode") {
        return true;
    }
    if line == "esc interrupt" || line == "esc  interrupt" {
        return true;
    }

    // Right-panel section headers that bleed through as standalone lines
    let right_panel_headers = [
        "Context",
        "LSP",
        "tokens",
        "used",
        "spent",
        "Modified Files",
        "Todo",
    ];
    for hdr in &right_panel_headers {
        if line == *hdr {
            return true;
        }
    }

    false
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_stays_empty() {
        assert_eq!(clean_output(""), "");
    }

    #[test]
    fn plain_text_unchanged() {
        let input = "Hello, world!\nThis is a test.";
        assert_eq!(clean_output(input), input);
    }

    #[test]
    fn strips_ansi_codes() {
        let input = "\x1b[32mGreen text\x1b[0m";
        assert_eq!(clean_output(input), "Green text");
    }

    #[test]
    fn strips_box_drawing() {
        // U+2503 BOX DRAWINGS LIGHT VERTICAL — box chars removed, but a leading
        // space artifact from the removed ┃ is acceptable (TUI padding); trim in
        // the assertion to verify the content is present.
        let input = "┃ some content ┃";
        assert_eq!(clean_output(input).trim(), "some content");
    }

    #[test]
    fn strips_spinner_chars_pure_line_dropped() {
        // A line that is ONLY braille spinner chars is dropped by is_ui_chrome
        // (not by strip_box_chars — braille now passes through strip_box_chars)
        let input = "⠋⠙⠹";
        assert_eq!(clean_output(input), "");
    }

    #[test]
    fn braille_chars_pass_through() {
        // Braille chars embedded in real content should NOT be stripped
        let input = "Working ⠋ processing request";
        assert_eq!(clean_output(input), "Working ⠋ processing request");
    }

    #[test]
    fn geometric_shapes_pass_through() {
        // Geometric shapes (U+25A0–U+25FF) should NOT be stripped
        // ▶ (U+25B6), ● (U+25CF), ◆ (U+25C6) are common in real output
        let input = "▶ Running tests\n● passed\n◆ result";
        assert_eq!(clean_output(input), "▶ Running tests\n● passed\n◆ result");
    }

    #[test]
    fn indented_code_preserved() {
        // Lines with 4+ spaces (indented code) must NOT be truncated
        let input = "fn main() {\n    let x = 42;\n    println!(\"{}\", x);\n}";
        assert_eq!(clean_output(input), input);
    }

    #[test]
    fn collapses_blank_lines() {
        let input = "line1\n\n\n\nline2";
        assert_eq!(clean_output(input), "line1\n\nline2");
    }

    #[test]
    fn collapses_thinking_blocks() {
        let input = "before\nThinking: step 1\nThinking: step 2\nThinking: step 3\nafter";
        assert_eq!(clean_output(input), "before\n[thinking...]\nafter");
    }

    #[test]
    fn drops_status_bar() {
        let input = "Build  Claude Sonnet 4.6 Anthropic · high";
        assert_eq!(clean_output(input), "");
    }

    #[test]
    fn drops_bottom_bar() {
        let input = "ctrl+t variants  tab agents  ctrl+p commands    • OpenCode 1.2.15";
        assert_eq!(clean_output(input), "");
    }

    #[test]
    fn pure_box_line_removed() {
        let input = "┏━━━━━━━━━━━━━━━━━━━━━┓\nsome content\n┗━━━━━━━━━━━━━━━━━━━━━┛";
        assert_eq!(clean_output(input), "some content");
    }
}
