# Xodus 0.3.0

- `xodus-cli run` now accepts game arguments after `--` and forwards each argument
  literally after the game executable in Wine's argument vector.
- Spaces, Unicode, empty arguments and leading dashes are preserved. Arguments
  after the separator cannot become Xodus options. No shell expansion occurs.
- Existing commands without game arguments behave as before. The local launch
  license, private socket/profile, keyring unlock and encrypted-file mapping
  remain unchanged.

Example syntax (replace placeholders with flags supported by your game):

```sh
xodus-cli run GAME_DIRECTORY /path/to/wine --offline-license --exe GAME.exe -- ARGUMENT VALUE
```

The caller is responsible for choosing supported game flags. Xodus does not add
Minecraft-specific flags or change the game's display settings. Orion handles
per-instance environment settings and optional resolution through Gamescope.

Tests cover literal argument forwarding, CLI separator handling and compatibility
with existing invocations. A complete Minecraft session was not live-tested.
