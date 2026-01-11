#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    Word(String),

    Newline,
    Semicolon,

    Pipe,
    AndIf,
    OrIf,
    Background,

    RedirectIn,
    RedirectOut,
    RedirectAppend,
    RedirectInOut,
    Heredoc,

    SingleQuoted(String),
    DoubleQuoted(Vec<Token>),

    Variable(String),
    VariableBraced(String),

    LParen,
    RParen,
    LBrace,
    RBrace,

    Eof,
}


pub struct Lexer {
    input: Vec<char>,
    pos: usize,
}

impl Lexer {
    pub fn new(input: &str) -> Self {
        Self {
            input: input.chars().collect(),
            pos: 0,
        }
    }

    fn peek(&self) -> Option<char> {
        self.input.get(self.pos).copied()
    }

    fn next(&mut self) -> Option<char> {
        let ch = self.peek();
        if ch.is_some() {
            self.pos += 1;
        }
        ch
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(' ' | '\t')) {
            self.next();
        }
    }

    pub fn next_token(&mut self) -> Token {
        self.skip_whitespace();

        match self.next() {
            Some('\n') => Token::Newline,
            Some(';') => Token::Semicolon,

            Some('&') => {
                if self.peek() == Some('&') {
                    self.next();
                    Token::AndIf
                } else {
                    Token::Background
                }
            }

            Some('|') => {
                if self.peek() == Some('|') {
                    self.next();
                    Token::OrIf
                } else {
                    Token::Pipe
                }
            }

            Some('<') => match self.peek() {
                Some('<') => {
                    self.next();
                    Token::Heredoc
                }
                Some('>') => {
                    self.next();
                    Token::RedirectInOut
                }
                _ => Token::RedirectIn,
            },

            Some('>') => {
                if self.peek() == Some('>') {
                    self.next();
                    Token::RedirectAppend
                } else {
                    Token::RedirectOut
                }
            }

            Some('(') => Token::LParen,
            Some(')') => Token::RParen,
            Some('{') => Token::LBrace,
            Some('}') => Token::RBrace,

            Some('\'') => Token::SingleQuoted(self.read_single_quoted()),
            Some('"') => Token::DoubleQuoted(self.read_double_quoted()),

            Some('$') => self.read_variable(),

            Some(ch) => self.read_word(ch),

            None => Token::Eof,
        }
    }

    fn read_word(&mut self, first: char) -> Token {
        let mut buf = String::new();
        buf.push(first);

        while let Some(ch) = self.peek() {
            if ch.is_whitespace()
                || matches!(
                    ch,
                    '|' | '&' | ';' | '<' | '>' |
                    '(' | ')' | '{' | '}' |
                    '"' | '\'' | '$'
                )
            {
                break;
            }
            buf.push(self.next().unwrap());
        }

        Token::Word(buf)
    }

    fn read_single_quoted(&mut self) -> String {
        let mut buf = String::new();

        while let Some(ch) = self.next() {
            if ch == '\'' {
                break;
            }
            buf.push(ch);
        }

        buf
    }

    fn read_double_quoted(&mut self) -> Vec<Token> {
        let mut tokens = Vec::new();
        let mut buf = String::new();

        while let Some(ch) = self.next() {
            match ch {
                '"' => break,

                '$' => {
                    if !buf.is_empty() {
                        tokens.push(Token::Word(buf.clone()));
                        buf.clear();
                    }
                    tokens.push(self.read_variable());
                }

                '\\' => {
                    if let Some(escaped) = self.next() {
                        buf.push(escaped);
                    }
                }

                _ => buf.push(ch),
            }
        }

        if !buf.is_empty() {
            tokens.push(Token::Word(buf));
        }

        tokens
    }

    fn read_variable(&mut self) -> Token {
        if self.peek() == Some('{') {
            self.next();

            let mut name = String::new();
            while let Some(ch) = self.next() {
                if ch == '}' {
                    break;
                }
                name.push(ch);
            }

            Token::VariableBraced(name)
        } else {
            let mut name = String::new();

            while let Some(ch) = self.peek() {
                if ch.is_alphanumeric() || ch == '_' {
                    name.push(self.next().unwrap());
                } else {
                    break;
                }
            }

            Token::Variable(name)
        }
    }
}
