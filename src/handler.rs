use crate::{
  types::state::State,
  commands::{ reply, speak, knowlage, imagine },
  modules::state,
  presence,
  options
};

use std::{
  error::Error,
  sync::Arc,
  future::Future
};

use twilight_gateway::Event;

use twilight_model::{
  channel::Message,
  gateway::payload::incoming::MessageCreate
};

use twilight_http::request::channel::reaction::RequestReactionType;

use once_cell::sync::OnceCell;

static BOT_STRING: OnceCell<&'static str> = OnceCell::new();

static HEART: &str = "❤️";

fn spawn(fut: impl Future<Output = anyhow::Result<()>> + Send + 'static) {
  tokio::spawn(async move {
    if let Err(why) = fut.await {
      tracing::debug!("handler error: {why:?}");
    }
  });
}

async fn help(msg: Message, state: State) -> anyhow::Result<()> {
  tracing::debug!(
    "help command in channel {} by {}",
    msg.channel_id,
    msg.author.name
  );
  state
    .http
    .create_message(msg.channel_id)
    .reply(msg.id)
    .content("try to chat with me")
    .await?;
  Ok(())
}

fn get_bot_string() -> &'static str {
  BOT_STRING.get_or_init(|| Box::leak(format!("<@{}>", options::CONFIG.bot).into_boxed_str()))
}

fn contains_mention(text: &str) -> Option<(String, bool)> {
  let bot_str = get_bot_string();
  if let Some(pos) = text.find(bot_str) {
    let clean_text = text.replace(bot_str, "")
                         .trim().to_string();
    Some((clean_text, pos == 0))
  } else {
    None
  }
}

async fn handle_message(
  msg: Box<MessageCreate>,
  state: &State
) -> Result<(), Box<dyn Error + Send + Sync>> {
  if msg.author.bot {
    return Ok(());
  }

  let channel_id = msg.channel_id.get();
  if options::CONFIG.restricted_channels.contains(&channel_id) {
    return Ok(());
  }

  match msg.content.split_whitespace().next() {
    Some("~help")     => spawn(help(msg.0, Arc::clone(state))),
    Some("~imagine")  => {
      let prompt = msg.content.strip_prefix("~imagine ")
        .map(String::from)
        .unwrap_or_default();
      if prompt.is_empty() {
        let state_clone = Arc::clone(state);
        spawn(async move {
          state_clone.http
            .create_message(msg.channel_id)
            .reply(msg.id)
            .content("Please provide a prompt for image generation! Usage: `~imagine <prompt>` or `~imagine 1024x768 <prompt>`")
            .await?;
          Ok(())
        });
      } else {
        spawn(imagine::imagine(msg.0, prompt, Arc::clone(state)));
      }
    }
    Some(_cmd)        => {
      let author_name = match msg.guild_id {
        Some(guild_id) => {
          match state.http.guild_member(guild_id, msg.author.id).await {
            Ok(member_response) => {
              member_response.model().await
                .map(|member| member.nick.unwrap_or_else(|| msg.author.name.clone()))
                .unwrap_or_else(|_| msg.author.name.clone())
            }
            Err(_) => msg.author.name.clone()
          }
        }
        None => msg.author.name.clone()
      };
      let msg_content = msg.content.clone();
      if let Some(rfs) = &msg.referenced_message {
        if rfs.author.id == options::CONFIG.bot {
          spawn(reply::reply(msg.0, msg_content, author_name, Arc::clone(state)))
        }
      } else if let Some((rtext, first)) = contains_mention(msg_content.as_str()) {
        if first {
          match rtext.as_str() {
            "help"     => spawn(help(msg.0, Arc::clone(state))),
            "knowlage" => spawn(knowlage::stats(msg.0, Arc::clone(state))),
            "refresh"  => spawn(knowlage::refresh(msg.0, Arc::clone(state))),
            _cmd       => spawn(reply::reply(msg.0, rtext, author_name, Arc::clone(state)))
          }
        } else {
          spawn(reply::reply(msg.0, rtext, author_name, Arc::clone(state)))
        }
      } else {
        let chance = rand::random::<f32>();
        if chance <= 0.02 && msg.author.id.get() != options::CONFIG.owner {
          state.http.create_reaction(
            msg.channel_id,
            msg.id,
            &RequestReactionType::Unicode { name: HEART }
          )
          .await?;
          spawn(speak::speak( msg.0
                            , msg_content
                            , author_name
                            , Arc::clone(state)) )
        } else {
          spawn(state::update_global_state( msg_content
                                          , author_name
                                          , Arc::clone(state)));
        }
      }
    },
    None => {}
  };

  Ok(())
}

pub async fn handle_event(
  event: Event,
  state: State,
) -> Result<(), Box<dyn Error + Send + Sync>> {
  match event {
    Event::GuildCreate(guild) => {
      if !state.allowed_guilds.contains(&guild.id()) {
        state.http.leave_guild(guild.id()).await?;
        tracing::info!("Left unallowed guild: {}", guild.id());
      }
      Ok(())
    },
    Event::MessageCreate(msg) => handle_message(msg, &state).await,
    Event::Ready(_) => {
      tracing::info!("Shard is ready");
      tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(30));
        loop {
          interval.tick().await;
          if let Err(e) = presence::update_bot_status(&state).await {
            tracing::warn!("Failed to update bot status: {}", e);
          }
        }
      });
      
      Ok(())
    }
    _ => Ok(())
  }
}
