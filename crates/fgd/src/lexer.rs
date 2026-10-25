#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    At,
    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,
    Eq,
    Colon,
    Comma,
    Plus,
    Str(String),
    Word(String),
}

#[derive(Clone, Debug)]
pub struct Token {
    pub tok: Tok,
    pub start: usize,
    pub end: usize,
    pub line: u32,
    pub col: u32,
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | '\\' | '$' | '*' | '#' | '!' | '<' | '>' | '|' | '&' | '?' | '%' | '\'')
}

/// Splits FGD source into tokens; an unterminated string runs to the end of the file.
pub fn lex(src: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let mut it = src.char_indices().peekable();
    let (mut line, mut line_start) = (1u32, 0usize);
    while let Some((i, c)) = it.next() {
        let col = (src.get(line_start..i).map_or(0, |s| s.chars().count()) + 1) as u32;
        let single = match c {
            '\n' => {
                line += 1;
                line_start = i + 1;
                continue;
            }
            c if c.is_whitespace() => continue,
            '/' if it.peek().is_some_and(|(_, n)| *n == '/') => {
                while it.peek().is_some_and(|(_, n)| *n != '\n') {
                    it.next();
                }
                continue;
            }
            '@' => Some(Tok::At),
            '(' => Some(Tok::LParen),
            ')' => Some(Tok::RParen),
            '[' => Some(Tok::LBracket),
            ']' => Some(Tok::RBracket),
            '{' => Some(Tok::LBrace),
            '}' => Some(Tok::RBrace),
            '=' => Some(Tok::Eq),
            ':' => Some(Tok::Colon),
            ',' => Some(Tok::Comma),
            '+' => Some(Tok::Plus),
            _ => None,
        };
        if let Some(tok) = single {
            out.push(Token { tok, start: i, end: i + 1, line, col });
            continue;
        }
        if c == '"' {
            let mut end = src.len();
            let mut text = String::new();
            for (j, d) in it.by_ref() {
                if d == '"' {
                    end = j + 1;
                    break;
                }
                if d == '\n' {
                    line += 1;
                    line_start = j + 1;
                }
                text.push(d);
            }
            out.push(Token { tok: Tok::Str(text), start: i, end, line, col });
            continue;
        }
        let mut word = String::from(c);
        let mut end = i + c.len_utf8();
        while let Some(&(j, d)) = it.peek() {
            if !is_word(d) || (d == '/' && src.get(j + 1..).is_some_and(|s| s.starts_with('/'))) {
                break;
            }
            word.push(d);
            end = j + d.len_utf8();
            it.next();
        }
        out.push(Token { tok: Tok::Word(word), start: i, end, line, col });
    }
    out
}
