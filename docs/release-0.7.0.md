# Xodus 0.7.0

- Add Xbox token requests for UWP applications using their client and title identity. Cache short-lived sessions and audience tokens per account and application, retaining the associated signing key.
- Resolve title-specific Xbox endpoints, including secure WebSocket audiences used by real-time activity. Support request signatures without logging tokens or authenticated response bodies.
- Add Store catalog, signed application receipt, and content-license requests. Preserve Microsoft receipt XML and report entitlement or service failures instead of manufacturing licenses. This does not implement a purchase checkout flow.
- Preserve account selection, isolated game sessions, ownership checks, and profile commands from 0.6.1.

XML IPC adds Xbox request/response types 7/8 and Store request/response types 9/10. Ownership keeps types 5/6. Use this release with WineGDK 11.18-3-gdkcomponents or newer for the UWP bridge; earlier experimental UWP patches used conflicting type numbers.

The Linux archive contains `xodus-cli`, `xodus-service`, and `LICENSE`. Existing credentials remain in the user's credential store and are never included in the release.
