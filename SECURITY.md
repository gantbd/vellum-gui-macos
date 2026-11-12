# Security policy

This repository is unreleased. Report suspected vulnerabilities privately to
the repository owner; do not put workspace paths, source text, clipboard data,
or annotation content in a public issue.

Trust boundaries include window/input events, IME composition, rendered
geometry, literal locators, local file and annotation adapters, AppKit
clipboard access, vendored UI/native bindings, and Vellum host contracts.
Framework diagnostics must remain content-redacted. Stale attachment tokens,
destination confirmations, composition events, and clipboard completions must
not retarget newer state.

Root discovery is descriptor-anchored, bounded, and fail-closed, but it is not
a sandbox against a malicious same-user process or hostile parent directory.
The vendored patches are part of the security and supply-chain boundary; update
their provenance, differential review, tests, and hashes together.

Before release, repeat STRIDE/LINDDUN review, dependency and license inventory,
real-window accessibility/IME qualification, signing/notarization, artifact
verification, activation/rollback, and private reporting-channel setup.
