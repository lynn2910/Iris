use serde::{Deserialize, Serialize};
use crate::flags::Flags;
use crate::snowflake::Snowflake;

#[derive(Serialize, Deserialize, Debug, Clone, Ord, PartialOrd, PartialEq, Eq)]
pub struct User {
    id: Snowflake,
    username: String,
    discriminator: String,
    global_name: Option<String>,
    avatar: Option<String>,
    bot: Option<bool>,
    system: Option<bool>,
    mfa_enabled: Option<bool>,
    banner: Option<String>,
    accent_color: Option<i64>,
    locale: Option<String>,
    verified: Option<bool>,
    email: Option<String>,
    // TODO
    flags: Option<Flags>,
    // TODO
    premium_type: Option<i64>,
    // TODO
    public_flags: Option<Flags>,
    // TODO
    // avatar_decoration_data
}