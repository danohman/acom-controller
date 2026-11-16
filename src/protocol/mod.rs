//! ACOM solid-state amplifier RS-232 protocol: framing, message decoding and
//! command construction. Pure logic with no I/O, fully unit-tested.

pub mod bands;
pub mod commands;
pub mod errors;
pub mod frame;
pub mod messages;
