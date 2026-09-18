const ESC: char = '\u{1b}';

/// Strips ANSI escape sequences (CSI, OSC, and bare two-byte forms) from `line`.
/// Returns the input unchanged, without allocating, when it contains no escape byte.
pub(crate) fn strip_ansi_codes(line: String) -> String {
    if !line.contains(ESC) {
        return line;
    }

    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch != ESC {
            out.push(ch);
            continue;
        }
        match chars.peek() {
            Some('[') => {
                chars.next();
                for next in chars.by_ref() {
                    if ('@'..='~').contains(&next) {
                        break;
                    }
                }
            }
            Some(']') | Some('P') | Some('X') | Some('^') | Some('_') => {
                chars.next();
                loop {
                    match chars.next() {
                        None => break,
                        Some('\u{7}') => break,
                        Some(ESC) if chars.peek() == Some(&'\\') => {
                            chars.next();
                            break;
                        }
                        Some(_) => {}
                    }
                }
            }
            Some(_) => {
                chars.next();
            }
            None => {}
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::strip_ansi_codes;

    #[test]
    fn plain_line_passes_through_unchanged() {
        assert_eq!(strip_ansi_codes("hello world".to_string()), "hello world");
    }

    #[test]
    fn strips_sgr_color_codes() {
        let input = "\u{1b}[31;1mfnm: \u{1b}[0msome text";
        assert_eq!(strip_ansi_codes(input.to_string()), "fnm: some text");
    }

    #[test]
    fn strips_several_sequences() {
        let input = "\u{1b}[1m\u{1b}[32mok\u{1b}[0m \u{1b}[2mdim\u{1b}[0m";
        assert_eq!(strip_ansi_codes(input.to_string()), "ok dim");
    }

    #[test]
    fn strips_osc_sequence_terminated_by_bel() {
        let input = "\u{1b}]0;window title\u{7}rest";
        assert_eq!(strip_ansi_codes(input.to_string()), "rest");
    }

    #[test]
    fn strips_osc_sequence_terminated_by_esc_backslash() {
        let input = "\u{1b}]0;window title\u{1b}\\rest";
        assert_eq!(strip_ansi_codes(input.to_string()), "rest");
    }

    #[test]
    fn truncated_csi_at_end_of_line_does_not_panic() {
        let input = "prefix\u{1b}[31;1";
        assert_eq!(strip_ansi_codes(input.to_string()), "prefix");
    }

    #[test]
    fn empty_line_stays_empty() {
        assert_eq!(strip_ansi_codes(String::new()), "");
    }

    #[test]
    fn literal_bracket_survives() {
        assert_eq!(strip_ansi_codes("array[0] = 1".to_string()), "array[0] = 1");
    }

    #[test]
    fn strips_dcs_sequence_terminated_by_esc_backslash() {
        let input = "\u{1b}P1$rdata\u{1b}\\rest";
        assert_eq!(strip_ansi_codes(input.to_string()), "rest");
    }
}
