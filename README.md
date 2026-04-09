# Desk Switch

A cross-platform KVM switch built in Rust. Use two laptops (Windows + Mac) with one monitor, and seamlessly switch which laptop is primary while using the other as an additional display.

## Features

- **One-click GUI** -- double-click to launch a beautiful desktop app, pick Primary or Display, done
- **Screen streaming** -- primary machine captures its screen and streams it to the display machine at up to 30 FPS
- **Input forwarding** -- keyboard and mouse events from the display machine are forwarded back to the primary
- **Role switching** -- toggle which laptop is primary with a single command
- **Auto-discovery** -- machines find each other on the same WiFi network via UDP broadcast
- **Auth key** -- shared key prevents unauthorized connections
- **Single binary** -- GUI + CLI in one executable, no runtime dependencies

## Requirements

- Both machines on the same local network (WiFi or Ethernet)
- That's it. The installer handles everything else.

## Install (One Click)

### macOS

1. Clone or download this repo
2. Double-click **`Install.command`** in Finder
3. It installs Rust (if needed), builds, and creates **Desk Switch.app** in `~/Applications`
4. The app opens automatically. Find it in Launchpad or `~/Applications` from now on.

### Windows

1. Clone or download this repo
2. Double-click **`Install.bat`** in Explorer
3. It installs Rust (if needed), builds, and creates shortcuts on **Desktop** and **Start Menu**
4. The app opens automatically. Click the Desktop shortcut from now on.

### Connecting Two Machines

1. Install on **both** machines using the steps above
2. On Machine A: open the app, expand **Settings**, and **Copy** the auth key
3. On Machine B: open the app, expand **Settings**, paste the key into the **Paste Key** field
4. Click **PRIMARY** on the machine you want as your workstation
5. Click **DISPLAY** on the other machine
6. They auto-discover each other and start streaming

## Advanced: CLI Mode

You can also use the command line directly:

```bash
desk-switch                      # launches the GUI (default)
desk-switch start --primary      # CLI: start as primary
desk-switch start --display      # CLI: start as display
desk-switch switch               # toggle default role
```

## CLI Reference


| Command                                | Description                                           |
| -------------------------------------- | ----------------------------------------------------- |
| `desk-switch setup`                    | Generate auth key, create config, set up permissions  |
| `desk-switch start --primary`          | Start as primary (capture + stream screen)            |
| `desk-switch start --display`          | Start as display (view remote screen + forward input) |
| `desk-switch start`                    | Start with whatever `default_role` is set in config   |
| `desk-switch status`                   | Show config and available monitors                    |
| `desk-switch switch`                   | Toggle `default_role` between primary/display         |
| `desk-switch monitors`                 | List displays with index and resolution               |
| `desk-switch config`                   | Print full config as JSON                             |
| `desk-switch config set <key> <value>` | Update a config value                                 |
| `desk-switch config get <key>`         | Read a config value                                   |


## Configuration

Stored at `~/.desk-switch/config.json`.


| Key               | Default         | Description                                              |
| ----------------- | --------------- | -------------------------------------------------------- |
| `hostname`        | (auto-detected) | Machine identifier                                       |
| `default_role`    | `idle`          | Role on start: `idle`, `primary`, or `display`           |
| `auth_key`        | (generated)     | Shared key -- must match on both machines                |
| `stream_port`     | `9876`          | TCP port for screen data + input events                  |
| `discovery_port`  | `9877`          | UDP port for peer discovery broadcast                    |
| `capture_quality` | `60`            | JPEG quality 1-100. Lower = smaller frames, less CPU     |
| `capture_monitor` | `0`             | Index of monitor to capture (see `desk-switch monitors`) |
| `viewer_monitor`  | `0`             | Index of monitor to show viewer on                       |
| `max_fps`         | `30`            | Frame rate cap                                           |


## Setup Examples

### Windows primary, Mac as 3rd screen

```
[Windows Laptop] --HDMI--> [Monitor]       (2 screens on Windows)
[Mac Laptop]     --WiFi--> shows Windows screen  (3rd screen)
```

```bash
# Windows
desk-switch start --primary

# Mac
desk-switch start --display
```

### Mac primary, Windows as 3rd screen

```
[Mac Laptop]              primary workstation
[Windows Laptop] --HDMI--> [Monitor]       shows Mac's screen
[Windows screen]                           also shows Mac's screen
```

```bash
# Mac
desk-switch start --primary

# Windows
desk-switch start --display
```

### Choosing which monitor to capture/display

```bash
# List monitors on each machine first:
desk-switch monitors

# Capture the HDMI monitor (index 1) instead of built-in:
desk-switch config set capture_monitor 1

# Show viewer on the HDMI monitor (index 1):
desk-switch config set viewer_monitor 1
```

## Permissions

### macOS

- **Screen Recording**: required for screen capture. Grant in System Settings > Privacy & Security > Screen Recording.
- **Accessibility**: required for input capture and simulation. Grant in System Settings > Privacy & Security > Accessibility.

Running `desk-switch setup` will open the relevant settings pane.

### Windows

- **Firewall**: `desk-switch setup` adds inbound rules for TCP/UDP ports 9876-9877. If it fails, run an elevated (Administrator) terminal and re-run setup, or add the rules manually.

## Architecture

```
Primary (3 pipelined threads):
  [Screen Capture] --> [JPEG Encode] --> [TCP Send]

Display (3 pipelined threads):
  [TCP Receive + JPEG Decode] --> [Window Render]
  [Input Capture (rdev)]      --> [TCP Send to primary]
```

Discovery uses UDP broadcast on port 9877. Both machines announce themselves every 2 seconds. When the display finds a primary, it initiates a TCP connection on port 9876, performs an auth handshake, and begins receiving frames.

## Troubleshooting


| Problem               | Fix                                                      |
| --------------------- | -------------------------------------------------------- |
| "Config not found"    | Run `desk-switch setup` first                            |
| No peer found         | Both machines on same network? Firewall allows UDP 9877? |
| Authentication failed | Auth key must match on both machines                     |
| Black/no image        | Grant Screen Recording permission on macOS primary       |
| High latency          | Lower `capture_quality` (e.g. 40) or `max_fps` (e.g. 15) |
| Input not working     | Grant Accessibility permission on macOS                  |


## License

MIT
