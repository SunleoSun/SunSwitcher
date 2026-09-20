#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Boundary {
    Character(char),
    Enter,
    Tab,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletedToken {
    text: String,
    boundary: Boundary,
}

impl CompletedToken {
    pub fn new(text: impl Into<String>, boundary: Boundary) -> Self {
        Self {
            text: text.into(),
            boundary,
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn boundary(&self) -> Boundary {
        self.boundary
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputEvent {
    Character(char),
    Backspace,
    Boundary(Boundary),
    Invalidate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputOutcome {
    Continue,
    Completed(CompletedToken),
    Invalidated,
}

#[derive(Debug, Default)]
pub struct InputBuffer {
    token: String,
}

impl InputBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn process(&mut self, event: InputEvent) -> InputOutcome {
        match event {
            InputEvent::Character(character) if is_token_character(character) => {
                self.token.push(character);
                InputOutcome::Continue
            }
            InputEvent::Character(character) => self.complete(Boundary::Character(character)),
            InputEvent::Boundary(boundary) => self.complete(boundary),
            InputEvent::Backspace => {
                self.token.pop();
                InputOutcome::Continue
            }
            InputEvent::Invalidate => {
                self.token.clear();
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
        InputOutcome::Completed(CompletedToken::new(text, boundary))
    }
}

fn is_token_character(character: char) -> bool {
    character.is_alphanumeric() || matches!(character, '_' | '-' | '\'' | '’')
}
