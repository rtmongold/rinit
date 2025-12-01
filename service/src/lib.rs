#![feature(iterator_try_collect)]
use serde::{
    Deserialize,
    Serialize,
};

pub mod config;
pub mod dirs;
pub mod graph;
pub mod service_state;
pub mod types;

/// Differentiate how the dependency graph is created
/// (user, root, or project)
#[derive(Serialize, Deserialize, Debug, Default, Clone, Copy, Eq, PartialEq)]
pub enum Mode {
    #[default]
    User,
    Root,
    Project,
}
