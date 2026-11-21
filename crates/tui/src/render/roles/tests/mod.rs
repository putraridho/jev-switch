//! Tests for the conversation roles, by role.

use super::edit::*;
use super::group::*;
use super::prose::*;
use super::result::*;
use super::sent::*;
use super::tool::*;
use crate::render::route::RouteFacts;
use crate::render::strip_ansi;
use crate::render::test_facts::*;
use std::path::Path;

mod edit;
mod group;
mod prose;
mod result;
mod safety;
mod sent;
mod tool;
