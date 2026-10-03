# Xodus 0.2.1

- On Linux, `xodus-service` now requests the desktop's native password dialog when
  the default Secret Service keyring is locked, before accessing credentials or
  provisioning a device. Passwords are handled by the desktop, not Xodus.
- Cancelling or timing out the dialog stops startup cleanly instead of panicking
  or treating inaccessible credentials as a missing device identity.
- An already unlocked keyring does not show a prompt. Service startup never
  creates a replacement keyring; first-time keyring creation remains part of login.

Launcher integration: allow at least the five-minute native prompt window before
timing out service startup. Orion's updated startup timeout is six minutes and
still permits immediate cancellation. Update the launcher as well as Xodus.

Validation: 62 Rust tests and 53 Orion tests passed locally, including simulated
locked/unlocked/missing keyrings, cancellation, and startup beyond the old
20-second launcher timeout. The user's real keyring was not locked for testing.
