# DisWidgets v2 Bot

## MongoDB schema

In order to seperate the bot from the site, the bot employs its own schemas and exposes its own API. The MongoDB schemas are unstable and are subject to change.

- ``bot__server_info`` -> Basic info about the server
- ``bot__server_user`` -> User precense info
- ``bot__server_channel`` -> Channel info

All collections are prefixed by ``bot__`` to avoid conflicts with website-managed collections.