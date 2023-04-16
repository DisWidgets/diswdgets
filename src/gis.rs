use log::info;
use mongodb::{bson::{self, doc, Document, Bson}, Collection};
use poise::serenity_prelude::{Presence, GuildId, OnlineStatus};

use crate::Error;

pub fn add_user_precense(
    guild_id: GuildId,
    p: &Presence,
) -> Result<Bson, Error> {
    let user = p.user.to_user().ok_or("Failed to get user")?;

    let user_doc = bson::to_bson(&crate::models::User {
        id: user.id.to_string(),
        guild_id: guild_id.to_string(),
        name: user.name.clone(),
        discriminator: format!("{:.04}", user.discriminator),
        avatar: user.avatar_url().unwrap_or("https://cdn.discordapp.com/embed/avatars/0.png".to_string()),
        status: match p.status {
            OnlineStatus::Online => "online",
            OnlineStatus::Idle => "idle",
            OnlineStatus::DoNotDisturb => "dnd",
            OnlineStatus::Offline => "offline",
            OnlineStatus::Invisible => "invisible",
            _ => "unknown"
        }.to_string()
    })?;

    Ok(user_doc)
}   

pub async fn add_or_update(
    col: &Collection<Document>,
    id: &str,
    gid: &str,
    bson: Bson
) -> Result<(), Error> {
    // Check for user in mongo
    let user_check = col.find_one(doc! {"id": id}, None).await?;

    if user_check.is_none() {
        info!("User not found in mongo, creating new entry");
        let document = bson.as_document().ok_or("Failed to convert to document")?;        
        col.insert_one(document, None).await?;
    } else {
        info!("User found in mongo, updating user");
        col.update_one(doc! {"id": id, "guild_id": gid}, doc! {"$set": bson}, None).await?;
    }

    Ok(())
}