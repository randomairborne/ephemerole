#![warn(clippy::all, clippy::pedantic, clippy::nursery)]

use std::time::Instant;

use ephemerole::{AssignConfig, MessageMap};
use twilight_model::{
    channel::{message::MessageType, Message},
    gateway::payload::incoming::MessageCreate,
    id::Id,
    user::User,
    util::Timestamp,
};

fn main() {
    let message_count = 1_000_000_000;
    let started = Instant::now();
    let config = AssignConfig {
        role: Id::new(1),
        message_cooldown: 60,
        message_requirement: 60,
    };
    let mut messages = MessageMap::new();
    for (seq, i) in (1..100_000).cycle().take(message_count).enumerate() {
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
            timestamp: Timestamp::from_secs(seq.try_into().unwrap()).unwrap(),
            thread: None,
            tts: false,
            webhook_id: None,
        };
        let msg = MessageCreate(msg);
        std::hint::black_box(ephemerole::should_assign_role(&msg, config, &mut messages));
    }
    let elapsed = started.elapsed();
    println!(
        "Took {} seconds to process 1,000,000,000 messages from 100,000 users ({} ns/iter)",
        elapsed.as_secs_f64(),
        elapsed.as_nanos() / message_count as u128
    );
}
