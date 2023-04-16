use log::{error, info};
use poise::serenity_prelude::FullEvent;

use crate::cache::CacheHttpImpl;

use mongodb::{
    bson::{doc, Document},
    options::ClientOptions,
    Client,
};

mod cache;
mod config;
mod gis;
mod help;
mod models;
mod stats;

type Error = Box<dyn std::error::Error + Send + Sync>;
type Context<'a> = poise::Context<'a, Data, Error>;

// User data, which is stored and accessible in all command invocations
pub struct Data {
    cache_http: cache::CacheHttpImpl,
    mongo: Client,
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
                let err = ctx.say(format!("**{}**", error)).await;

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
    let scol = db.collection::<Document>("bot__server_info");
    let ucol = db.collection::<Document>("bot__server_user");
    let ccol = db.collection::<Document>("bot__server_channel");

    match event {
        FullEvent::InteractionCreate {
            interaction,
            ctx: _,
        } => {
            info!("Interaction received: {:?}", interaction.id());
        }
        FullEvent::Ready {
            data_about_bot,
            ctx: _,
        } => {
            info!("{} is ready!", data_about_bot.user.name);
        }
        FullEvent::PresenceUpdate { ctx, new_data } => {
            let guild_id = match new_data.guild_id {
                Some(guild_id) => guild_id,
                None => {
                    info!("Presence update without guild id: uid={}", new_data.user.id);
                    return Ok(());
                }
            };

            let inserted = gis::add_or_update(
                &scol,
                doc! {"id": guild_id.to_string()},
                gis::guild(&user_data.cache_http, guild_id)?,
            )
            .await?;

            if inserted {
                info!(
                    "Inserted new guild, adding current precenses: {}",
                    guild_id.to_string()
                );
                // Get all precenses in guild
                let adds = {
                    let guild = guild_id
                        .to_guild_cached(&ctx)
                        .ok_or("Failed to get guild")?;

                    let mut adds = vec![];

                    for (_, precense) in guild.presences.iter() {
                        match gis::user_precense(guild_id, precense) {
                            Ok(bson) => adds.push(bson),
                            Err(e) => error!("Failed to create bson document for precense: {}", e),
                        }
                    }

                    adds
                };

                // Add all precenses to mongo
                for add in adds {
                    gis::add_or_update(
                        &ucol,
                        doc! {"id": &new_data.user.id.to_string(), "guild_id": &guild_id.to_string()},
                        add
                    ).await?;
                }
            } else {
                info!(
                    "Adding new precense: gid={}, uid={}",
                    guild_id.to_string(),
                    new_data.user.id.to_string()
                );
                gis::add_or_update(
                    &ucol,
                    doc! {"id": &new_data.user.id.to_string(), "guild_id":  &guild_id.to_string()},
                    gis::user_precense(guild_id, new_data)?,
                )
                .await?;
            }
        }
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

    let client_builder = serenity::all::ClientBuilder::new_with_http(
        http,
        serenity::all::GatewayIntents::all(), // TODO: Set intents properly
    );

    let framework = poise::Framework::new(
        poise::FrameworkOptions {
            initialize_owners: true,
            prefix_options: poise::PrefixFrameworkOptions {
                prefix: Some("sl!".into()),
                ..poise::PrefixFrameworkOptions::default()
            },
            listener: |event, _ctx, user_data| Box::pin(event_listener(event, user_data)),
            commands: vec![register(), help::help(), help::simplehelp(), stats::stats()],
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
                let client_options = ClientOptions::parse(config::CONFIG.mongodb_url.clone())
                    .await
                    .expect("Error parsing MongoDB URL");

                Ok(Data {
                    cache_http: CacheHttpImpl {
                        cache: ctx.cache.clone(),
                        http: ctx.http.clone(),
                    },
                    mongo: Client::with_options(client_options)
                        .expect("Error creating MongoDB client"),
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
