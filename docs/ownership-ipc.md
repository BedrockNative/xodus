# Purchase verification and local launch licenses

This extension keeps the WineGDK token protocol compatible. It does not turn a
successful content-license request into evidence of purchase: subscriptions can
also grant content licenses.

## Launcher flow (Linux)

Use the same private `XODUS_CONFIG_DIR` and socket for login, the service, install,
and play. Do not connect to the system Xodus service as a fallback.

On Linux, `login` first asks the desktop Secret Service to create a missing default
keyring or unlock an existing one. Enter the keyring password only in the native
desktop prompt. Xodus never receives that password, runs itself as root, or
silently falls back to plaintext storage. Cancelling the prompt stops before
Microsoft sign-in. An existing unlocked keyring does not show this prompt.
`xodus-service` also requests this native unlock prompt before reading credentials
or provisioning a device. Cancelling it stops startup cleanly, without treating a
locked keyring as a missing device identity. Service startup does not create a
missing keyring; use `login` for initial setup. The native prompt allows up to five
minutes, so launchers must keep the service alive while waiting (Orion allows six
minutes for the prompt and remaining startup work). Cancelling in Orion still
stops the waiting service immediately.
GNOME Keyring or a Secret Service-compatible KWallet must be installed and running
in the user session; a missing service produces an actionable error. Installing
desktop packages is not done automatically or without user authorization.

```sh
export XODUS_CONFIG_DIR="$HOME/.config/orion-launcher/xodus"
export XODUS_SOCK_NAME=orion-example.sock
xodus-cli login
# Start xodus-service in another terminal with these same variables.
xodus-cli check-ownership 9NBLGGH2JHXJ
xodus-cli install-owned 9NBLGGH2JHXJ PACKAGE_URL GAME_DIRECTORY
xodus-cli run GAME_DIRECTORY /path/to/wine --offline-license --exe 'Minecraft.Windows.exe'
```

For a local package, pass `file:///absolute/path/game.msixvc` as the source.
Starting with Xodus 0.3.0, `run` accepts game arguments after `--`, for example
`xodus-cli run GAME_DIRECTORY /path/to/wine --offline-license --exe GAME.exe -- ARGUMENT VALUE`.
They are passed as separate literal OS arguments after the game executable, never
through a shell. The caller is responsible for supplying flags supported by the game.
The retained-license and encrypted-file mapping behavior is unchanged.
`install-owned` checks purchase through IPC **before** opening/downloading the
package, creating the destination, or acquiring a content key. The caller must
supply the Store product matching the package (not a bundle or subscription ID).
The Microsoft licensing service must separately grant a license for that exact
package's content ID; a purchase result alone cannot decrypt anything.

On successful installation, the content key acquired from Microsoft is saved in
the selected profile's credential backend, indexed by content UUID. It is not
written into the instance directory or returned over IPC. `run --offline-license`
uses only this installed key: no purchase or content-license request, and no
automatic network fallback if it is missing. Existing installations made with
`streaming`/`extract` need an `install-owned` installation to use this mode.
The game may still use Xbox network services through WineGDK.

Successful logins remember accounts in the **same profile**. Checks try the active
account first, then other saved accounts, without changing the active account.
One proven purchase is enough; a failed account check never counts as a purchase.
`logout` clears all remembered account credentials in that profile, but retains
installed content keys so existing instances can still launch. Account isolation
is independent of socket isolation. No credentials from a different profile are
discovered or copied. Prefer the default OS keychain backend; the optional
development-only `key-chain-file` feature is not encrypted storage.

## Wire format

Connect to `XODUS_SOCKET` if nonempty (absolute path), otherwise to
`$XDG_RUNTIME_DIR/$XODUS_SOCK_NAME` on Linux (`/tmp` on macOS). The default filename
is `xodus.sock`. The service socket has mode `0600`.

All numbers are little-endian. The existing eight-byte header is unchanged:

| Offset | Type | Value |
| --- | --- | --- |
| 0 | u32 | `0x58445358` (XML magic) |
| 4 | u16 | `5` request, `6` response |
| 6 | u16 | UTF-8 payload length in bytes |
| 8 | bytes | XML payload |

```xml
<OwnershipRequest><productId>9NBLGGH2JHXJ</productId></OwnershipRequest>
<OwnershipResponse><productId>9NBLGGH2JHXJ</productId><status>purchased</status></OwnershipResponse>
```

Store product IDs must be twelve uppercase ASCII letters/digits. Request fields
other than `productId` are rejected. Clients must validate magic, response type,
payload length, matching product ID and the exact positive status. Read headers
and payloads completely: a socket read can return only part of either.

| Status | Meaning | CLI exit |
| --- | --- | --- |
| `purchased` | Active, non-trial, `Single` acquisition verified online | 0 |
| `not_owned` | Completed checks found no qualifying purchase | 2 |
| `no_account` | No signed-in account in this profile | 3 |
| `unavailable` | Network/authentication/storage failure or ambiguous evidence | 4 |
| `invalid_request` | Invalid XML or product ID | 4 |

`check-ownership` prints the same fields as JSON. Transport failures use exit 4
and a generic error on stderr. No tokens, account IDs or content keys are exposed.
The server limits a complete check to 45 seconds; the client allows 50 seconds.
Collections pagination and response sizes are bounded. There is no ownership
cache: invoke the check when installing, not on every Play or in a polling loop.

## Microsoft contract and validation

The purchase classifier uses the Microsoft Store Collections fields documented
in [Collections v8](https://learn.microsoft.com/en-us/gaming/gdk/docs/store/commerce/service-to-service/microsoft-store-apis/xstore-v8-query-for-products).
`Single` means direct purchase or code redemption; `Recurring` and `Conditional`
do not meet the purchase policy. Status, validity dates, and explicit non-trial
flags are required. Missing/unknown evidence fails closed. A license token's
far-future expiration date or `isShared` field is **not** proof of purchase.

Authentication exchanges the profile's device and user MSA credentials for
`www.microsoft.com` tokens, sharing the content-licensing authentication code.
The device token authorizes the request; the user token identifies an explicit
MSA beneficiary. An Xbox XSTS token without this beneficiary returned an empty
collection for a bought-game account and must not be used here.
The Collections endpoint is documented for partner services; this consumer
flow is an upstream compatibility dependency, not a guaranteed public client API.
Authentication restrictions (including 401/403) produce `unavailable`, never
`not_owned` or `purchased`.

Live validation with a bought-PC-game account returned both a trial entitlement
and a separate active, non-trial `Single` entitlement for Minecraft for Windows.
Only the latter satisfies the purchase policy. Subscription-only accounts have
automated fixture coverage, but have not been validated against the live service.

Automated tests cover classification, missing evidence, HTTP failures,
pagination, fragmented Unix-socket messages, malformed XML, profile/account
separation, retained local licenses, and the install gate preceding all game I/O.
They do not substitute for testing a bought-game account and a subscription-only
account against Microsoft's live service.
