# Vellum GUI for macOS

`vellum-gui-macos` is the standalone macOS application and reusable Iced
presentation library for Vellum. It is a peer of `vellum-tui` and `vellum-mcp`,
not part of the semantic engine and not a second source/history authority.

It starts empty or from files, directories, or a mix:

```sh
cargo run --locked --
cargo run --locked -- README.md src
```

It can instead project a running local Vellum service:

```sh
cargo run --locked -- --service /path/to/vellum.sock
cargo run --locked -- --service /path/to/vellum.sock --credential /path/to/credential
```

The Open field accepts one file or directory at a time. Directory discovery is
deterministic, refuses symbolic links and special files, and admits at most 16
unique regular files. The editor host independently revalidates each locator.
In service mode the File and Directory buttons submit literal locators to the
service, framework edits become witnessed replacements, and an indeterminate
operation locks editing until its exact replay completes.

`tests/live/service_journey.sh` builds the exact sibling Vellum daemon, starts
it on private Unix sockets with edit/save ceilings, drives the reusable GUI
controller headlessly through open/edit/save/disconnect, verifies exact
persisted bytes, and requests orderly control-socket shutdown. CI runs this
journey without opening a desktop window.

## Composition boundary

The `vellum_gui_macos` library owns Iced state, projection, input translation,
rendered-selection geometry, and attachment identities. It owns no process
lifecycle, filesystem policy, document source, or history. The binary selects
the macOS adapters and window lifecycle.

Vellum and Chronos now use the same exact accessible `plushie-iced` 0.8.4
lineage. A workbench can import both presentation libraries under one Iced
runtime. Because Cargo applies `[patch]` only at the workspace root, that
workbench must select Vellum's qualified vendored Plushie packages itself. The
composition root must also route effects to each engine's own authority and
must not import either standalone binary. This dependency convergence makes
composition possible but does not itself claim a qualified combined GUI
executable.

Both embedded and service-backed presentations expose component update,
subscription, view, and lifecycle output. Component close never calls
`iced::exit`; only the standalone application wrapper owns process exit and
native window-close subscription.

## Qualified dependency patches

The renderer contract relies on reviewed, pinned patches to the accessible
`plushie-iced` 0.8.4 lineage and `objc2-app-kit` retained under `vendor/`. They
add a read-only, bounded selection-geometry query and the generated
private-pasteboard release binding. Their upstream revisions, archive hashes,
licenses, exact changes, and nonclaims are recorded in
[vendor/README.md](vendor/README.md). Upstream Plushie Iced does not provide the
geometry query; removing these patches would discard qualified multi-selection
rendering behavior.

## Current source checkout

Until the pinned Vellum commit is reachable from its declared GitHub remote,
this unreleased peer uses a sibling `../vellum` checkout. Run:

```sh
cargo test --all-targets --locked --offline
```

GitHub source installation will replace the Vellum paths with exact Git
revisions after remote reachability is established. Nothing is published to
crates.io, signed, notarized, or released.

The optional `desktop-qualification` feature opens a real window and must be
run deliberately. Normal automated tests do not open UI windows.

The Unix-socket service mode is a local peer projection, not a remote-network
transport claim. Live filesystem watching, IME preedit, bidi layout,
screen-reader behavior, signing, notarization, and a stable UI API remain
unclaimed.
