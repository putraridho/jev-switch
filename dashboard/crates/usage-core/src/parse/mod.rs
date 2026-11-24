//! The allow-list parsers: one per wire format (DESIGN 6, 8.3). Each surfaces numbers,
//! closed enums and `Label`s only.

pub mod claude_auth;
pub mod claude_event;
pub mod claude_log;
pub mod codex;
pub mod deepseek;
