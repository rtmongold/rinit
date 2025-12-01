mod dependency_graph;
mod node;

use std::{
    fmt::Display,
    str::FromStr,
};

pub use dependency_graph::{
    DependencyGraph,
    DependencyGraphError,
};
pub use node::Node;
use serde::{
    Deserialize,
    Serialize,
};
use snafu::Snafu;

#[derive(Serialize, Deserialize, Default, Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum Target {
    #[default]
    Default,
    Boot,
    Graphical,
}

#[derive(Debug, Snafu)]
#[snafu(display(""))]
pub struct TargetParseError {
    runlevel: String,
}

impl FromStr for Target {
    type Err = TargetParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "boot" => Ok(Target::Boot),
            "default" => Ok(Target::Default),
            "graphical" => Ok(Target::Graphical),
            _ => {
                TargetParseSnafu {
                    runlevel: s.to_string(),
                }
                .fail()
            }
        }
    }
}

impl Display for Target {
    fn fmt(
        &self,
        f: &mut std::fmt::Formatter<'_>,
    ) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Target::Boot => "boot",
                Target::Default => "default",
                Target::Graphical => "graphical",
            }
        )
    }
}
