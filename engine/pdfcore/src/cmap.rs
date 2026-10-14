//! `ToUnicode` CMaps: what a font's character codes mean as text, and back.
//!
//! A subset font embedded in a PDF carries only the glyphs the document used,
//! and its codes are arbitrary (often just glyph numbers). The `ToUnicode`
//! stream is the document's own statement of what each code stands for — the
//! only reliable way to read the text, and, run in reverse, the only way to
//! write new text with the same font: find the code whose meaning is "é".

use std::collections::HashMap;

#[derive(Debug, Default, Clone)]
pub struct CMap {
    /// Bytes per code, from the codespace range. Zero when the map never said.
    pub code_bytes: usize,
    to_text: HashMap<u32, String>,
    to_code: HashMap<String, u32>,
}

impl CMap {
    pub fn parse(data: &[u8]) -> CMap {
        let mut cmap = CMap::default();
        let mut tokens = Tokens::new(data);
        while let Some(token) = tokens.next() {
            match token {
                Token::Word(w) if w == "begincodespacerange" => {
                    while let Some(Token::Hex(low)) = tokens.peek() {
                        cmap.code_bytes = cmap.code_bytes.max(low.len());
                        tokens.next();
                        tokens.next(); // the high end
                    }
                }
                Token::Word(w) if w == "beginbfchar" => {
                    while let Some(Token::Hex(code)) = tokens.peek() {
                        tokens.next();
                        if let Some(Token::Hex(dst)) = tokens.next() {
                            cmap.insert(be(&code), utf16(&dst));
                        }
                    }
                }
                Token::Word(w) if w == "beginbfrange" => {
                    while let Some(Token::Hex(low)) = tokens.peek() {
                        tokens.next();
                        let (Some(Token::Hex(high)), Some(target)) = (tokens.next(), tokens.next())
                        else {
                            break;
                        };
                        let (low, high) = (be(&low), be(&high));
                        match target {
                            Token::Hex(dst) => {
                                let base = utf16(&dst);
                                for (i, code) in (low..=high.min(low + 0xFFFF)).enumerate() {
                                    cmap.insert(code, bump(&base, i as u32));
                                }
                            }
                            Token::Open => {
                                let mut code = low;
                                while let Some(Token::Hex(dst)) = tokens.peek() {
                                    tokens.next();
                                    cmap.insert(code, utf16(&dst));
                                    code += 1;
                                }
                                tokens.next(); // the closing bracket
                            }
                            _ => break,
                        }
                    }
                }
                _ => {}
            }
        }
        cmap
    }

    fn insert(&mut self, code: u32, text: String) {
        // The first code to claim a text wins: a subset rarely has two, and
        // when it does, the lower is the one the original producer wrote.
        self.to_code.entry(text.clone()).or_insert(code);
        self.to_text.insert(code, text);
    }

    pub fn text_of(&self, code: u32) -> Option<&str> {
        self.to_text.get(&code).map(String::as_str)
    }

    pub fn code_of(&self, text: &str) -> Option<u32> {
        self.to_code.get(text).copied()
    }

    /// Every `(code, text)` the map defines.
    pub fn entries(&self) -> Vec<(u32, String)> {
        self.to_text.iter().map(|(c, t)| (*c, t.clone())).collect()
    }
}

/// Add `by` to the last UTF-16 unit of a range's starting text.
fn bump(base: &str, by: u32) -> String {
    let mut chars: Vec<char> = base.chars().collect();
    if let Some(last) = chars.last_mut() {
        if let Some(next) = char::from_u32(*last as u32 + by) {
            *last = next;
        }
    }
    chars.into_iter().collect()
}

fn be(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0u32, |n, b| (n << 8) | *b as u32)
}

fn utf16(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes.chunks(2).map(|c| be(c) as u16).collect();
    String::from_utf16_lossy(&units)
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Hex(Vec<u8>),
    Word(String),
    Open,
    Close,
}

struct Tokens<'a> {
    data: &'a [u8],
    at: usize,
    peeked: Option<Token>,
}

impl<'a> Tokens<'a> {
    fn new(data: &'a [u8]) -> Self {
        Tokens { data, at: 0, peeked: None }
    }

    fn peek(&mut self) -> Option<Token> {
        if self.peeked.is_none() {
            self.peeked = self.read();
        }
        self.peeked.clone()
    }

    fn next(&mut self) -> Option<Token> {
        self.peeked.take().or_else(|| self.read())
    }

    fn read(&mut self) -> Option<Token> {
        loop {
            let byte = *self.data.get(self.at)?;
            match byte {
                b if b.is_ascii_whitespace() => self.at += 1,
                b'%' => {
                    while self.data.get(self.at).is_some_and(|b| *b != b'\n' && *b != b'\r') {
                        self.at += 1;
                    }
                }
                b'<' if self.data.get(self.at + 1) == Some(&b'<') => self.at += 2,
                b'>' if self.data.get(self.at + 1) == Some(&b'>') => self.at += 2,
                b'<' => {
                    self.at += 1;
                    let mut digits = Vec::new();
                    while let Some(&b) = self.data.get(self.at) {
                        self.at += 1;
                        if b == b'>' {
                            break;
                        }
                        if b.is_ascii_hexdigit() {
                            digits.push(b);
                        }
                    }
                    if digits.len() % 2 == 1 {
                        digits.push(b'0');
                    }
                    let bytes = digits
                        .chunks(2)
                        .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
                        .collect();
                    return Some(Token::Hex(bytes));
                }
                b'[' => {
                    self.at += 1;
                    return Some(Token::Open);
                }
                b']' => {
                    self.at += 1;
                    return Some(Token::Close);
                }
                _ => {
                    let start = self.at;
                    while self.data.get(self.at).is_some_and(|b| {
                        !b.is_ascii_whitespace() && !matches!(b, b'<' | b'>' | b'[' | b']')
                    }) {
                        self.at += 1;
                    }
                    return Some(Token::Word(
                        String::from_utf8_lossy(&self.data[start..self.at]).into_owned(),
                    ));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_chars_and_ranges() {
        let cmap = CMap::parse(
            b"1 begincodespacerange <0000> <FFFF> endcodespacerange
              2 beginbfchar <0003> <0020> <0024> <0041> endbfchar
              2 beginbfrange <0030> <0032> <0061> <0040> <0041> [<00E9> <00F1>] endbfrange",
        );
        assert_eq!(cmap.code_bytes, 2);
        assert_eq!(cmap.text_of(0x03), Some(" "));
        assert_eq!(cmap.text_of(0x24), Some("A"));
        assert_eq!(cmap.text_of(0x31), Some("b"));
        assert_eq!(cmap.text_of(0x40), Some("é"));
        assert_eq!(cmap.text_of(0x41), Some("ñ"));
        assert_eq!(cmap.code_of("A"), Some(0x24));
        assert_eq!(cmap.code_of("z"), None);
    }
}
