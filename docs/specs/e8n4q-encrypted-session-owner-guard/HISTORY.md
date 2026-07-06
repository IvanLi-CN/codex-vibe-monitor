# Encrypted Session Owner Guard History

- Introduced after production incidents where `encrypted_content` conversations failed after automatic upstream/account migration.
- Separates manual binding intent from encrypted session ownership so operator override remains explicit and automatic routing remains safe.
- Replaced the browser-native dangerous rebinding confirmation with the project Dialog surface so the warning stays localized, accessible, and visually consistent with the route-binding drawer.
- Added a global pause switch for encrypted owner routing so operators can temporarily stop owner binding/enforcement and silence owner warnings without deleting historical owner state.
- Updated the setting default to disabled for fresh databases and added one-time env initialization from `OPENAI_PROXY_ENCRYPTED_SESSION_OWNER_ROUTING_ENABLED`, while preserving saved values in existing databases.
