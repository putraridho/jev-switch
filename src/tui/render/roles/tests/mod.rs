//! Tests for the conversation roles, by role.

use super::edit::*;
use super::group::*;
use super::prose::*;
use super::result::*;
use super::sent::*;
use super::tool::*;
use crate::tui::render::route::RouteFacts;
use crate::tui::render::strip_ansi;
use crate::tui::render::test_facts::*;
use std::path::Path;

mod edit;
mod group;
mod prose;
mod result;
mod safety;
mod sent;
mod tool;
