# Xodus 0.5.0

- Allow a service session to pin a saved identity with `XODUS_ACCOUNT_ID`,
  using the opaque ID returned by `xodus-cli accounts list`.
- Keep session user credentials and refreshed user tokens in memory. Selecting
  a session account never changes the profile's active account or other sessions.
- Preserve access to the same private profile's device identity and previously
  installed content keys. This does not add a purchase check when playing.
- Fail closed if the selected account no longer exists; never silently use
  another account.

Launchers should start one service per game with a unique `XODUS_SOCKET`, pass
the same socket to WineGDK, and stop that service when the game exits. To follow
the launcher's current account, resolve its ID at Play time and pin that ID for
the lifetime of the game. Keep `XODUS_CONFIG_DIR` set to the launcher's private
profile. A separate service without `XODUS_ACCOUNT_ID` may manage that profile
and perform installation ownership checks across its saved accounts.
