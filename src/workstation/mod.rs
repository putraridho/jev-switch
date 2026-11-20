//! The workstation extras around the input box and the transcript: fuzzy file finding, vim
//! input mode, user keybindings, folded tool output and the
//! accessibility switches (reduced motion, a colourless theme). Each is a pure piece the
//! interface plugs in.

pub mod access;
pub mod collapse;
pub mod fuzzy;
pub mod keymap;
pub mod vim;
