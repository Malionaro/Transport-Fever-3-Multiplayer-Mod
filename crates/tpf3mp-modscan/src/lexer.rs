//! Just enough of a Lua (and Teal) lexer to find what a script calls: names,
//! punctuation and string literals, each with its line, and no comments.
//! Teal's type annotations are names and punctuation like any other, so the
//! same lexer reads `.tl` files.

/// One token of a script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    /// A name or keyword.
    Name(String),
    /// A string literal's contents, escapes left as written.
    Str(String),
    /// A number, as written.
    Number,
    /// One punctuation character (`..` is two).
    Punct(char),
}

/// A token and the line it starts on, from 1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Located {
    pub token: Token,
    pub line: usize,
}

/// The tokens of `text`. Never fails: an unterminated string or comment
/// runs to the end of the text.
pub fn tokens(text: &str) -> Vec<Located> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut line = 1;
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        match c {
            b'\n' => {
                line += 1;
                i += 1;
            }
            b' ' | b'\t' | b'\r' | 0x0b | 0x0c => i += 1,
            b'-' if bytes.get(i + 1) == Some(&b'-') => {
                i += 2;
                if let Some(level) = long_bracket(bytes, i) {
                    let (end, lines) = skip_long(bytes, i, level);
                    line += lines;
                    i = end;
                } else {
                    while i < bytes.len() && bytes[i] != b'\n' {
                        i += 1;
                    }
                }
            }
            b'"' | b'\'' => {
                let start_line = line;
                let mut value = Vec::new();
                i += 1;
                while i < bytes.len() && bytes[i] != c {
                    if bytes[i] == b'\\' && i + 1 < bytes.len() {
                        if bytes[i + 1] == b'\n' {
                            line += 1;
                        }
                        value.push(bytes[i]);
                        value.push(bytes[i + 1]);
                        i += 2;
                        continue;
                    }
                    if bytes[i] == b'\n' {
                        // An unfinished string ends at its line.
                        break;
                    }
                    value.push(bytes[i]);
                    i += 1;
                }
                if i < bytes.len() && bytes[i] == c {
                    i += 1;
                }
                out.push(Located {
                    token: Token::Str(String::from_utf8_lossy(&value).into_owned()),
                    line: start_line,
                });
            }
            b'[' if long_bracket(bytes, i).is_some() => {
                let level = long_bracket(bytes, i).unwrap_or(0);
                let start_line = line;
                let open = i + level + 2;
                let (end, lines) = skip_long(bytes, i, level);
                let close = end.saturating_sub(level + 2).max(open);
                line += lines;
                out.push(Located {
                    token: Token::Str(String::from_utf8_lossy(&bytes[open..close]).into_owned()),
                    line: start_line,
                });
                i = end;
            }
            c if c == b'_' || c.is_ascii_alphabetic() => {
                let start = i;
                while i < bytes.len() && (bytes[i] == b'_' || bytes[i].is_ascii_alphanumeric()) {
                    i += 1;
                }
                out.push(Located {
                    token: Token::Name(text[start..i].to_owned()),
                    line,
                });
            }
            c if c.is_ascii_digit() => {
                while i < bytes.len()
                    && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'.' || bytes[i] == b'_')
                {
                    i += 1;
                }
                out.push(Located {
                    token: Token::Number,
                    line,
                });
            }
            c if c.is_ascii() => {
                out.push(Located {
                    token: Token::Punct(char::from(c)),
                    line,
                });
                i += 1;
            }
            _ => i += 1,
        }
    }
    out
}

/// The level of a long bracket (`[[`, `[=[`, ...) opening at `at`.
fn long_bracket(bytes: &[u8], at: usize) -> Option<usize> {
    if bytes.get(at) != Some(&b'[') {
        return None;
    }
    let mut level = 0;
    while bytes.get(at + 1 + level) == Some(&b'=') {
        level += 1;
    }
    (bytes.get(at + 1 + level) == Some(&b'[')).then_some(level)
}

/// Skips the long bracket of `level` opening at `at`: where it ends, and
/// how many newlines it held.
fn skip_long(bytes: &[u8], at: usize, level: usize) -> (usize, usize) {
    let mut i = at + level + 2;
    let mut lines = 0;
    while i < bytes.len() {
        if bytes[i] == b'\n' {
            lines += 1;
        }
        if bytes[i] == b']'
            && bytes[i + 1..].len() > level
            && bytes[i + 1..i + 1 + level].iter().all(|b| *b == b'=')
            && bytes.get(i + 1 + level) == Some(&b']')
        {
            return (i + level + 2, lines);
        }
        i += 1;
    }
    (bytes.len(), lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(text: &str) -> Vec<(String, usize)> {
        tokens(text)
            .into_iter()
            .filter_map(|t| match t.token {
                Token::Name(n) => Some((n, t.line)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn comments_and_strings_hide_the_names_in_them() {
        let text = "-- api.cmd.sendCommand\nlocal a = \"game.interface\" --[[ load(x)\n]] b\n--[==[\n]]\n]==] c 'x\\'y'";
        assert_eq!(
            names(text),
            [
                ("local".to_owned(), 2),
                ("a".to_owned(), 2),
                ("b".to_owned(), 3),
                ("c".to_owned(), 6)
            ]
        );
        let strings: Vec<String> = tokens(text)
            .into_iter()
            .filter_map(|t| match t.token {
                Token::Str(s) => Some(s),
                _ => None,
            })
            .collect();
        assert_eq!(strings, ["game.interface", "x\\'y"]);
    }

    #[test]
    fn long_strings_keep_their_text_and_count_their_lines() {
        let text = "x = [[one\ntwo]] y";
        let all = tokens(text);
        assert_eq!(all[2].token, Token::Str("one\ntwo".into()));
        assert_eq!(
            all[3],
            Located {
                token: Token::Name("y".into()),
                line: 2
            }
        );
    }

    #[test]
    fn an_unfinished_string_or_comment_ends_the_text_without_panicking() {
        assert_eq!(tokens("x = 'abc").len(), 3);
        assert_eq!(tokens("--[[ never closed").len(), 0);
        assert_eq!(tokens("[==[ never closed").len(), 1);
    }
}
