# Xodus 0.7.1

- Support older UWP Xbox sign-in requests that provide package and title identity without an OAuth client ID. Resolve the public Minecraft Windows registration only for the matching package/title pair; explicit client IDs continue to work.
- Request content entitlement and runtime leases together. Validate package identity, license type, expiration, policy flags and fresh matching leases before reporting an active Store license.
- Recognize runtime lease XML and non-expiring full licenses. A valid full entitlement takes precedence over a legacy trial receipt; a lease alone never grants ownership.
- Improve malformed-license diagnostics without logging credentials or license bodies.

Use with WineGDK 11.18-3-gdkcomponents or newer. Login still requires an authenticated Xodus account and Store access still requires a valid entitlement. Purchase checkout is not implemented.

The archive uses the existing `xodus-0.7.1-linux-x86_64.tar.gz` naming scheme and contains `xodus-cli`, `xodus-service` and `LICENSE`, with a separate `SHA256SUMS` file.
