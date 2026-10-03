# Xodus 0.4.0

- Add `accounts list`, `accounts select ID` and `accounts remove ID` for
  multi-account frontends. JSON contains only opaque IDs, usernames and the
  active-account flag; tokens remain in the profile-scoped OS keyring.
- `accounts --unlock ...` uses the desktop's native keyring prompt. Account
  management does not provision a device or make network/license requests.
- Switching accounts invalidates derived tokens. Removing one account keeps
  other accounts and installed-content keys; removing the active account selects
  the next saved account, if any.
- Route tracing diagnostics to stderr to keep machine-readable stdout clean.

Frontends must stop their owned Xodus service before changing accounts and
restart it afterward so WineGDK uses the selected identity. Purchase checks
continue to consider all accounts saved in that isolated profile.

For in-memory game execution, use WineGDK with `WINE_DLL_FILE_MAP` support.
