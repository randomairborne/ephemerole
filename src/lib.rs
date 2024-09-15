#![warn(clippy::all, clippy::pedantic, clippy::nursery)]

use std::{
    collections::hash_map::Entry,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use ahash::AHashMap;
use tokio::runtime::Handle;
use tokio_util::task::TaskTracker;
use twilight_http::{request::AuditLogReason, Client};
use twilight_model::{
    gateway::{event::Event, payload::incoming::MessageCreate},
    id::{
        marker::{GuildMarker, RoleMarker, UserMarker},
        Id,
    },
};

/// Keep our temporary information about specific users all in one place
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct UserData {
    /// How many messages did this user send
    pub messages: u64,
    /// When was the last message at?
    pub last_message_at: u64,
}

/// This holds the configuration data for the bot, plus the client for telling
/// discord to do something.
#[derive(Clone)]
pub struct AppState {
    pub guild: Id<GuildMarker>,
    pub role: Id<RoleMarker>,
    pub client: Arc<Client>,
    pub message_cooldown: u64,
    pub message_requirement: u64,
}

/// This is a type alias. It is a map of user ID to user data
pub type MessageMap = AHashMap<Id<UserMarker>, UserData>;

pub fn handle_event(
    event: Event,
    state: &AppState,
    message_map: &mut MessageMap,
    background_tasks: &mut TaskTracker,
    sender_rt: &Handle,
    shutdown: &AtomicBool,
) -> bool {
    match event {
        // Process a message
        Event::MessageCreate(mc) => {
            let target_id = mc.author.id;
            // If we should add the role, spawn a background task to add the role
            if should_assign_role(&mc, state, message_map) {
                let client = state.client.clone();
                background_tasks.spawn_on(
                    add_role(client, state.guild, state.role, target_id),
                    sender_rt,
                );
            }
        }
        Event::GatewayClose(_) => {
            // The bot automatically reconnects to discord when
            // improperly disconnected, so we check if we meant to shut down
            // then exit the loop if we did
            if shutdown.load(Ordering::Acquire) {
                return true;
            }
        }
        // Ignore events that aren't new messages or shutdowns
        _ => {}
    }
    false
}

/// Add a role to a specific user, reporting the error in the console
async fn add_role(
    client: Arc<Client>,
    guild: Id<GuildMarker>,
    role: Id<RoleMarker>,
    target: Id<UserMarker>,
) {
    // Attempt to add the user's role, reporting the error if we can't
    if let Err(error) = client
        .add_guild_member_role(guild, target, role)
        .reason("User hit required message count")
        .await
    {
        eprintln!("ERROR: could not calculate user's message count: {error:?}");
    }
}

// Convert a discord message ID to a seconds value of when it was sent relative to the discord epoch
const fn snowflake_to_timestamp<T>(id: Id<T>) -> u64 {
    (id.get() >> 22) / 1000
}

/// Determine if the sender of a message should get a role, and track their progress
pub fn should_assign_role(
    message_create: &MessageCreate,
    state: &AppState,
    message_map: &mut MessageMap,
) -> bool {
    // If we know the user's roles, and we know they contain the role we'd assign
    // ignore them
    if message_create
        .member
        .as_ref()
        .is_some_and(|v| v.roles.contains(&state.role))
    {
        return false;
    }

    // give the user ID a shorter name
    let user_id = message_create.author.id;

    // When was the message created
    let message_sent_at = snowflake_to_timestamp(message_create.id);

    // This looks at the current state the user is in, if it exists. If it doesn't have a state
    // for that user, it adds one. Otherwise, we look and see if they're on cooldown and if they'd
    // sent enough messages. Had they sent enough messages, we return `true`, which
    // sets the return value of this function to true, as it is the last expression in the function,
    // and it does not have a semicolon at the end.
    match message_map.entry(user_id) {
        Entry::Occupied(entry) => {
            // We only do stuff to users if there has been at least message_cooldown seconds since their last message.
            // Saturating means that if the value is too small (which it can't really be in this code), just make it as big as possible.
            if message_sent_at.saturating_sub(entry.get().last_message_at) >= state.message_cooldown
            {
                // Have they sent enough messages? Find out today!
                if entry.get().messages >= state.message_requirement {
                    // We don't need to know about this user anymore. Forget about them.
                    entry.remove();
                    // They've sent enough messages! let the code later know that we need
                    // to give them a role
                    true
                } else {
                    // Get a changeable version of their stored data
                    let entry = entry.into_mut();
                    // Set when the message was sent as the last message from this user
                    entry.last_message_at = message_sent_at;
                    // Increase the number of messages this user has been known to send
                    entry.messages += 1;
                    // The user hasn't sent enough messages, don't give them a rule
                    false
                }
            } else {
                // The user is on cooldown, don't give them a role
                false
            }
        }
        // if we've never seen this user, add that they've sent one message as of right now
        Entry::Vacant(entry) => {
            entry.insert(UserData {
                messages: 1,
                last_message_at: message_sent_at,
            });
            // The user has only sent one message; why would we give them a role?
            false
        }
    }
}
