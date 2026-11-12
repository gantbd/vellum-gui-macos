# Pinned dependency patches

## AppKit private-pasteboard release

`objc2-app-kit/` is crates.io `objc2-app-kit` 0.3.2, archive SHA-256
`d49e936b501e5c5bf01fda3a9452ff86dc3ea98ad5f283e1455153142d97518c`.
Its recorded upstream revision is `7b1abfd750a2cacaea71d6a56ecfb83cb7de560b`.
The only source change adds the generated safe `NSPasteboard::releaseGlobally`
declaration, omitted upstream because the generator skips `oneway void`.
The cached crate archive was verified before extraction; licenses and upstream
metadata are retained. See [upstream licensing](objc2-LICENSE.md).

The feature-gated qualification adapter calls it only for a uniquely created
private board, consumes its release eligibility before dispatch and refuses later
I/O without falling back to General. First-party Rust remains unsafe-forbidden.
Remove this patch when a selected upstream binding exposes the method.

Apple contract: <https://developer.apple.com/documentation/appkit/nspasteboard/releaseglobally()>.
Objective-C `oneway` is ignored by objc2's encoding equivalence checks;
`Encoding::Void.equivalent_to_str("Vv")` has an upstream test.

## Accessible Iced lineage and read-only rendered geometry

The following cached crates.io archives were SHA-256 verified before extraction:

| Crate | Version | Archive SHA-256 |
| --- | --- | --- |
| plushie-iced | 0.8.4 | `77b714da831fb8f93b4ba4a7732422e23d5d2aaeae9bb65a3ab97cda1516cf00` |
| plushie-iced-core | 0.8.4 | `10c052034a848926588a398905e605950f0ee2bd3a19c3640aab85da76fc1cdd` |
| plushie-iced-graphics | 0.8.4 | `02281c5f16efc9b26a003ec9849a4381beac50b28a48022cdf1c78d10fe98508` |
| plushie-iced-widget | 0.8.4 | `37f3c06658edc0d9d95a465636d5f69ef4cab1036225a1840c5a5c52be0407a9` |
| plushie-iced-futures | 0.8.4 | `5bdc00d5d80eff21a6efaa7422250f748714fba39d89f1ab3cb15a6bc4b9c8e5` |
| plushie-iced-winit | 0.8.4 | `49404343107bcd1ded65ed793ca3851def3c09d6def4f89a2ecce4898f993538` |

All five extracted packages record upstream revision
`e423a8efb3ec953071be5f2fc995cdb99c21b320` from
[plushie-ui/plushie-iced](https://github.com/plushie-ui/plushie-iced). The
[MIT license](iced-LICENSE) and upstream package metadata are retained. This is
the same exact accessible renderer lineage used by Chronos; the `a11y` feature
remains enabled.

Changes are limited to the three `text/editor.rs` / `text_editor.rs` files:
an optional renderer trait query, extraction of the existing primary-geometry
calculation for read-only explicit cursors with a region ceiling and cached-layout
validation, and a widget query gated on completed layout, action invalidation and
exact text-viewport dimensions. Unsupported renderers return None. Reads neither
move cursors nor shape text; failures return no partial geometry. There is no
independent layout algorithm or engine dependency. These queries describe the
last completed widget layout, not future/unapplied widget styles or OS geometry.

`plushie-iced-futures` and `plushie-iced-winit` are also locally patched so the
published 0.8.4 graph remains reproducible with the current locked WebAssembly
support crates. The sole source-metadata change is
`plushie-iced-winit/Cargo.toml`: its WebAssembly-target-only `web-sys`
requirement is pinned from `0.3.85` to `=0.3.107`, the version selected with
the compatible `wasm-bindgen` family in this lockfile. It does not alter the
macOS runtime. No WebAssembly behavior is claimed until that target is tested.
Three trailing-space runs in upstream documentation comments were removed as
non-semantic packaging normalization so the repository-wide whitespace gate
remains exact.
