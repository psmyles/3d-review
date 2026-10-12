//! The glob patterns naming rules are written in: `*` (any run of characters),
//! `?` (any one character) and `[...]` classes (`[abc]`, `[a-z]`, `[!0-9]`).
//! Matching is case-sensitive, because engine naming conventions are.

/// A parsed glob pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Glob {
    tokens: Vec<Token>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Literal(char),
    AnyOne,
    AnyRun,
    Class {
        negated: bool,
        ranges: Vec<(char, char)>,
    },
}

/// Why a pattern could not be parsed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GlobError {
    #[error("unclosed '[' in pattern \"{0}\"")]
    UnclosedClass(String),
    #[error("empty pattern")]
    Empty,
}

impl Glob {
    pub fn new(pattern: &str) -> Result<Self, GlobError> {
        if pattern.is_empty() {
            return Err(GlobError::Empty);
        }
        let mut tokens = Vec::new();
        let mut chars = pattern.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '*' => {
                    if tokens.last() != Some(&Token::AnyRun) {
                        tokens.push(Token::AnyRun);
                    }
                }
                '?' => tokens.push(Token::AnyOne),
                '[' => {
                    let negated = chars.next_if(|&c| c == '!' || c == '^').is_some();
                    let mut ranges = Vec::new();
                    let mut closed = false;
                    // A `]` straight after the opening is a literal member.
                    let mut first = true;
                    while let Some(c) = chars.next() {
                        if c == ']' && !first {
                            closed = true;
                            break;
                        }
                        first = false;
                        if chars.peek() == Some(&'-') {
                            chars.next();
                            match chars.next() {
                                Some(']') => {
                                    ranges.push((c, c));
                                    ranges.push(('-', '-'));
                                    closed = true;
                                    break;
                                }
                                Some(end) => ranges.push((c.min(end), c.max(end))),
                                None => break,
                            }
                        } else {
                            ranges.push((c, c));
                        }
                    }
                    if !closed {
                        return Err(GlobError::UnclosedClass(pattern.to_owned()));
                    }
                    tokens.push(Token::Class { negated, ranges });
                }
                c => tokens.push(Token::Literal(c)),
            }
        }
        Ok(Self { tokens })
    }

    pub fn matches(&self, text: &str) -> bool {
        let chars: Vec<char> = text.chars().collect();
        // Iterative wildcard matching with one backtrack point per `*`, which
        // is linear-ish and never recursive.
        let (mut t, mut c) = (0_usize, 0_usize);
        let mut star: Option<(usize, usize)> = None;
        while c < chars.len() {
            match self.tokens.get(t) {
                Some(Token::AnyRun) => {
                    star = Some((t, c));
                    t += 1;
                    continue;
                }
                Some(token) if token_matches(token, chars[c]) => {
                    t += 1;
                    c += 1;
                    continue;
                }
                _ => {}
            }
            match star {
                Some((star_t, star_c)) => {
                    t = star_t + 1;
                    c = star_c + 1;
                    star = Some((star_t, star_c + 1));
                }
                None => return false,
            }
        }
        self.tokens[t..].iter().all(|token| *token == Token::AnyRun)
    }
}

fn token_matches(token: &Token, c: char) -> bool {
    match token {
        Token::Literal(literal) => *literal == c,
        Token::AnyOne => true,
        Token::AnyRun => false,
        Token::Class { negated, ranges } => {
            ranges.iter().any(|&(low, high)| low <= c && c <= high) != *negated
        }
    }
}

/// A list of patterns a name has to match one of.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GlobSet {
    globs: Vec<Glob>,
}

impl GlobSet {
    pub fn new(patterns: &[String]) -> Result<Self, GlobError> {
        Ok(Self {
            globs: patterns
                .iter()
                .map(|pattern| Glob::new(pattern))
                .collect::<Result<_, _>>()?,
        })
    }

    pub fn is_empty(&self) -> bool {
        self.globs.is_empty()
    }

    pub fn matches(&self, text: &str) -> bool {
        self.globs.iter().any(|glob| glob.matches(text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matches(pattern: &str, text: &str) -> bool {
        Glob::new(pattern).unwrap().matches(text)
    }

    #[test]
    fn stars_and_questions() {
        assert!(matches("SM_*", "SM_Crate"));
        assert!(matches("SM_*", "SM_"));
        assert!(!matches("SM_*", "sm_Crate"), "case-sensitive");
        assert!(matches("*_LOD?", "Wall_LOD2"));
        assert!(!matches("*_LOD?", "Wall_LOD10"));
        assert!(matches("*_LOD*", "Wall_LOD10"));
        assert!(matches("a*b*c", "a__b__b__c"));
        assert!(!matches("a*b*c", "a__b__b__"));
        assert!(matches("*", ""));
    }

    #[test]
    fn classes() {
        assert!(matches("UC[XP]_*", "UCX_Crate"));
        assert!(matches("UC[XP]_*", "UCP_Crate"));
        assert!(!matches("UC[XP]_*", "UCB_Crate"));
        assert!(matches("[A-Z]*", "Body"));
        assert!(!matches("[A-Z]*", "body"));
        assert!(matches("[!0-9]*", "Body"));
        assert!(!matches("[!0-9]*", "9Body"));
        assert!(matches("[]x]", "]"));
    }

    #[test]
    fn malformed_patterns_are_errors() {
        assert!(matches!(
            Glob::new("UC[X"),
            Err(GlobError::UnclosedClass(_))
        ));
        assert_eq!(Glob::new(""), Err(GlobError::Empty));
    }
}
