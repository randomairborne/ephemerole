use std::{
    collections::{hash_map::Entry, HashMap},
    str::FromStr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use tokio::task::JoinSet;
use twilight_gateway::{EventTypeFlags, Shard, StreamExt};
use twilight_http::{request::AuditLogReason, Client};
use twilight_model::{
    gateway::{event::Event, payload::incoming::MessageCreate, CloseFrame, Intents, ShardId},
    id::{
        marker::{GuildMarker, RoleMarker, UserMarker},
        Id,
    },
};

/// Keep our temporary information about specific users all in one place
struct UserData {
    /// How many messages did this user send
    messages: u64,
    /// Instant is opaque. It's only usable from the memory
    /// within this program.
    last_message: Instant,
}

/// This holds the configuration data for the bot, plus the client for telling
/// discord to do something.
#[derive(Clone)]
struct AppState {
    guild: Id<GuildMarker>,
    role: Id<RoleMarker>,
    client: Arc<Client>,
}

/// This is a type alias. It is a map of user ID to user data
type MessageMap = HashMap<Id<UserMarker>, UserData>;

// How long users must wait before getting more credit
const COOLDOWN: Duration = Duration::from_secs(60);
// How much credit users must get before getting the role
const MESSAGE_COUNT: u64 = 60;

// tokio::main starts the background task manager
#[tokio::main]
async fn main() {
    // Read in our discord bot token, the server we're working in (discord calls them guilds behind the scenes)
    // and the role we need to assign
    let token: String = parse_var("DISCORD_TOKEN");
    let guild: Id<GuildMarker> = parse_var("DISCORD_GUILD");
    let role: Id<RoleMarker> = parse_var("DISCORD_ROLE");

    // We only care about new server messages, only have one bot instance, and don't care about message content
    let mut shard = Shard::new(ShardId::ONE, token.clone(), Intents::GUILD_MESSAGES);

    // Create a new client for telling discord what to do (adding roles)
    let client = Arc::new(Client::new(token));

    // Do we need to shut down?
    let shutdown = Arc::new(AtomicBool::new(false));
    // Makes a copy of shutdown, so we can change it in the shutdown waiter
    let shutdown_setter = shutdown.clone();

    // Makes a messenger to the shard, so we can tell it to stop
    let shutdown_sender = shard.sender();

    // Start a background task
    tokio::spawn(async move {
        // Wait until we're told to stop
        shutdown_signal().await;
        // Set `shutdown` to true, ensuring that every time it is read in the future
        // it will be true. If the store ordering was not Release, or the load ordering
        // was not Acquire, this would be a lazy operation
        shutdown_setter.store(true, Ordering::Release);
        // Tell discord "hey, disconnect me"
        shutdown_sender.close(CloseFrame::NORMAL).ok();
    });

    // Create a map of users -> current message counts and last message sent time
    let mut message_map: MessageMap = HashMap::new();

    // Store the target server and role, plus the map of user messages, and the discord
    // notifier all together
    let state = AppState {
        guild,
        role,
        client,
    };

    // create a set of background tasks to handle new messages, so we don't
    // shut them down uncleanly
    let mut background_tasks = JoinSet::new();
    // We only care about new messages
    let event_types = EventTypeFlags::MESSAGE_CREATE;
    // while there are more messages, process them
    while let Some(msg) = shard.next_event(event_types).await {
        // Failing to receive one message is okay. Log it and go on to the next one.
        let msg = match msg {
            Ok(event) => event,
            Err(error) => {
                eprintln!("ERROR: Failed to receive event: {error:?}");
                continue;
            }
        };
        match msg {
            // Process a message
            Event::MessageCreate(mc) => {
                let target_id = mc.author.id;
                // If we should add the role, spawn a background task to add the role
                if should_assign_role(*mc, &state, &mut message_map) {
                    let client = state.client.clone();
                    background_tasks.spawn(add_role(client, state.guild, state.role, target_id));
                }
            }
            Event::GatewayClose(_) => {
                // The bot automatically reconnects to discord when
                // improperly disconnected, so we check if we meant to shut down
                // then exit the loop if we did
                if shutdown.load(Ordering::Acquire) {
                    break;
                }
            }
            // Ignore events that aren't new messages or shutdowns
            _ => {}
        }
    }
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
        eprintln!("ERROR: could not calculate user's message count: {error:?}")
    }
}

/// Determine if the sender of a message should get a role, and track their progress
fn should_assign_role(
    message_create: MessageCreate,
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

    // We got the message at this instant, so we assume that now is Close Enough to when
    // the message was sent.
    let message_sent_at = Instant::now();

    // This looks at the current state the user is in, if it exists. If it doesn't have a state
    // for that user, it adds one. Otherwise, we look and see if they're on cooldown and if they'd
    // sent enough messages. Had they sent enough messages, we return `true`, which
    // sets the return value of this function to true, as it is the last expression in the function,
    // and it does not have a semicolon at the end.
    match message_map.entry(user_id) {
        Entry::Occupied(entry) => {
            // We only do stuff to users if there has been at least COOLDOWN seconds since their last message.
            // Saturating means that if the value is too big (which it can't really be in this code), just make it as big as possible.
            if message_sent_at.saturating_duration_since(entry.get().last_message) > COOLDOWN {
                // Have they sent enough messages? Find out today!
                if entry.get().messages >= MESSAGE_COUNT {
                    // We don't need to know about this user anymore. Forget about them.
                    entry.remove();
                    // They've sent enough messages! let the code later know that we need
                    // to give them a role
                    true
                } else {
                    // Get a changeable version of their stored data
                    let entry = entry.into_mut();
                    // Set when the message was sent as the last message from this user
                    entry.last_message = message_sent_at;
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
                last_message: message_sent_at,
            });
            // The user has only sent one message; why would we give them a role?
            false
        }
    }
}

// The bot uses "environment variables" for configuration.
// This helps the bot pick one of them out and convert the value (which is always text) into a number.
fn parse_var<T: FromStr>(name: &str) -> T {
    std::env::var(name)
        .unwrap_or_else(|_| panic!("Expected {name} in the environment!"))
        .parse::<T>()
        .unwrap_or_else(|_| panic!("Could not parse {name} as {}!", std::any::type_name::<T>()))
}

/// Windows and Linux supported code to return from this function when this app is told to shut down.
async fn shutdown_signal() {
    // Unix is macOS and Linux. For complicated but silly reasons, this code is only used on macOS and Linux
    #[cfg(target_family = "unix")]
    {
        use tokio::signal::unix::{signal, SignalKind};
        // listen for the INTERRUPT signal
        let mut interrupt = signal(SignalKind::interrupt()).expect("Failed to listen to sigint");
        // Listen for the QUIT signal
        let mut quit = signal(SignalKind::quit()).expect("Failed to listen to sigquit");
        // Listen for the TERMINATE signal
        let mut terminate = signal(SignalKind::terminate()).expect("Failed to listen to sigterm");
        #[allow(clippy::redundant_pub_crate)] // This shuts off a warning that we can't avoid
        {
            // Wait until any of these signals is sent, then continue to exit the function
            tokio::select! {
                _ = interrupt.recv() => {},
                _ = quit.recv() => {},
                _ = terminate.recv() => {}
            }
        }
    }
    // If we aren't on macOS or Linux, defer to the Tokio project's shutdown listener
    #[cfg(not(target_family = "unix"))]
    tokio::signal::ctrl_c()
        .await
        .expect("Failed to listen to ctrl+c");
}
