# Xodus 0.2.0

- Add purchase verification over Unix IPC (`check-ownership`) across the accounts
  saved in the selected Xodus profile. Only active, explicitly non-trial `Single`
  Store entitlements qualify; subscription access alone does not.
- Add `install-owned`, which requires a positive IPC purchase check before any
  game download or extraction and saves the installed content key in the keyring.
- Add `run --offline-license` to launch an installed game without a new purchase
  or content-license request. Missing local keys fail without a network fallback.
- Support an explicit `XODUS_SOCKET` alongside the isolated `XODUS_CONFIG_DIR`.
- Create or unlock the desktop's default Secret Service keyring through its
  native password prompt when signing in. No plaintext credential fallback.
- Publish tested Linux x86_64 binaries with SHA-256 checksums.

Validated locally through IPC with a purchased Minecraft for PC account.
Trial and subscription rejection, multi-account/profile separation, pagination,
transport errors, keyring setup, and the pre-download gate have automated coverage.
A subscription-only account and full game launch were not live-tested for this release.

See [the IPC and launcher integration guide](ownership-ipc.md) for protocol,
exit codes, compatibility limits, and retained-license behavior.
