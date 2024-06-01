use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, Ord, PartialOrd, Eq, PartialEq)]
pub struct Flags(u64);