//! Splits a line into tokens with byte ranges. Words, runs of digits, and
//! single punctuation characters are separate tokens; `glued` records
//! whether a token followed the previous one with no whitespace, which is
//! what distinguishes `#family` from `# family` and `5pm` from `5 pm`.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Word,
    Number,
    Punct(char),
}

#[derive(Clone, Debug)]
pub struct Token<'a> {
    pub text: &'a str,
    pub lower: String,
    pub start: usize,
    pub end: usize,
    pub kind: Kind,
    pub glued: bool,
}

impl Token<'_> {
    /// The token as a number, if it is a run of at most nine digits.
    pub fn number(&self) -> Option<u32> {
        if self.kind != Kind::Number || self.text.len() > 9 {
            return None;
        }
        self.text.parse().ok()
    }

    pub fn is_word(&self, word: &str) -> bool {
        self.kind == Kind::Word && self.lower == word
    }

    pub fn is_any(&self, words: &[&str]) -> bool {
        self.kind == Kind::Word && words.iter().any(|w| *w == self.lower)
    }

    pub fn is_punct(&self, c: char) -> bool {
        self.kind == Kind::Punct(c)
    }
}

fn is_word_char(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

pub fn tokenize(input: &str) -> Vec<Token<'_>> {
    let mut tokens = Vec::new();
    let mut chars = input.char_indices().peekable();
    let mut glued = false;
    while let Some((start, c)) = chars.next() {
        if c.is_whitespace() {
            glued = false;
            continue;
        }
        let mut end = start + c.len_utf8();
        let kind = if c.is_ascii_digit() {
            while let Some((i, d)) = chars.peek().copied() {
                if d.is_ascii_digit() {
                    end = i + d.len_utf8();
                    chars.next();
                } else {
                    break;
                }
            }
            Kind::Number
        } else if is_word_char(c) {
            while let Some((i, d)) = chars.peek().copied() {
                let apostrophe = (d == '\'' || d == '\u{2019}')
                    && input[i + d.len_utf8()..]
                        .chars()
                        .next()
                        .is_some_and(is_word_char);
                if is_word_char(d) || apostrophe {
                    end = i + d.len_utf8();
                    chars.next();
                } else {
                    break;
                }
            }
            Kind::Word
        } else {
            Kind::Punct(c)
        };
        let text = &input[start..end];
        tokens.push(Token {
            text,
            lower: text.to_lowercase(),
            start,
            end,
            kind,
            glued,
        });
        glued = true;
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_words_numbers_and_punctuation() {
        let t = tokenize("call mom 5pm #family !high 17:00 don't");
        let shape: Vec<(&str, Kind, bool)> = t.iter().map(|t| (t.text, t.kind, t.glued)).collect();
        assert_eq!(
            shape,
            vec![
                ("call", Kind::Word, false),
                ("mom", Kind::Word, false),
                ("5", Kind::Number, false),
                ("pm", Kind::Word, true),
                ("#", Kind::Punct('#'), false),
                ("family", Kind::Word, true),
                ("!", Kind::Punct('!'), false),
                ("high", Kind::Word, true),
                ("17", Kind::Number, false),
                (":", Kind::Punct(':'), true),
                ("00", Kind::Number, true),
                ("don't", Kind::Word, false),
            ]
        );
        assert_eq!(t[2].number(), Some(5));
        assert_eq!((t[2].start, t[2].end), (9, 10));
        assert_eq!((t[3].start, t[3].end), (10, 12));
    }

    #[test]
    fn handles_unicode_and_empty_input() {
        assert!(tokenize("").is_empty());
        assert!(tokenize("   \t\n").is_empty());
        let t = tokenize("çağır annemi yarın 5'te");
        assert_eq!(t.len(), 6);
        assert_eq!(t[0].text, "çağır");
        assert_eq!(t[3].text, "5");
        assert_eq!(t[4].text, "'");
        assert_eq!(t[5].text, "te");
        assert!(t[5].glued);
        let t = tokenize("😀 x");
        assert_eq!(t[0].kind, Kind::Punct('😀'));
        assert_eq!(t[0].end, 4);
    }
}
