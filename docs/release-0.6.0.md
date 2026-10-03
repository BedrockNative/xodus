# Xodus 0.6.0

- Add `xodus-cli accounts profile <account-id>` for launchers to retrieve a saved account's Xbox gamertag and gamer picture URL.
- Return only presentation metadata and the existing opaque account ID: never tokens, credentials, real names or Xbox user IDs.
- Query a session-local identity without changing the profile's current account, interrupting running games or checking game ownership.
- Use bounded requests and sanitized errors. Profile lookup does not provision a device or open a login dialog; offline callers can keep their own presentation cache.

The existing `accounts list`, account selection, private sockets and per-game sessions remain compatible.
