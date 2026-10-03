# Xodus 0.6.1

- Add `xodus-cli accounts profile <account-id>` for launchers to retrieve a saved account's Xbox gamertag and gamer picture URL. Return only presentation metadata and the existing opaque account ID, never tokens, real names or Xbox user IDs.
- Profile queries use an isolated identity snapshot without changing the current account, interrupting games, provisioning devices or checking ownership. Requests are bounded and errors omit credentials.
- Fix a pre-existing prefix-cache race uncovered by release CI: wait for buffered file writes before reading through the separate cache handle, and use newly cached bytes before requesting more upstream data. Includes a deterministic regression test with the file worker deliberately blocked.

The existing account commands, private sockets and per-game sessions remain compatible.
The 0.6.0 tag did not produce a release; 0.6.1 includes its profile functionality and the cache correction.
