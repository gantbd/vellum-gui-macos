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

## Iced read-only rendered geometry

The following cached crates.io archives were SHA-256 verified before extraction:

| Crate | Version | Archive SHA-256 |
| --- | --- | --- |
| iced_core | 0.14.0 | `91ab1937d699403e7e69252ae743a902bcee9f4ab2052cc4c9a46fcf34729d85` |
| iced_graphics | 0.14.0 | `234ca1c2cec4155055f68fa5fad1b5242c496ac8238d80a259bca382fb44a102` |
| iced_widget | 0.14.2 | `b1596afa0d3109c2618e8bc12bae6c11d3064df8f95c42dfce570397dbe957ab` |

Core/graphics record upstream revision `3997291f318a8bc06fa522f5579836fb3feb94df`;
widget records `54abf81d13fec06d4d9ac754b03ce2c3313e8a1f`. The
[MIT license](iced-LICENSE) and upstream package metadata are retained.

Changes are limited to the three `text/editor.rs` / `text_editor.rs` files:
an optional renderer trait query, extraction of the existing primary-geometry
calculation for read-only explicit cursors with a region ceiling and cached-layout
validation, and a widget query gated on completed layout, action invalidation and
exact text-viewport dimensions. Unsupported renderers return None. Reads neither
move cursors nor shape text; failures return no partial geometry. There is no
independent layout algorithm or engine dependency. These queries describe the
last completed widget layout, not future/unapplied widget styles or OS geometry.
