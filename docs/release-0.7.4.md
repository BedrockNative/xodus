# Xodus 0.7.4

Refresh the Xbox sign-in window with system-aware styling and an embedded desktop icon.

- Apply light and dark themes throughout sign-in, including verification and recovery pages, while preserving forced-color accessibility modes.
- Match the native WebView background to the initial system theme and use Microsoft's white logo in dark mode.
- Name the window "Login with Xbox" and embed the Xbox icon in the CLI binary.
- On Linux, install the embedded icon and hidden desktop entry under `$XDG_DATA_HOME` (default `~/.local/share`) so Wayland desktops can resolve the window icon. No separate asset installation is needed.

Microsoft continues to handle form submission and authentication.

The release workflow checks formatting, runs the workspace test suite (excluding the live developer-token test), and builds the release binaries.

The archive contains `xodus-cli`, `xodus-service` and `LICENSE`, with a separate `SHA256SUMS` file, using the existing `xodus-0.7.4-linux-x86_64.tar.gz` naming scheme.
