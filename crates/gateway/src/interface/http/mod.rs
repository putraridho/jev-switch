//! The HTTP front of the gateway: the proxy, the orchestrate checks and the request guard.

mod check_handlers;
mod choose_model;
pub mod compose;
mod guard;
mod proxy_handler;
mod upstream;

#[cfg(test)]
mod auxiliary_tests;
#[cfg(test)]
mod golden_rig;
#[cfg(test)]
mod golden_tests;
