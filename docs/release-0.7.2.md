# Xodus 0.7.2

Reuse account-bound Xbox bootstrap sessions across WineGDK launches, reducing repeated authentication requests while checking fresh account identity and credential expiration.

- Add a bounded, volatile GDK session cache scoped to account, OAuth client, title and trust mode. Logout, account changes and forced refresh invalidate the cached state.
- Advertise cache protocol support to WineGDK; existing clients remain compatible. Ownership retains message IDs 5/6, Xbox tokens 7/8 and Store 9/10; GDK sessions use 11/12.
- Replace the fixed five-minute Xbox broker-session cutoff with checks against the actual credential expiration times and a renewal margin.
- Do not persist or log cached token/proof-key payloads. OAuth credentials are still requested through the existing authentication path.

Use with WineGDK 11.18-5-winrt for cross-process startup caching. Existing login and Store entitlement requirements remain unchanged.

Validation: cache isolation, wire round trips, expiry, account switching and logout tests; fresh and cached GDK launches with online services tested alongside WineGDK.

The archive contains `xodus-cli`, `xodus-service` and `LICENSE`, with a separate `SHA256SUMS` file, using the existing `xodus-0.7.2-linux-x86_64.tar.gz` naming scheme.
