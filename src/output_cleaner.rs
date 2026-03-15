/// Clean raw tmux capture-pane output from opencode/claude/codex TUI.
///
/// Removes box-drawing characters, ANSI escape sequences, status bar lines,
/// right-panel fragments, spinner characters, and collapses blank lines.
pub fn clean_output(raw: &str) -> String {
    let lines: Vec<&str> = raw.lines().collect();
    let mut cleaned: Vec<String> = Vec::with_capacity(lines.len());
    let mut blank_run = 0usize;
    let mut in_thinking = false;

    for line in lines {
        // 1. Strip ANSI escape sequences
        let line = strip_ansi(line);

        // 2. Strip box-drawing and geometric shape characters
        let line = strip_box_chars(&line);

        // 3. Remove right-panel content (large whitespace gap + short trailing text)
        let line = strip_right_panel(&line);

        // 4. Trim leading/trailing whitespace
        let line = line.trim().to_string();

        // 5. Drop known status bar / UI chrome lines
        if is_ui_chrome(&line) {
            continue;
        }

        // 6. Handle thinking blocks
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

        // 7. Collapse consecutive blank lines
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

/// Remove Unicode box-drawing (U+2500–U+257F), block elements (U+2580–U+259F),
/// geometric shapes (U+25A0–U+25FF), Braille patterns used as spinners (U+2800–U+28FF),
/// and a few other common TUI decoration code points.
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
            // Geometric Shapes
            if (0x25A0..=0x25FF).contains(&cp) {
                return false;
            }
            // Braille Patterns (used as spinners: ⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏)
            if (0x2800..=0x28FF).contains(&cp) {
                return false;
            }
            // Miscellaneous Symbols (includes ▼ ▶ etc. used as panel headers)
            if (0x2600..=0x26FF).contains(&cp) {
                return false;
            }
            // Dingbats
            if (0x2700..=0x27BF).contains(&cp) {
                return false;
            }
            // Supplemental Arrows / Math operators sometimes appear
            // Keep everything else
            true
        })
        .collect()
}

/// If a line has a run of 4+ consecutive spaces followed by short text at the
/// end, strip everything from that gap onwards. This removes right-panel
/// fragments (Context, LSP, Todo, Modified Files) that the TUI inlines
/// at the ends of lines after a wide whitespace column.
fn strip_right_panel(s: &str) -> String {
    // Find the first run of 4+ spaces
    let chars: Vec<char> = s.chars().collect();
    let len = chars.len();
    let mut i = 0;
    while i < len {
        if chars[i] == ' ' {
            // Count the run
            let start = i;
            while i < len && chars[i] == ' ' {
                i += 1;
            }
            let run_len = i - start;
            if run_len >= 4 {
                // Everything before the gap is the main content
                let main: String = chars[..start].iter().collect();
                return main;
            }
        } else {
            i += 1;
        }
    }
    s.to_string()
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
        // U+2503 BOX DRAWINGS LIGHT VERTICAL
        let input = "┃ some content ┃";
        assert_eq!(clean_output(input), "some content");
    }

    #[test]
    fn strips_spinner_chars() {
        let input = "⠋⠙⠹";
        // Should become empty (only spinner chars → is_ui_chrome after strip_box_chars)
        // Actually braille is stripped in strip_box_chars, so the line becomes empty
        assert_eq!(clean_output(input), "");
    }

    #[test]
    fn collapses_blank_lines() {
        let input = "line1\n\n\n\nline2";
        assert_eq!(clean_output(input), "line1\n\nline2");
    }

    #[test]
    fn strips_right_panel() {
        let input = "main content    Context";
        assert_eq!(clean_output(input), "main content");
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
