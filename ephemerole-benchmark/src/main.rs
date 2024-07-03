use std::{
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};

use ephemerole::{AppState, MessageMap};
use tokio::{task::JoinSet, time::Instant};
use twilight_http::Client;
use twilight_model::{
    channel::{message::MessageType, Message},
    gateway::{event::Event, payload::incoming::MessageCreate},
    id::Id,
    user::User,
    util::Timestamp,
};

#[tokio::main]
async fn main() {
    let mut total = Duration::from_secs(0);
    let message_count = 1_000_000_000;
    let started = Instant::now();
    let state = AppState {
        guild: Id::new(1),
        role: Id::new(1),
        client: Arc::new(Client::builder().build()),
    };
    let mut tasks = JoinSet::new();
    let mut messages = MessageMap::new();
    for i in (1..100_000).cycle().take(message_count) {
        let author = User {
            accent_color: None,
            avatar: None,
            avatar_decoration: None,
            banner: None,
            bot: false,
            discriminator: 0,
            email: None,
            flags: None,
            global_name: None,
            id: Id::new(i),
            locale: None,
            mfa_enabled: None,
            name: String::new(),
            premium_type: None,
            public_flags: None,
            system: None,
            verified: None,
        };
        let msg = Message {
            activity: None,
            application: None,
            application_id: None,
            attachments: vec![],
            author,
            channel_id: Id::new(1),
            components: vec![],
            content: String::new(),
            edited_timestamp: None,
            embeds: vec![],
            flags: None,
            guild_id: None,
            id: Id::new(1),
            interaction: None,
            kind: MessageType::Regular,
            member: None,
            mention_channels: vec![],
            mention_everyone: false,
            mention_roles: vec![],
            mentions: vec![],
            pinned: false,
            reactions: vec![],
            reference: None,
            referenced_message: None,
            role_subscription_data: None,
            sticker_items: vec![],
            timestamp: Timestamp::from_secs(1).unwrap(),
            thread: None,
            tts: false,
            webhook_id: None,
        };
        // let serialized = serde_json::to_string(&msg).unwrap();
        let start = Instant::now();
        // let msg = serde_json::from_str(&serialized).unwrap();
        let msg = MessageCreate(msg);
        let event = Event::MessageCreate(Box::new(msg));
        ephemerole::handle_event(
            event,
            &state,
            &mut messages,
            &mut tasks,
            &AtomicBool::new(false),
        )
        .await;
        total += start.elapsed();
    }
    println!(
        "Took {} ({} including bench serializer) seconds to process 1,000,000,000 messages from 100,000 users (single thread)",
        total.as_secs_f64(), started.elapsed().as_secs_f64()
    );
    println!("{} ns/iter", total.as_nanos() / message_count as u128);
}
