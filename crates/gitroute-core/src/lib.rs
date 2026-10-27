//! gitroute core: config model, rule resolver, GitHub API + device flow,
//! token store, managed gitconfig writer. No UI; all logic lives here.

pub mod accounts;
pub mod config;
pub mod doctor;
pub mod github;
pub mod helper_log;
pub mod managed;
pub mod paths;
pub mod repos;
pub mod resolve;
pub mod token_store;
