mod input_buffer;

pub use input_buffer::{Boundary, CompletedToken, InputBuffer, InputEvent, InputOutcome};

#[cfg(test)]
mod input_buffer_certification;
#[cfg(test)]
mod input_buffer_tests;
