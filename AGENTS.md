# MyMemo Clipboard repository rules

- This repository owns only the MyMemo Clipboard desktop application.
- Do not import, copy, embed, invoke, or require Jamserver source, packages, routes, credentials, configuration, or runtime services.
- The production server origin is exactly `https://memos.my`; desktop sessions use only the `smc_` token format.
- Server integration is limited to the versioned Clipboard HTTP API and its documented request/response data structures in `contracts/clipboard-v1.openapi.json`.
- Keep the vendored contract byte-identical to the canonical copy in the SmallMemo repository. Do not share implementation packages or add a git submodule between the repositories.
- Before release, run TypeScript, frontend, static regression, and native Rust tests. Never reintroduce macOS Keychain access for app session or local history encryption keys.
