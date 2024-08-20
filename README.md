# Ephemerole

Ephemerole is a discord role-per-activity bot designed to incredibly strict standards of privacy, because Microsoft.
(Yes, you read that right.)

## Why?

I have
a... [very](https://github.com/randomairborne/hypersonic) [bad](https://github.com/randomairborne/minixpd) [habit](https://github.com/randomairborne/tinylevel)
of writing discord bots in the hopes that they would be considered for use in the Minecraft discord. None of them ever
actually have been used, but I hold out hope. This bot stores ALL of its leveling data in memory. This means that no
user data is ever persisted. This is better than our current solution because (redacted, contact me over discord for a
full explanation if you're a mod or AM in the discord).

## Ok, how do I use it?

Ephemerole is a single docker container, published as `ghcr.io/randomairborne/ephemerole:latest`. It supports arm64 and
x86. It takes just three environment variables:

- `DISCORD_TOKEN`: The Discord app API key from the [developer dashboard](https://discord.com/developers/applications)
- `DISCORD_GUILD`: The ID of the guild you wish to use the bot in
- `DISCORD_ROLE`: The ID of the role you wish to grant after `MESSAGE_REQUIREMENT` is met.

Once you've set these up, probably using Docker Compose, start up the bot, and voilá! Users should be granted the role
automatically.

Do note that you will need to ensure the bot actually has permissions to add the `DISCORD_ROLE` it has been informed
about. Its highest role must be located above this role, and it must have the `MANAGE_ROLES` permission.

## "Advanced" configuration

If you want to have a little more control, you can also change the message cooldown with the below environment
variables.

- `MESSAGE_REQUIREMENT`: Message count before the user is granted the role. (default 60)
- `MESSAGE_COOLDOWN`: Amount of time, in seconds, required between messages for them to be counted. (default 60)
