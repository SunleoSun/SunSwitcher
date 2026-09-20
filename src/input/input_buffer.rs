#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PhysicalKey {
    Other,
    Grave,
    LeftBracket,
    RightBracket,
    Semicolon,
    Quote,
    Comma,
    Period,
}

impl PhysicalKey {
    pub const fn is_layout_ambiguous(self) -> bool {
        !matches!(self, Self::Other)
    }

    pub const fn from_layout_symbol(character: char) -> Self {
        match character {
            '`' | '~' => Self::Grave,
            '[' | '{' => Self::LeftBracket,
            ']' | '}' => Self::RightBracket,
            ';' | ':' => Self::Semicolon,
            '\'' | '"' => Self::Quote,
            ',' | '<' => Self::Comma,
            '.' | '>' => Self::Period,
            _ => Self::Other,
        }
    }

    pub fn is_shifted_symbol(self, character: char) -> bool {
        self.shifted_symbol() == Some(character)
    }

    pub const fn shifted_symbol(self) -> Option<char> {
        match self {
            Self::Grave => Some('~'),
            Self::LeftBracket => Some('{'),
            Self::RightBracket => Some('}'),
            Self::Semicolon => Some(':'),
            Self::Quote => Some('"'),
            Self::Comma => Some('<'),
            Self::Period => Some('>'),
            Self::Other => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TypedCharacter {
    produced: char,
    physical_key: PhysicalKey,
}

impl TypedCharacter {
    pub const fn new(produced: char, physical_key: PhysicalKey) -> Self {
        Self {
            produced,
            physical_key,
        }
    }

    pub const fn plain(produced: char) -> Self {
        Self::new(produced, PhysicalKey::Other)
    }

    pub const fn produced(self) -> char {
        self.produced
    }

    pub const fn physical_key(self) -> PhysicalKey {
        self.physical_key
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Boundary {
    Character(char),
    Enter,
    Tab,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletedToken {
    text: String,
    physical_keys: Vec<PhysicalKey>,
    boundary: Boundary,
}

impl CompletedToken {
    pub fn new(text: impl Into<String>, boundary: Boundary) -> Self {
        let text = text.into();
        let physical_keys = vec![PhysicalKey::Other; text.chars().count()];
        Self {
            text,
            physical_keys,
            boundary,
        }
    }

    fn from_typed_parts(text: String, physical_keys: Vec<PhysicalKey>, boundary: Boundary) -> Self {
        debug_assert_eq!(text.chars().count(), physical_keys.len());
        Self {
            text,
            physical_keys,
            boundary,
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn physical_keys(&self) -> &[PhysicalKey] {
        &self.physical_keys
    }

    pub fn boundary(&self) -> Boundary {
        self.boundary
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputEvent {
    Character(TypedCharacter),
    Backspace,
    Boundary(Boundary),
    Invalidate,
}

impl InputEvent {
    pub const fn character(character: char) -> Self {
        Self::Character(TypedCharacter::plain(character))
    }

    pub const fn typed_character(character: char, physical_key: PhysicalKey) -> Self {
        Self::Character(TypedCharacter::new(character, physical_key))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputOutcome {
    Continue,
    Completed(CompletedToken),
    Invalidated,
}

#[derive(Debug)]
pub struct InputBuffer {
    token: String,
    physical_keys: Vec<PhysicalKey>,
    synchronized: bool,
}

impl Default for InputBuffer {
    fn default() -> Self {
        Self {
            token: String::new(),
            physical_keys: Vec::new(),
            synchronized: true,
        }
    }
}

impl InputBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn process(&mut self, event: InputEvent) -> InputOutcome {
        if !self.synchronized {
            return match event {
                InputEvent::Character(character) if !is_token_character(character) => {
                    self.synchronized = true;
                    InputOutcome::Continue
                }
                InputEvent::Boundary(_) => {
                    self.synchronized = true;
                    InputOutcome::Continue
                }
                InputEvent::Invalidate => InputOutcome::Invalidated,
                InputEvent::Character(_) | InputEvent::Backspace => InputOutcome::Continue,
            };
        }

        match event {
            InputEvent::Character(character) if is_token_character(character) => {
                self.token.push(character.produced());
                self.physical_keys.push(character.physical_key());
                InputOutcome::Continue
            }
            InputEvent::Character(character) => {
                self.complete(Boundary::Character(character.produced()))
            }
            InputEvent::Boundary(boundary) => self.complete(boundary),
            InputEvent::Backspace => {
                self.token.pop();
                self.physical_keys.pop();
                InputOutcome::Continue
            }
            InputEvent::Invalidate => {
                self.token.clear();
                self.physical_keys.clear();
                self.synchronized = false;
                InputOutcome::Invalidated
            }
        }
    }

    pub fn current_token(&self) -> &str {
        &self.token
    }

    fn complete(&mut self, boundary: Boundary) -> InputOutcome {
        if self.token.is_empty() {
            return InputOutcome::Continue;
        }

        let text = std::mem::take(&mut self.token);
        let physical_keys = std::mem::take(&mut self.physical_keys);
        InputOutcome::Completed(CompletedToken::from_typed_parts(
            text,
            physical_keys,
            boundary,
        ))
    }
}

fn is_token_character(character: TypedCharacter) -> bool {
    let produced = character.produced();
    produced.is_alphanumeric()
        || matches!(produced, '_' | '-' | '\'' | '’')
        || character.physical_key().is_layout_ambiguous()
}
