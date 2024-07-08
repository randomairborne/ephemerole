use std::{
    str::FromStr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use ephemerole::{AppState, MessageMap};
use tokio::task::JoinSet;
use twilight_gateway::{EventTypeFlags, Shard, StreamExt};
use twilight_http::Client;
use twilight_model::{
    gateway::{CloseFrame, Intents, ShardId},
    id::{
        marker::{GuildMarker, RoleMarker},
        Id,
    },
};

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
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
    let mut message_map = MessageMap::new();

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
    while let Some(event) = shard.next_event(event_types).await {
        // Failing to receive one message is okay. Log it and go on to the next one.
        let event = match event {
            Ok(event) => event,
            Err(error) => {
                eprintln!("ERROR: Failed to receive event: {error:?}");
                continue;
            }
        };
        // Calls the handle_event function in lib.rs
        if ephemerole::handle_event(
            event,
            &state,
            &mut message_map,
            &mut background_tasks,
            shutdown.as_ref(),
        )
        .await
        {
            break;
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
