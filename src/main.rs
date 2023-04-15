use std::{time::Duration, num::NonZeroU64};

use log::{info, error};
use poise::serenity_prelude::{FullEvent, GuildId, OnlineStatus};

use crate::cache::CacheHttpImpl;

use mongodb::{Client, options::ClientOptions, bson::{Document, doc, self}};

mod cache;
mod config;
mod help;
mod stats;
mod models;

type Error = Box<dyn std::error::Error + Send + Sync>;
type Context<'a> = poise::Context<'a, Data, Error>;

// User data, which is stored and accessible in all command invocations
pub struct Data {
    cache_http: cache::CacheHttpImpl,
    mongo: Client
}

#[poise::command(prefix_command)]
async fn register(ctx: Context<'_>) -> Result<(), Error> {
    poise::builtins::register_application_commands_buttons(ctx).await?;
    Ok(())
}

async fn on_error(error: poise::FrameworkError<'_, Data, Error>) {
    // This is our custom error handler
    // They are many errors that can occur, so we only handle the ones we want to customize
    // and forward the rest to the default handler
    match error {
        poise::FrameworkError::Setup { error, .. } => panic!("Failed to start bot: {:?}", error),
        poise::FrameworkError::Command { error, ctx } => {
            error!("Error in command `{}`: {:?}", ctx.command().name, error,);
            let err = ctx
                .say(format!(
                    "There was an error running this command: {}",
                    error
                ))
                .await;

            if let Err(e) = err {
                error!("SQLX Error: {}", e);
            }
        }
        poise::FrameworkError::CommandCheckFailed { error, ctx } => {
            error!(
                "[Possible] error in command `{}`: {:?}",
                ctx.command().name,
                error,
            );
            if let Some(error) = error {
                error!("Error in command `{}`: {:?}", ctx.command().name, error,);
                let err = ctx
                    .say(format!(
                        "**{}**",
                        error
                    ))
                    .await;

                if let Err(e) = err {
                    error!("Error while sending error message: {}", e);
                }
            }
        }
        error => {
            if let Err(e) = poise::builtins::on_error(error).await {
                error!("Error while handling error: {}", e);
            }
        }
    }
}

