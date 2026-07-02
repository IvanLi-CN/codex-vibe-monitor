# Encrypted Session Owner Guard History

- Introduced after production incidents where `encrypted_content` conversations failed after automatic upstream/account migration.
- Separates manual binding intent from encrypted session ownership so operator override remains explicit and automatic routing remains safe.
- Replaced the browser-native dangerous rebinding confirmation with the project Dialog surface so the warning stays localized, accessible, and visually consistent with the route-binding drawer.
