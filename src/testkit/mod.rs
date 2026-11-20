//! Test-only helpers: `fixture!` loads a recorded provider output, `golden!` loads an expected-bytes file,
//! `scratch_dir` makes a fresh temp directory, `git_repo` runs git in one and makes a committed repository.

#[macro_use]
pub mod fixture;
#[macro_use]
pub mod golden;
pub mod capture;
pub mod fake_jev;
pub mod git_repo;
pub mod scratch_dir;
