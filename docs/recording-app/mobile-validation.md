# Mobile validation record

This file is the evidence log for the mobile speaking practice release. A passing TypeScript check or web export does not establish native microphone, codec, file-system, battery, or background behavior.

## Current implementation evidence

Recorded on 2026-09-08 in the repository checkout:

| Check | Result | Scope and limit |
| --- | --- | --- |
| `cargo fmt --all -- --check` | Passed | Rust formatting only |
| `cargo test --lib` | Passed: 40 tests | Quota and media behavior still need physical-device evidence; the existing OIDC integration tests require a socket-capable environment |
| `cargo test` | Passed: 40 library, 3 OIDC, 2 protocol, and 14 spontaneous integration tests | Full Rust suite passed with socket access; this is still not native device evidence |
| `cargo check --bins` | Passed | Includes `practice_worker` and both existing binaries |
| `npm run typecheck` | Passed | Expo TypeScript graph only |
| `npm run export:web` | Passed | Web bundles and static routes including `/logs` only |
| `scripts/build-install-practice-android.sh --build --phone` | Passed | Generated the Android project with OOM limits, assembled a 48,485,364-byte arm64 standalone release APK, installed it with ADB, and launched the app on `192.168.1.115:34335`; SHA-256 `a1f32a3763f9cbc8a92ff481c7bb85c7ceec919ab64f687635491a8470df0522`; bundled build date `2026-09-07T23:03:06Z` |
| Final secure-storage error build | Built; install blocked by device disconnect | Assembled a 48,485,512-byte arm64 standalone release APK; SHA-256 `d8e74587743d82a95b042ace63c2c3f8155f9b91808eb83a744f87b45dba43e0`; bundled build date `2026-09-07T23:15:40Z`. The ADB push returned `EOF`, then the phone stopped accepting wireless ADB connections; the preceding corrected APK remained installed and running |
| `scripts/configure-practice-pocket-id.sh` | Passed | Idempotently found the production API resource, updated the native client, and verified its user delegated permission grant without printing the admin key |
| Corrected Android launch smoke test | Passed with device limit | `am start -W` returned `Status: ok` for `MainActivity`, and the APK bundled the production Pocket ID URLs, client ID, callback, and resource. The phone was locked behind its six digit PIN during UI inspection, so Settings taps and microphone capture were not exercised |
| Pocket ID native OAuth configuration | Passed | Live admin API verified public PKCE client `aprendiendo-practice-mobile`, callback `aprendiendo-practice-mvp://oauth/callback`, unrestricted user groups, and user delegated `learning:access` for `https://mars.timdumol.com` |
| Pocket ID OAuth endpoint smoke test | Passed | Registered authorize request returned the Pocket ID interaction redirect; an intentionally invalid code at `/api/oidc/token` returned `invalid_grant`, confirming the client and token endpoint were recognized without authenticating a user |
| Production release `aprendiendo-mcp:release-20260907231058` | Passed | Deployed to `https://mars.timdumol.com` with the MCP service, practice worker, Caddy, Gemini configuration, and shared media/database volume |
| Production health/readiness and OIDC metadata | Passed | `/health`, `/ready`, protected-resource metadata, Pocket ID discovery, and unauthenticated MCP challenge passed through TLS |
| Production practice API route protection | Passed | Upload, analysis, job, and recording endpoints returned `401` without a bearer token after the latest release |
| Production worker process | Passed | `app-practice-worker-1` runs `/usr/local/bin/practice_worker`, remained running with zero restarts; the MCP process stabilized after two startup retries while OIDC JWKS connectivity came up |
| Native iOS recording | Not run | Requires a development build and a physical or simulator microphone test |
| Native Android recording | Not run | Requires a development build and a physical or emulator microphone test |
| Authenticated durable upload/job round trip | Not run | Requires a deployed TLS endpoint, learner credential, worker, and a stub or approved provider |
| Gemini audio-only request | Not run | No paid provider request was made |
| Gemini image + audio request | Not run | Multimodal request construction and provider response have unit coverage; no paid run was made |

The bundled task visuals are project-generated practice assets. Their provenance and checksums are recorded in [`apps/practice/assets/tasks/README.md`](../../apps/practice/assets/tasks/README.md). They are not official DELE photographs.

## Device session sheet

Complete one row per release candidate and attach the build identifier to the review artifact.

| Field | iOS | Android |
| --- | --- | --- |
| Device and OS | Pending | CPH2651 / Android 16 (API 36), arm64-v8a |
| Expo SDK / native build | Expo 57 / pending dev build | Expo 57 / standalone release APK, version `0.1.0` / `versionCode=1` |
| Requested container and codec | AAC-LC/M4A, mono, 64 kb/s | AAC-LC/M4A, mono, 64 kb/s |
| Actual container, codec, channels, sample rate, bitrate | Pending probe | Pending probe |
| File size for 1, 3, 5, and 9 minute takes | Pending | Pending |
| Cold start and seek | Pending | Pending |
| Battery for one 4–3–2 session | Pending | Pending |
| Permission denial and retry | Pending | Pending |
| Call, lock, background, and media-service reset | Pending | Pending |
| Bluetooth disconnect | Pending | Pending |
| Low disk and quota eviction | Pending | Pending |
| Export and deletion | Pending | Pending |

## Required scenarios

Run these scenarios with airplane mode first, then repeat with a reachable authenticated service:

1. Open each mode, complete a short take, force-close after Save, reopen, seek, play, and confirm the task snapshot and metadata remain.
2. Complete 4–3–2 with automatic 240/180/120 second stops in an instrumented build or with shortened test durations. Stop early once, confirm the round is saved and the next round does not start automatically. Confirm feedback remains hidden until all three rounds are saved.
3. Start image description, verify the image stays visible during recording, edit notes, use the preparation timer, save the three-minute take, and confirm the image is attached to the durable request when online.
4. Deny microphone permission, disconnect Bluetooth, lock/background the app, trigger a phone call or media-service reset, and press Stop twice. A partial finalized take must be labelled interrupted; an unfinalized candidate must not be labelled Saved.
5. Set a tiny test quota, record A/B/C, replay A, then record D. Confirm eviction follows immutable creation order rather than playback order. Confirm protected or pending audio blocks admission with an action message.
6. Queue coaching offline, restart the app, restore connectivity, and confirm the outbox reuses the same idempotency keys and eventually marks the job completed. Delete a recording while queued and confirm the local file and remote job are cancelled/deleted safely.
7. Export a run, inspect the metadata and audio files, delete the run, restart, and confirm playback is disabled for removed media and no managed audio remains.

## Backend release evidence

Before exposing the authenticated service, record the deployment identifier, database backup and restore result, migration result, TLS endpoint, worker memory/CPU/disk limits, cleanup metrics, server audio cap, and the seven-day abandoned-media cleanup result. Logs must contain request/job identifiers and sanitized status or provider metadata only; never log audio bytes, transcripts, task text, bearer tokens, or API keys.

The release command specified by the repository instructions is `scripts/release.sh`. Inspect it before using it and add the mobile/backend build and worker checks deliberately; this validation record must contain the resulting release identifier.