async fn event_listener(event: &FullEvent, user_data: &Data) -> Result<(), Error> {
    let db: mongodb::Database = user_data.mongo.database("diswidgets");
    let scol = db.collection::<Document>("servers");
    let ucol = db.collection::<Document>("server_user");
    let ccol = db.collection::<Document>("server_channel");

    match event {
        FullEvent::InteractionCreate {
            interaction,
            ctx: _,
        } => {
            info!("Interaction received: {:?}", interaction.id());
        },
        FullEvent::Ready {
            data_about_bot,
            ctx: _,
        } => {
            info!(
                "{} is ready!",
                data_about_bot.user.name
            );
        },
        FullEvent::PresenceUpdate { ctx, new_data } => {
            let guild_id = match new_data.guild_id {
                Some(guild_id) => guild_id,
                None => {
                    info!("Presence update without guild id: uid={}", new_data.user.id);
                    return Ok(())
                },
            };

            let user = new_data.user.to_user().ok_or_else(|| {
                error!("Presence update without user: uid={}", new_data.user.id);
                "Presence update without user"
            })?;

            // Try to find guild in either cache or http
            let (
                name,
                icon,
                member_count
            ) = {
                let g = guild_id.to_guild_cached(&ctx).ok_or_else(|| {
                error!("Presence update without guild: gid={}", guild_id);
                "Presence update without guild"
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
                    g.icon_url().unwrap_or("https://cdn.discordapp.com/embed/avatars/0.png".to_string()),
                    member_count
                )
            };


            let guild_doc = bson::to_bson(&models::Server {
                id: guild_id.to_string(),
                name,
                icon,
                member_count
            })?;

            // Check for server in mongo
            let guild_check = scol.find_one(doc! {"id": guild_id.to_string()}, None).await?;

            if guild_check.is_none() {
                info!("Server not found in mongo, creating new entry");
                let document = guild_doc.as_document().ok_or("Failed to convert to document")?;        
                scol.insert_one(document, None).await?;
            } else {
                info!("Server found in mongo, updating guild");
                scol.update_one(doc! {"id": guild_id.to_string()}, doc! {"$set": guild_doc}, None).await?;
            }

            let user_doc = bson::to_bson(&models::User {
                id: user.id.to_string(),
                guild_id: guild_id.to_string(),
                name: user.name.clone(),
                discriminator: format!("{:.04}", user.discriminator),
                avatar: user.avatar_url().unwrap_or("https://cdn.discordapp.com/embed/avatars/0.png".to_string()),
                status: match new_data.status {
                    OnlineStatus::Online => "online",
                    OnlineStatus::Idle => "idle",
                    OnlineStatus::DoNotDisturb => "dnd",
                    OnlineStatus::Offline => "offline",
                    OnlineStatus::Invisible => "invisible",
                    _ => "unknown"
                }.to_string()
            })?;

            // Check for user in mongo
            let user_check = ucol.find_one(doc! {"id": user.id.to_string()}, None).await?;

            if user_check.is_none() {
                info!("User not found in mongo, creating new entry");
                let document = user_doc.as_document().ok_or("Failed to convert to document")?;        
                ucol.insert_one(document, None).await?;
            } else {
                info!("User found in mongo, updating user");
                ucol.update_one(doc! {"id": user.id.to_string(), "guild_id": guild_id.to_string()}, doc! {"$set": user_doc}, None).await?;
            }
        },
        _ => {}
    }

    Ok(())
}

#[tokio::main]
async fn main() {
    const MAX_CONNECTIONS: u32 = 3; // max connections to the database, we don't need too many here

    std::env::set_var("RUST_LOG", "infernoplex=info");

    env_logger::init();

    info!("Proxy URL: {}", config::CONFIG.proxy_url);

    let http = serenity::all::HttpBuilder::new(&config::CONFIG.token)
        .proxy(config::CONFIG.proxy_url.clone())
        .ratelimiter_disabled(true)
        .build();

    let client_builder =
        serenity::all::ClientBuilder::new_with_http(
            http, 
            serenity::all::GatewayIntents::all() // TODO: Set intents properly
        );

    let framework = poise::Framework::new(
        poise::FrameworkOptions {
            initialize_owners: true,
            prefix_options: poise::PrefixFrameworkOptions {
                prefix: Some("sl!".into()),
                ..poise::PrefixFrameworkOptions::default()
            },
            listener: |event, _ctx, user_data| Box::pin(event_listener(event, user_data)),
            commands: vec![
                register(),
                help::help(),
                help::simplehelp(),
                stats::stats(),
            ],
            /// This code is run before every command
            pre_command: |ctx| {
                Box::pin(async move {
                    info!(
                        "Executing command {} for user {} ({})...",
                        ctx.command().qualified_name,
                        ctx.author().name,
                        ctx.author().id
                    );
                })
            },
            /// This code is run after every command returns Ok
            post_command: |ctx| {
                Box::pin(async move {
                    info!(
                        "Done executing command {} for user {} ({})...",
                        ctx.command().qualified_name,
                        ctx.author().name,
                        ctx.author().id
                    );
                })
            },
            on_error: |error| Box::pin(on_error(error)),
            ..Default::default()
        },
        move |ctx, _ready, _framework| {
            Box::pin(async move {
                let client_options = ClientOptions::parse(config::CONFIG.mongodb_url.clone()).await.expect("Error parsing MongoDB URL");

                Ok(Data {
                    cache_http: CacheHttpImpl {
                        cache: ctx.cache.clone(),
                        http: ctx.http.clone(),
                    },                    
                    mongo: Client::with_options(client_options).expect("Error creating MongoDB client")
                })
            })
        },
    );

    let mut client = client_builder
        .framework(framework)
        .await
        .expect("Error creating client");

    if let Err(why) = client.start().await {
        error!("Client error: {:?}", why);
    }
}