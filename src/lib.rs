#![warn(clippy::all, clippy::pedantic, clippy::nursery)]

use std::collections::hash_map::Entry;

use ahash::AHashMap;
use twilight_model::{
    gateway::payload::incoming::MessageCreate,
    id::{
        marker::{RoleMarker, UserMarker},
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
#[derive(Clone, Copy)]
pub struct AssignConfig {
    pub role: Id<RoleMarker>,
    pub message_cooldown: u64,
    pub message_requirement: u64,
}

/// This is a type alias. It is a map of user ID to user data
pub type MessageMap = AHashMap<Id<UserMarker>, UserData>;

// Convert a discord message ID to a seconds value of when it was sent relative to the discord epoch
const fn snowflake_to_timestamp<T>(id: Id<T>) -> u64 {
    (id.get() >> 22) / 1000
}

/// Determine if the sender of a message should get a role, and track their progress
pub fn should_assign_role(
    message_create: &MessageCreate,
    config: AssignConfig,
    message_map: &mut MessageMap,
) -> bool {
    // If we know the user's roles, and we know they contain the role we'd assign
    // ignore them
    if message_create
        .member
        .as_ref()
        .is_some_and(|v| v.roles.contains(&config.role))
    {
        return false;
    }

    // When was the message created
    let message_sent_at = snowflake_to_timestamp(message_create.id);

    // This looks at the current state the user is in, if it exists. If it doesn't have a state
    // for that user, it adds one. Otherwise, we look and see if they're on cooldown and if they'd
    // sent enough messages. Had they sent enough messages, we return `true`, which
    // sets the return value of this function to true, as it is the last expression in the function,
    // and it does not have a semicolon at the end.
    match message_map.entry(message_create.author.id) {
        Entry::Occupied(entry) => {
            // We only do stuff to users if there has been at least message_cooldown seconds since their last message.
            // Saturating means that if the value is too small (which it can't really be in this code), just make it as big as possible.
            if message_sent_at.saturating_sub(entry.get().last_message_at)
                >= config.message_cooldown
            {
                // Have they sent enough messages? Find out today!
                if entry.get().messages >= config.message_requirement {
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
