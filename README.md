<p align="center"><img width="128" src="assets/Icon/Icon.ico" /></p>
<h1 align="center">Xodus</h1>
<p align="center">The great gaming migration to Linux</p>
<p align="center">
    <a href="https://discord.gg/ZG774FK4tq">
        <img src="https://img.shields.io/discord/1123890623586504714?logo=discord&style=for-the-badge&color=red&label=Game+Launchers+Reverse+Engineering" alt="Discord" />
    </a>
</p>

> [!CAUTION]
> This is an unofficial project - use at your own risk. It is not affiliated with, endorsed by, or sponsored by Microsoft or XBOX; all trademarks, product names, and company names or logos mentioned herein are the property of their respective owners.

## Current state of the project

The project can now login, download packages and obtain licenses for games.

These parts are still quite scattered around.

- [x] Device login
- [x] User login
- [x] XBOX authorization
- [x] MSIXVC download
- [x] On-demand .exe decryption [#50](https://github.com/xodus-gaming/xodus/issues/50)
- [ ] MSIXVC2 support [#53](https://github.com/xodus-gaming/xodus/issues/53)

## FAQ

**Q: What is Xodus**  
Xodus aims to bring XBOX PC games to Linux and possibly Mac devices.

**Q: When can I play my Minecraft Bedrock?**  
While Xodus is quickly maturing, there is still a lot of work to support it from Wine standpoint to provide necessary XBOX Services to games.  
_TL;DR_ soon<sup>tm</sup>

**Q: How to get involved?**  
Start by joining our Discord or review any open GitHub issues .

**Q: What games will be supported?**  
We hope to manage to support most of the catalog, the limitation is the game has to be GDK and in MSIXVC format.  
So far `Gears of War 4` is a prominent unsupported title for the time being.

**Q: Will XBOX Backward Compatibility on PC work?**  
While Xodus is capable of downloading and running those titles. It's possible these games will work only after additional patches to wine, dxvk or vkd3d-proton.

## Building

The project structure is as follows.

```
.
├── msixvc - [rlib] common rlib crate for utilities for parsing MSIXVC and XSP files
├── xodus - [rlib] common rlib crate that contains core xodus functionality, API calls abstractions and utilities
├── xodus-cli - [bin] CLI currently used for iterating over new xodus features
└── xodus-service - [bin] service process exposing a xodus.sock for IPC communication, it takes care of xgameruntime.dll integration.
```

> [!NOTE]
> xodus-service aims to become a main point of integration. All xodus clients will connect to it to interact with games and XBOX services.

### Prerequisites

- Rust version 1.98 or later
- Right now CLI relies on wry and tao to show a login page. Consult https://docs.rs/wry/latest/wry/#platform-considerations
- xodus-service relies on `protoc` to compile `proto/` definitions make sure to install it for your platform

### Running

Building all crates in release mode

```bash
cargo build --release --workspace
```

Running cli in debug

```
cargo run -- --help
```

Running xodus-service in debug

```
cargo run --bin xodus-service
```

Set `XODUS_SOCK_NAME` to change the socket filename under `$XDG_RUNTIME_DIR`
on Linux (`/tmp` on macOS). An unset or empty value defaults to `xodus.sock`.
Use the same value when launching WineGDK so both connect to the same socket:

```bash
XODUS_SOCK_NAME=my-xodus.sock cargo run --bin xodus-service
```

Set `XODUS_CONFIG_DIR` to give the CLI and service a shared, isolated profile:

```bash
export XODUS_CONFIG_DIR="$HOME/.config/xodus/my-profile"
export XODUS_SOCK_NAME=my-profile.sock
cargo run --bin xodus-cli -- login
cargo run --bin xodus-service
```

The directory is created automatically. The existing credential backend stays
in use: system-keyring credentials are scoped by the canonical directory path,
so different profiles cannot overwrite each other's entries. Builds using the
`xodus/key-chain-file` feature store `.xodus-keyring.ron` inside the profile
instead. On Linux, the CLI's login browser data is stored in `webview/` inside
the profile. Unset or empty `XODUS_CONFIG_DIR` preserves the existing defaults.
Existing credentials are not copied into a new profile; log in for that profile.
Moving a profile changes its credential namespace, while symlinks resolving to
the same directory share the same namespace.

The service socket is configured independently: give simultaneously running
profiles different `XODUS_SOCK_NAME` values and pass the matching value to
WineGDK. Download, extraction, and license output paths still follow CLI options.

Debug and profile `xodus-cli` or `xodus-service` with [tokio-console]([tokio-console](https://github.com/tokio-rs/console))

```
RUSTFLAGS="--cfg tokio_unstable" cargo run --features tokio_console 
```

> [!WARNING]
> For better performance when decrypting MSIXVC files, the `aes` and `ssse3` features are enabled on `x86_64`,
> and the `aes` feature is enabled on `aarch64`. This means that the program will crash with an illegal instruction
> error when running on a CPU which doesn't support those instructions.
>
> See https://en.wikipedia.org/wiki/AES_instruction_set for a list of compatible CPUs (every processor from
> 2011 onwards should be supported).

### CLI Usage

For launcher integration, purchase checks over IPC, remembered accounts, and
playing installed games without another license request, see
[Purchase verification and local launch licenses](docs/ownership-ipc.md).

`XODUS_SOCKET` can select an absolute socket path; it takes precedence over
`XODUS_SOCK_NAME` in both the CLI and the service.

```
Usage: xodus-cli <COMMAND>

Commands:
  download    Download msixvc or xsp files for a given game
  license     Dump CIKs for use with XvdTool
  extract     Extract locally stored msixvc file
  login       
  streaming   Download and extract the game through streaming algorithm
  clep        Generate or decrypt base64-encoded CLEP challenge data
  sp-license  Decode SPLicenseBlock
  help        Print this message or the help of the given subcommand(s)

Options:
  -h, --help     Print help
  -V, --version  Print version
```

## Special Thanks

- [XvdTool.Streaming](https://github.com/LukeFZ/XvdTool.Streaming) and [CikExtractor](https://github.com/LukeFZ/CikExtractor) by LukeFZ
