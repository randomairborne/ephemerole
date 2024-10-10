#![warn(clippy::all, clippy::pedantic, clippy::nursery)]
use std::{
    env::VarError,
    str::FromStr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use ephemerole::{AssignConfig, MessageMap};
use tokio::runtime::Builder as RuntimeBuilder;
use tokio_util::task::TaskTracker;
use twilight_gateway::{EventTypeFlags, Shard, StreamExt};
use twilight_http::{request::AuditLogReason, Client};
use twilight_model::{
    gateway::{event::Event, CloseFrame, Intents, ShardId},
    id::{
        marker::{GuildMarker, RoleMarker, UserMarker},
        Id,
    },
};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    // Read in our discord bot token, the server we're working in (discord calls them guilds behind the scenes)
    // and the role we need to assign
    let token: String = parse_var("DISCORD_TOKEN");
    let guild: Id<GuildMarker> = parse_var("DISCORD_GUILD");
    let role: Id<RoleMarker> = parse_var("DISCORD_ROLE");

    // These values are optional, and they both have default values of 60
    let message_requirement: u64 = get_var("MESSAGE_REQUIREMENT").unwrap_or(60);
    let message_cooldown: u64 = get_var("MESSAGE_COOLDOWN").unwrap_or(60);

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

    // Create a different runtime to do non-critical tasks on a different thread
    let sender_rt = RuntimeBuilder::new_current_thread()
        .enable_all()
        .thread_name("role_adder")
        .build()
        .unwrap();
    // Create a way to send new tasks to this runtime
    let sender_rt_handle = sender_rt.handle().clone();

    // start a background runtime to handle other I/O tasks, like adding roles
    std::thread::spawn(move || {
        // Provide something for the main thread of the background runtime to do
        // it needs something to do, or we can't spawn on it.
        sender_rt.block_on(async move {
            // Wait until we're told to stop
            shutdown_signal().await;
            // Set `shutdown` to true, ensuring that every time it is read in the future
            // it will be true. If the store ordering was not Release, or the load ordering
            // was not Acquire, this would be a lazy operation
            shutdown_setter.store(true, Ordering::Release);
            // Tell discord "hey, disconnect me"
            shutdown_sender.close(CloseFrame::NORMAL).ok();
        });
    });

    // Create a map of users -> current message counts and last message sent time
    // load_from_file tries to load from the save file if it exists.
    let mut message_map = MessageMap::new();

    // Store the target server and role, plus the map of user messages, and the discord
    // notifier all together
    let config = AssignConfig {
        role,
        message_cooldown,
        message_requirement,
    };

    // create a set of background tasks to handle new messages, so we don't
    // shut them down uncleanly
    let background_tasks = TaskTracker::new();
    // We only care about new messages
    let event_types = EventTypeFlags::MESSAGE_CREATE;

    // while there are more messages, process them
    while let Some(event) = shard.next_event(event_types).await {
        // Failing to receive one message is okay. Log it and go on to the next one.
        let event = match event {
            Ok(event) => event,
            Err(error) => {
                if shutdown.load(Ordering::Acquire) {
                    eprintln!("Got error {error} receiving event, but was shutting down anyway");
                    break;
                }
                eprintln!("ERROR: Failed to receive event: {error:?}");
                continue;
            }
        };

        if matches!(event, Event::GatewayClose(_)) {
            // The bot automatically reconnects to discord when
            // improperly disconnected, so we check if we meant to shut down
            // then exit the loop if we did
            if shutdown.load(Ordering::Acquire) {
                break;
            }
        }
        // If we should add the role, spawn a background task to add the role
        if let Event::MessageCreate(mc) = event {
            if ephemerole::should_assign_role(&mc, config, &mut message_map) {
                let client = client.clone();
                background_tasks.spawn_on(
                    add_role(client, guild, config.role, mc.author.id),
                    &sender_rt_handle,
                );
            }
        }
    }
    background_tasks.close();
    // Wait for all background tasks to complete
    background_tasks.wait().await;
    println!("Done, thank you!");
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

// This function wraps parse_var_res to give human-readable fatal errors
fn parse_var<T: FromStr>(name: &str) -> T {
    match parse_var_res(name) {
        Ok(v) => v,
        Err(ParseVarError::Parse(_)) => {
            panic!("Could not parse {name} as {}!", std::any::type_name::<T>())
        }
        Err(ParseVarError::Var(VarError::NotPresent)) => {
            panic!("Could not find {name} in environment!")
        }
        Err(ParseVarError::Var(VarError::NotUnicode(_))) => {
            panic!("{name} does not have a unicode value!")
        }
    }
}

// This function wraps parse_var_res to see if the value is invalid (and error if it is) or nonexistent (so we can default it)
fn get_var<T: FromStr>(name: &str) -> Option<T> {
    match parse_var_res(name) {
        Ok(v) => Some(v),
        Err(ParseVarError::Parse(_)) => {
            panic!("Could not parse {name} as {}!", std::any::type_name::<T>())
        }
        Err(ParseVarError::Var(VarError::NotPresent)) => None,
        Err(ParseVarError::Var(VarError::NotUnicode(_))) => {
            panic!("{name} does not have a unicode value!")
        }
    }
}

// The bot uses "environment variables" for configuration.
// This helps the bot pick one of them out and convert the value (which is always text) into a number.
fn parse_var_res<T: FromStr>(name: &str) -> Result<T, ParseVarError<T>> {
    std::env::var(name) // get the variable
        .map_err(ParseVarError::Var)? // if it doesn't exist, convert the error to a ParseVarError and bail out
        .parse() // Try to turn it into the type we want
        .map_err(ParseVarError::Parse) // If it can't be turned into that, wrap up the error and return it
}

/// The different types of errors we can get when we try to parse a variable
enum ParseVarError<T: FromStr> {
    Var(VarError),
    Parse(<T as FromStr>::Err),
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
