/// GIS is a module for handling the basic collection operations
///
/// It stands for "Guild Information Setup"
use log::{error, info};
use mongodb::{
    bson::{self, doc, Bson, Document},
    Collection,
};
use poise::serenity_prelude::{GuildId, OnlineStatus, Presence};

use crate::{cache::CacheHttpImpl, Error};

pub fn user_precense(guild_id: GuildId, p: &Presence) -> Result<Bson, Error> {
    let user = p.user.to_user().ok_or("Failed to get user")?;

    Ok(bson::to_bson(&crate::models::User {
        id: user.id.to_string(),
        guild_id: guild_id.to_string(),
        name: user.name.clone(),
        discriminator: format!("{:.04}", user.discriminator),
        avatar: user
            .avatar_url()
            .unwrap_or("https://cdn.discordapp.com/embed/avatars/0.png".to_string()),
        status: match p.status {
            OnlineStatus::Online => "online",
            OnlineStatus::Idle => "idle",
            OnlineStatus::DoNotDisturb => "dnd",
            OnlineStatus::Offline => "offline",
            OnlineStatus::Invisible => "invisible",
            _ => "unknown",
        }
        .to_string(),
    })?)
}

pub fn guild(cache_http: &CacheHttpImpl, guild_id: GuildId) -> Result<Bson, Error> {
    // Try to find guild in either cache or http
    let (name, icon, member_count) = {
        let g = guild_id.to_guild_cached(&cache_http.cache).ok_or_else(|| {
            error!("Guild not found in cache: gid={}", guild_id);
            "Guild not found in cache"
        })?;

        let member_count = {
            if g.member_count > 0 {
                g.member_count
            } else if !g.members.is_empty() {
                g.members.len() as u64
            } else {
                g.approximate_member_count.unwrap_or(0)
            }
        };

        (
            g.name.clone(),
            g.icon_url()
                .unwrap_or("https://cdn.discordapp.com/embed/avatars/0.png".to_string()),
            member_count,
        )
    };

    Ok(bson::to_bson(&crate::models::Server {
        id: guild_id.to_string(),
        name,
        icon,
        member_count,
    })?)
}

/// Helper method to either add or update a document in a collection
///
/// The bool returned is true if the document was added, false if it was updated
pub async fn add_or_update(
    col: &Collection<Document>,
    filter: Document,
    bson: Bson,
) -> Result<bool, Error> {
    // Check for user in mongo
    let check = col.find_one(filter.clone(), None).await?;

    if check.is_none() {
        info!(
            "Entity not found in mongo, creating new entry (col={})",
            col.name()
        );
        let document = bson.as_document().ok_or("Failed to convert to document")?;
        col.insert_one(document, None).await?;
        Ok(true)
    } else {
        info!(
            "Entity found in mongo, updating entity (col={})",
            col.name()
        );
        col.update_one(filter, doc! {"$set": bson}, None).await?;
        Ok(false)
    }
}
