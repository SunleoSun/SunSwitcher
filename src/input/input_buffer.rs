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
    layout_span: String,
    synchronized: bool,
}

impl Default for InputBuffer {
    fn default() -> Self {
        Self {
            token: String::new(),
            physical_keys: Vec::new(),
            layout_span: String::new(),
            synchronized: true,
        }
    }
}

impl InputBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn process(&mut self, event: InputEvent) -> InputOutcome {
        if matches!(&event, InputEvent::Invalidate) {
            self.token.clear();
            self.physical_keys.clear();
            self.layout_span.clear();
            self.synchronized = false;
            return InputOutcome::Invalidated;
        }

        if !self.synchronized {
            return match event {
                InputEvent::Character(character) => {
                    self.synchronized = true;
                    self.update_layout_span_for_character(character.produced());
                    if is_token_character(character) {
                        // Invalidation discards ownership of everything that existed before it, but a
                        // newly observed token character is text SunSwitcher can own from this point.
                        self.token.push(character.produced());
                        self.physical_keys.push(character.physical_key());
                    }
                    InputOutcome::Continue
                }
                InputEvent::Boundary(boundary) => {
                    self.synchronized = true;
                    self.update_layout_span_for_boundary(boundary);
                    InputOutcome::Continue
                }
                InputEvent::Backspace => {
                    self.layout_span.pop();
                    InputOutcome::Continue
                }
                InputEvent::Invalidate => {
                    unreachable!("invalidation is handled before ownership state")
                }
            };
        }

        match event {
            InputEvent::Character(character) if is_token_character(character) => {
                self.update_layout_span_for_character(character.produced());
                self.token.push(character.produced());
                self.physical_keys.push(character.physical_key());
                InputOutcome::Continue
            }
            InputEvent::Character(character) => {
                self.update_layout_span_for_character(character.produced());
                self.complete(Boundary::Character(character.produced()))
            }
            InputEvent::Boundary(boundary) => {
                self.update_layout_span_for_boundary(boundary);
                self.complete(boundary)
            }
            InputEvent::Backspace => {
                self.token.pop();
                self.physical_keys.pop();
                self.layout_span.pop();
                InputOutcome::Continue
            }
            InputEvent::Invalidate => {
                unreachable!("invalidation is handled before ownership state")
            }
        }
    }

    pub fn current_token(&self) -> &str {
        &self.token
    }

    pub fn current_layout_span(&self) -> &str {
        &self.layout_span
    }

    pub fn replace_layout_switch_span(&mut self, text: &str) {
        self.token.clear();
        self.physical_keys.clear();
        self.layout_span.clear();
        self.layout_span.push_str(text);
        // The injected replacement is owned for another explicit layout switch, but it
        // has no physical-key history and therefore must not become a correction token.
        self.synchronized = false;
    }

    fn update_layout_span_for_character(&mut self, character: char) {
        if character.is_whitespace() {
            if !self.layout_span.is_empty() {
                self.layout_span.push(character);
            }
            return;
        }

        if self
            .layout_span
            .chars()
            .last()
            .is_some_and(char::is_whitespace)
        {
            self.layout_span.clear();
        }
        self.layout_span.push(character);
    }

    fn update_layout_span_for_boundary(&mut self, boundary: Boundary) {
        match boundary {
            Boundary::Character(character) => self.update_layout_span_for_character(character),
            Boundary::Enter | Boundary::Tab => self.layout_span.clear(),
        }
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
