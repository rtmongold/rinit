use serde::{
    Deserialize,
    Serialize,
};

/// Store options for Longrun and Oneshot
#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct BundleOptions {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub contents: Vec<String>,
}
