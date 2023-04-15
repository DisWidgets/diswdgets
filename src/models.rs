use poise::serenity_prelude::ChannelType;
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize, Debug)]
pub struct Server {   
    /// Server ID
    pub id: String,
    pub name: String,
    pub icon: String,
    pub member_count: u64
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Channels {
    pub id: String,
    pub guild_id: String,
    pub name: String,
    pub channel_type: ChannelType,
    pub category_name: String,
    pub category_id: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct User {
    pub id: String,
    pub guild_id: String,
    pub name: String,
    pub discriminator: String,
    pub avatar: String,
    pub status: String,
}
