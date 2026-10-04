# Xodus 0.7.3

Fix periodic multi-second stutters in Minecraft UWP caused by synchronous Store license queries waiting for the 60-second cache to refresh.

- Renew requested Store licenses in the background after 40 seconds, before the cache expires.
- Serve valid cached replies without waiting for the network refresh lock; serialize network fetches to avoid duplicate requests.
- Preserve the 60-second cache lifetime and check absolute license expiration and current account credentials. Failed refreshes do not extend cached entitlement.
- Schedule one advance renewal per client demand and bound retries; unused entries do not poll the Store indefinitely.

Measured on Minecraft UWP 1.21.0.3: recurring frame stalls were approximately 2.3–3.3 seconds before the fix, with three captured stacks blocking in the Store license socket read on the main thread. After the fix, local license queries at 65 and 130 seconds took 3.76 ms and 3.64 ms with a valid license. The in-world test also confirmed the periodic stutters stopped. These are development measurements, not a controlled hardware benchmark.

Cold or expired-cache requests can still wait for the Store. No Wine update or protocol change is required for this fix; existing sign-in and entitlement requirements remain unchanged.

Validation includes eight Store tests covering cache concurrency, expiration, account separation, logout and license validation, plus the release workflow's workspace test suite and a release workspace build. The live developer-token test is excluded, as in the release workflow.

The archive contains `xodus-cli`, `xodus-service` and `LICENSE`, with a separate `SHA256SUMS` file, following the existing `xodus-0.7.3-linux-x86_64.tar.gz` naming scheme.
