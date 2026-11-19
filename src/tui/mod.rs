//! The interactive front end of `jev-code`: an input box pinned under the conversation, a
//! menu for commands and files, approval cards and a footer with the mode. The conversation
//! itself goes into the terminal's own scrollback, so scrolling, search and copy work as
//! they do everywhere else; only the live area under it is redrawn.
//!
//! [`app`] is the state and what keys do, [`view`] how it looks, [`screen`] the terminal
//! writes, and [`run`] the loop that ties them to the session.

pub mod app;
pub mod editor;
pub mod history;
pub mod menu;
pub mod view;
pub mod run;
pub mod screen;
pub mod transcript;
