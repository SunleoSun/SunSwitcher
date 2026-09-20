mod input_buffer;

pub use input_buffer::{
    Boundary, CompletedToken, InputBuffer, InputEvent, InputOutcome, PhysicalKey, TypedCharacter,
};

#[cfg(test)]
mod input_buffer_certification;
#[cfg(test)]
mod input_buffer_tests;
