use serde::{Deserialize, Serialize};

/// A Discord ID
#[derive(Serialize, Deserialize, Clone, Debug, Ord, PartialOrd, Eq, PartialEq)]
pub struct Snowflake(String);

impl Snowflake {
    pub fn new(id: String) -> Self {
        Self(id)
    }
}