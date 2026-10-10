# magicpen

Draw on top of everything on your screen. A small, fast screen-annotation overlay for Linux (X11), written in Rust.

[فارسی](README.fa.md)

![Annotations drawn over the screen, with the toolbar at the top](docs/drawing.png)

*The overlay rendered over a test checkerboard; on a real desktop your windows show through.*

magicpen puts a transparent layer over the whole desktop. Pick a tool and draw on any window, video or slide; switch back to the mouse and keep working while the annotations stay on screen. It is meant for presentations, screen recordings, teaching and code reviews.

- **Small:** a single binary of about 1 MB, with no GTK, Qt or GPU dependency.
- **Idle means idle:** the event loop blocks on the X connection, so it uses no CPU while you are not drawing.
- **Light on memory:** about 11 MB resident on a 1920×1080 screen.

## Contents

- [Features](#features)
- [Requirements](#requirements)
- [Install](#install)
- [Usage](#usage)
- [Keyboard shortcuts](#keyboard-shortcuts)
- [Command line](#command-line)
- [Settings](#settings)
- [Configuration file](#configuration-file)
- [Start on login](#start-on-login)
- [Troubleshooting](#troubleshooting)
- [Limitations](#limitations)
- [How it works](#how-it-works)
- [Development](#development)
- [License](#license)

## Features

- Pen, highlighter, line, arrow, rectangle, ellipse and eraser
- Eight colours and adjustable sizes, with a live size preview in the toolbar
- Undo and redo, including undo of "clear screen"
- Click-through when you are not drawing: the mouse reaches the windows below
- Global shortcuts that work from any application, rebindable in the settings panel
- Hold `Shift` for 45° lines and arrows, squares and circles
- Smoothed freehand strokes
- Draggable toolbar that remembers its position
- Control from scripts with `magicpen toggle`, `clear`, `undo` and so on
- Covers all monitors; scales its interface with the display DPI

## Requirements

- Linux with an **X11** session. Wayland is not supported.
- A **compositing** window manager, so the overlay can be transparent. Xfce (xfwm4 with the compositor on), KDE, MATE, Cinnamon, or a standalone compositor such as picom all qualify.
- `fontconfig` (the `fc-match` command) for the text in the settings panel. Everything else works without it.
- To build: Rust 1.88 or newer. No C libraries or `-dev` packages are needed.

magicpen is developed on Xfce. Other compositing window managers should work but are not tested.

## Install

Clone this repository, then either install with Cargo:

```sh
cargo install --path . --locked
```

or with the Makefile, which also installs an application-menu entry:

```sh
make install            # installs to ~/.local
sudo make install PREFIX=/usr/local
```

`make uninstall` removes it again. To only build, run `cargo build --release`; the binary is `target/release/magicpen`.

## Usage

Start it:

```sh
magicpen
```

A toolbar appears at the top of the screen. From left to right:

| Control | What it does |
| --- | --- |
| Grip (dots) | Drag to move the toolbar. Dragging any empty part of the bar works too. |
| Arrow | Mouse mode: stop drawing and let clicks through to your windows. |
| Pen, highlighter, line, arrow, rectangle, ellipse, eraser | Select a tool and start drawing. |
| Colour dots | Select a colour. |
| `−` dot `+` | Shrink or grow the current tool. The dot previews the size and colour. |
| Undo, redo, bin | Undo, redo, clear the screen. |
| Sliders | Open the settings panel. |
| `×` | Quit. |

There are two modes:

- **Mouse mode** (the default). Annotations stay visible, but only the toolbar reacts to the mouse. Everything else behaves as if magicpen were not there.
- **Drawing mode.** The overlay takes the mouse and the keyboard. Leave it with `Esc`, the arrow button, or the toggle shortcut.

Annotations live in memory only: hiding the overlay keeps them, quitting discards them.

## Keyboard shortcuts

Global shortcuts work from any application:

| Shortcut | Action |
| --- | --- |
| `Super+Shift+D` | Start or stop drawing |
| `Super+Shift+H` | Hide or show the overlay |
| `Super+Shift+C` | Clear the screen |

These are the defaults; change them in the [settings panel](#settings).

While drawing:

| Key | Action |
| --- | --- |
| `Esc` | Back to mouse mode |
| `Ctrl+Z` | Undo |
| `Ctrl+Y` or `Ctrl+Shift+Z` | Redo |
| `Delete` | Clear the screen |
| `P` `H` `L` `A` `R` `O` `E` | Pen, highlighter, line, arrow, rectangle, ellipse (oval), eraser |
| `1` – `8` | Select a colour |
| `[` `]` or mouse wheel | Shrink or grow the current tool |
| `Shift` while dragging | Snap lines and arrows to 45°; make squares and circles |

The letter shortcuts work with a non-Latin keyboard layout active, as long as a Latin layout is also configured.

## Command line

Running `magicpen` while it is already running sends a command to the running instance instead of starting a second one.

| Command | Action |
| --- | --- |
| `magicpen` | Start, or toggle drawing if already running |
| `magicpen toggle` | Start or stop drawing |
| `magicpen show` (or `hide`) | Hide or show the overlay |
| `magicpen clear` | Clear the screen |
| `magicpen undo` | Undo the last annotation |
| `magicpen quit` | Quit the running instance |
| `magicpen --help` | Show usage |

This is useful for binding actions in your desktop's own keyboard settings, or to a stylus or mouse button. If no instance is running, `quit` does nothing and every other command just starts magicpen.

## Settings

![The settings panel](docs/settings.png)

The sliders button opens the settings panel:

- **Pen size** (1–40 px): also used by line, arrow, rectangle and ellipse.
- **Highlighter size** (6–60 px) and **highlighter opacity** (10–90 %).
- **Eraser size** (8–120 px).
- **Shortcuts:** click a shortcut, then press the new combination. `Esc` cancels. A shortcut must include `Ctrl`, `Alt` or `Super`, or be a function key, so that it cannot swallow ordinary typing.

A shortcut shown in red with "(unavailable)" is already taken by another program; pick a different one.

Changes are saved immediately.

## Configuration file

Settings are stored in `~/.config/magicpen/config` (or under `$XDG_CONFIG_HOME`). The file is plain `key = value` text. magicpen rewrites it whenever a setting changes, so edit it while magicpen is not running.

| Key | Default | Meaning |
| --- | --- | --- |
| `tool` | `pen` | Last tool: `pen`, `highlighter`, `line`, `arrow`, `rect`, `ellipse`, `eraser` |
| `color` | `0` | Last colour, `0`–`7` in toolbar order |
| `pen_size` | `4` | 1–40 |
| `highlighter_size` | `22` | 6–60 |
| `highlighter_opacity` | `40` | 10–90, in percent |
| `eraser_size` | `30` | 8–120 |
| `hotkey_toggle` | `Super+Shift+D` | Start or stop drawing |
| `hotkey_hide` | `Super+Shift+H` | Hide or show the overlay |
| `hotkey_clear` | `Super+Shift+C` | Clear the screen |
| `toolbar_x`, `toolbar_y` | centred at the top | Toolbar position in pixels |
| `scale` | `0` | Interface scale. `0` follows the display DPI (`Xft.dpi`); set for example `1.5` to override. |

A shortcut is written as modifiers and one key joined by `+`. Modifiers are `Super`, `Ctrl`, `Alt` and `Shift`. Keys are letters, digits, punctuation, `F1`–`F12`, and `Space`, `Escape`, `Tab`, `Enter`, `Backspace`, `Delete`, `Insert`, `Home`, `End`, `PageUp`, `PageDown`, `Left`, `Right`, `Up`, `Down`, `Print`, `Pause`.

## Start on login

`make install` puts `magicpen.desktop` in your applications menu. To start magicpen with your session, copy it to the autostart directory:

```sh
cp ~/.local/share/applications/magicpen.desktop ~/.config/autostart/
```

On Xfce you can instead add it under Settings → Session and Startup → Application Autostart.

## Troubleshooting

**The screen turns black, or the overlay is not transparent.**
No compositor is running; magicpen prints a warning on startup when this is the case. On Xfce, enable it under Settings → Window Manager Tweaks → Compositor. On a bare window manager, run a compositor such as picom.

**A shortcut does nothing, or shows "(unavailable)".**
Another program or your desktop already owns that combination. Choose another in the settings panel, or bind `magicpen toggle` in your desktop's keyboard settings.

**I cannot type in other applications.**
You are in drawing mode, where magicpen holds the keyboard. Press `Esc`.

**The settings panel has no text.**
`fc-match` was not found or returned no usable font. Install fontconfig.

**The toolbar is too small or too large.**
Set `scale` in the [configuration file](#configuration-file).

**A menu or tooltip appears above my drawing.**
This is intended. Pop-up windows such as menus, tooltips and screen lockers are left above the overlay so they stay usable.

## Limitations

- X11 only. Wayland needs a different mechanism (layer-shell) and is not implemented.
- No pressure sensitivity for graphics tablets; a stylus works as a plain pointer.
- No text tool, and no saving or exporting of annotations. Use your screenshot tool to capture them.
- No system-tray icon.
- The interface is in English.
- The overlay is one window across all monitors, so the toolbar can be dragged anywhere but does not snap to a monitor.

## How it works

magicpen creates one override-redirect, 32-bit ARGB window covering the root window and lets the compositor blend it over the desktop.

- **Click-through** uses the X Shape extension. In mouse mode the window's input region is just the toolbar and settings panel; in drawing mode it is the whole screen.
- **Rendering** is done on the CPU with [tiny-skia](https://github.com/linebender/tiny-skia). Finished annotations are kept both as a vector history (for undo) and as one rasterised screen-sized buffer. On each change only the damaged rectangle is recomposed (buffer, shape in progress, widgets) and sent with `PutImage`.
- **Input** events are drained in a batch before each repaint, so fast pointer motion costs one frame rather than one per event.
- **Global shortcuts** are passive key grabs on the root window. The overlay never takes focus; while drawing it grabs the keyboard instead.
- **Stacking:** the overlay raises itself when the window manager restacks ordinary windows.
- **Single instance:** the running instance owns an X selection, and later invocations send it a client message.

Source layout:

| File | Contents |
| --- | --- |
| `src/main.rs` | X11 setup, event loop, application state, compositing |
| `src/canvas.rs` | Shapes, drawing, undo history |
| `src/ui.rs` | Toolbar and settings panel layout, icons, text |
| `src/keys.rs` | Shortcut parsing and keyboard mapping |
| `src/config.rs` | Configuration file |

## Development

```sh
cargo build --release
cargo fmt --check
cargo clippy -- -D warnings
```

There are no automated tests yet. To try changes without touching your desktop, run magicpen inside a nested X server:

```sh
Xephyr :7 -screen 1280x720 &
DISPLAY=:7 XDG_CONFIG_HOME=/tmp/magicpen-test cargo run --release
```

Xephyr has no compositor, so the background is black there, but all input handling and drawing can be exercised.

Bug reports and pull requests are welcome. Please keep the dependency list short.

## License

[MIT](LICENSE)
