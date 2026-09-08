# Mobile speaking practice: implementation plan

Date: 2026-09-08. Status: implementation foundation complete; the production API/worker is deployed and an Android release APK has been built, installed, and launched on a physical device. Native recording and authenticated-provider evidence remains pending. This document remains the behavior and release-gate contract for the mobile implementation.

## Outcome and scope

Extend `apps/practice/` into an iOS and Android app for everyday Spanish speaking practice: open a task, record reliably offline, replay later, inspect useful delivery measurements, request Gemini coaching, and repeat. Ship free speaking, 4–3–2 practice, and DELE A2 oral task 2 in the first complete mobile release.

This plan controls the next implementation where it differs from the earlier [product](./product.md), [backend](./backend.md), and [assessment](./assessment-and-delivery.md) designs. In particular: local audio uses a byte budget instead of mandatory 90-day retention; 4–3–2 and image description are required now; keep the successful Gemini adapter rather than requiring a multi-provider assessment pipeline. Shared MCP history integration remains a later milestone. Preserve the browser MVP while extracting reusable components.

Defaults below are implementation choices, configurable where stated:

| Area | First mobile release |
| --- | --- |
| Platforms | Native iOS and Android; retain the existing web experiment |
| Persistence | Local SQLite metadata and private durable audio files |
| Audio | Evaluate mono Opus at 32 kb/s first; AAC-LC/M4A at 64 kb/s is the compatibility fallback |
| Retention | Newest audio within N decimal GB per device; suggested N = 1 |
| Offline | All bundled tasks, recording, history and playback; cloud analysis queues until connected |
| Analysis | Deterministic signal/VAD measurements plus optional paid Gemini coaching |
| Capture | Foreground, continuous takes; finalized partial recordings on interruption where possible |
| Sync | Durable assessment jobs; full cloud archive and cross-device history deferred |
| Progress | Practice history and qualified within-task comparisons; no CEFR prediction or global fluency score |

Done means a real phone can complete all three modes, survive restart and network failure without losing finalized takes, enforce N GB, replay measured pauses, receive actual image-aware/audio-aware coaching, and export or delete its data. Both operating systems need device evidence before claiming support.

## 1. Start from the implementation that exists

Read root `AGENTS.md`, inspect `git status --short`, and preserve existing changes. Do not scaffold over the Expo app. The inspected checkout already has Expo 57, Expo Router, `expo-audio`, recording/player/feedback components, and the separate Rust `practice_mvp` binary. Its artifact type requires a `Blob`, its recorder calls `fetch(uri).blob()`, its two attempts are in memory, and its API is synchronous and intended for loopback use. These are concrete replacement boundaries.

Keep the validated Gemini prompt/parser and bounded feedback behavior. Move reusable provider code into a shared module when the production API needs it; keep the MVP binary and routes functional. Do not expose the unauthenticated MVP listener to the Internet. New native builds use an authenticated HTTPS endpoint, not a phone's `127.0.0.1`.

Suggested file boundaries (adapt names to existing conventions):

```text
apps/practice/app/                   # Practice, session, review, history, settings routes
apps/practice/components/            # existing recorder/player/feedback, extended
apps/practice/lib/domain/            # tasks, rounds, provenance, comparison rules
apps/practice/lib/storage/           # SQLite migrations, media journal, quota, export
apps/practice/lib/media/             # native/web recorder and file adapters
apps/practice/lib/sync/              # persistent upload/assessment outbox
apps/practice/assets/tasks/          # licensed photos and versioned task manifests
src/practice/                       # authenticated API, job store, provider orchestration
src/bin/practice_worker.rs           # bounded media analysis and assessment worker
docs/recording-app/mobile-validation.md
```

Use SDK-compatible Expo packages for SQLite, file access, secure credential storage, sharing, image selection and keeping the screen awake. Check the installed SDK documentation before choosing APIs. A native development build is the acceptance environment; Expo Go or a web screenshot is insufficient.

**Completion check:** document what is reused, introduce the platform media interface, and render native Practice and History without losing the working web flow.

## 2. Make recordings durable before adding more coaching

Replace the Blob-only artifact with an immutable recording ID, relative private path, actual container/codec, byte count, hash, decoded duration when known, client duration, creation order and interruption status. Blobs/object URLs remain a web adapter detail. Never persist absolute sandbox paths, which can change across app installations/updates.

Local tables:

| Entity | Required state |
| --- | --- |
| `practice_runs` | UUID, task snapshot/version, mode, creation order, run status, assistance/reflection, feedback exposure |
| `practice_rounds` | Run ID, sequence, target duration, baseline/repetition/coached classification, completion/interruption |
| `recordings` | Round ID, path, hash, actual encoding/bytes, capture and decoded durations, media availability |
| `analyses` | Recording hash, processor/model/config versions, typed metrics/findings, status and limitations |
| `outbox` | Stable operation ID, payload hash, upload/job ID, retry state, next attempt, last sanitized error |
| `media_operations` | Pending finalizations/deletions and reserved bytes for restart reconciliation |
| `settings` | N GB, explanation language, task timing preferences, analysis consent and spending settings |

The [Expo audio documentation](https://docs.expo.dev/versions/latest/sdk/audio/) describes recording options and the default cache location. Explicitly select durable document storage or move finalized recordings there; cache-only files are not saved history. Verify supported options against the pinned SDK.

Use a journaled sequence: create draft row and byte reservation → record to a known staging path → stop/finalize → stat/probe and move atomically where supported → commit ready metadata → release reservation. Reconcile files and rows on launch. SQLite and filesystem changes are not one transaction: each intermediate state must be recoverable. Do not navigate away showing “Saved” before durable finalization succeeds.

Keep the screen awake during recording. Route changes, incoming calls, microphone revocation, audio-service reset and app backgrounding must stop/finalize when possible and mark the take interrupted; never silently resume into a timed round. A killed process can leave an unplayable M4A: preserve the candidate for recovery, explain failure honestly, and do not promise recovery of unfinalized audio. Playback restores the appropriate audio mode after capture.

Capture uses a monotonic/native elapsed time rather than counting UI timer ticks. Stop is idempotent. Timed rounds use a native duration limit where supported; reconcile the display with recorder state. Keep countdown sounds outside the analyzed recording and use visual/haptic cues while recording. Save short takes even if they are ineligible for model feedback. Default free speaking limit: 10 minutes; server limits must match, while each 4–3–2 round is a separate file.

**Completion check:** airplane-mode capture → force close after Save → reopen → seek and replay. Test low disk, denied permission, Bluetooth disconnection, phone call, lock/background and double Stop on both platforms. Existing takes survive every failure.

## 3. Encoding and the N GB retention contract

AAC is a provisional compatibility baseline, not the preferred codec on compression merit. Before locking the archive format in milestone 2, compare mono Opus at 24/32/48 kb/s against AAC-LC/M4A at 48/64 kb/s on the same source recordings. Prefer Opus at 32 kb/s if native capture, playback/seek, export and assessment quality pass; retain AAC at 64 kb/s as the fallback if the integration cost or observed reliability is materially worse. Record the decision and evidence rather than automatically deferring Opus.

The [Opus project's comparisons](https://opus-codec.org/comparison/) support evaluating it for efficient speech storage. [Android documents native Opus encoding from Android 10](https://developer.android.com/media/platform/supported-formats), but operating-system codec support is not proof that every Expo recorder/container combination works. Verify iOS and Android encoding and container support independently, including the pinned Expo APIs; a small native libopus integration is an option if required. Distinguish codec support from Ogg/WebM/CAF container support. Provider incompatibility can be handled by a temporary server-side PCM derivative, without forcing the archive to AAC.

For the AAC baseline, request mono at 44.1 or 48 kHz where supported. Do not retain PCM/WAV as the archive. Probe actual output rather than assuming the recorder honors bitrate, sample rate or channel requests. Encode comparison candidates from the same lossless fixture, not by transcoding AAC into Opus. In production prefer direct capture into the selected codec; account for any temporary PCM in the storage reservation if native integration requires it. Retain browser WebM/Opus in its original format.

At 64 kb/s, payload arithmetic gives about 0.48 MB/minute, 4.32 MB for nine minutes, and 34.7 hours per decimal GB. At 32 kb/s, these become 0.24 MB/minute, 2.16 MB and 69.4 hours. Figures exclude container overhead and variable-bitrate variation. These are estimates; enforce limits using actual file sizes. Avoid repeated lossy transcoding. Decode bounded chunks for analysis and remove temporary PCM after use.

Define `quota_bytes = floor(N * 1_000_000_000)`, validate finite positive N, and expose presets plus custom entry. The budget covers all app-owned local audio, including staging files, upload copies and playback caches. Store no duplicate audio for an assessment. Database, images and compact analysis files have separate bounded caches and appear separately in Storage settings.

Retention algorithm:

1. Serialize quota admission and reserve a conservative maximum for the next take, allowing container overhead and measured device bitrate variance. Check OS free space independently, with a safety reserve (initially 100 MB; validate on devices).
2. If needed, evict finalized recordings oldest-first by immutable local creation sequence, with ID as tie-breaker. Playback does not make a file “recent.” Delete enough eligible files that actual usage plus reservations fits. No pinning in v1: export favorites outside the managed library.
3. Protect active recording/playback, unfinished current exercise rounds, and pending uploads. Protected bytes still count. If they prevent admission, block the new recording with the required space and actions to finish/cancel processing, export/delete, or increase N. Never silently delete an unsent assessment request's only input.
4. Record a pending deletion, unlink the exact managed file, then mark audio evicted. Recover safely after crashes at either boundary. Do not decrement physical usage until deletion succeeds. Retain task, transcript, feedback and metrics with “Audio removed by storage limit”; disable replay and reassessment needing missing audio.
5. Reconcile after finalization, upload completion, startup and a quota change. If actual recording growth threatens the reservation, stop/finalize before exceeding the admitted budget; if the platform cannot provide adequate control, increase the conservative reservation before enabling that preset.

Show “Keep up to N GB of recent audio; oldest eligible audio is removed automatically” during setup. Reducing N previews affected recordings and requires applying the new setting; if protected media prevents the reduction, keep the previous effective limit. Exported files and OS backups are outside the managed quota and must be explained. Exclude audio from automatic device cloud backup where feasible and verified; otherwise disclose backup behavior. Uninstall can remove the local library: this version is not a cloud archive.

Budget is per device. Server uploads are temporary processing inputs, deleted within 24 hours after terminal completion, with a seven-day absolute TTL for abandoned/failed work and a separate server byte cap. Active jobs have bounded deadlines within that TTL. Server responses state expiry; retries after expiry require re-upload. There is no silent permanent server audio copy. Future cloud retention needs its own explicit quota and deletion semantics.

**Completion check:** with a tiny test budget, record A/B/C, replay A, then add D: eviction follows creation order, not access order. Check reserved/protected bytes, interrupted finalization, failed unlink, oversized import and reducing N. After restart, filesystem usage and database accounting agree.

## 4. One exercise engine, three required modes

Represent a task as a versioned immutable snapshot: mode, Spanish instructions, assets and hashes, preparation policy, ordered round durations, feedback-release policy and rubric version. Persist every transition: `preparing → ready → recording → finalizing → round_saved → next_round/completed`. A failed/interrupted take does not advance automatically. Resume opens the saved boundary, never starts the microphone automatically.

### Free speaking and coached retry

Keep own topic and the three existing topics. Add a small offline deck covering routines, recent events, plans, explaining a problem and giving an opinion. Suggested duration is 1–2 minutes. Let the learner listen, optionally note one difficulty, request coaching and make a new linked attempt. Support more than two takes without replacing old audio. Same-task repetition remains repetition even before feedback; record actual feedback exposure separately.

### 4–3–2 practice

Choose one topic, optionally plan for 60 seconds, then deliver the same message in four, three and two minutes. The app's individual practice adaptation uses three recordings, with a suggested 30-second rest and an explicit Start next round. Freeze the topic across all rounds. Default to automatic stop at each target; early stop is allowed and recorded. A gentler 3–2–1 preset is labeled as a variant.

Withhold transcripts, coaching and metric results until all three rounds finish or the learner explicitly ends the exercise early. Local measurements or queued work may run, but cannot leak feedback into the next round. If the learner opens results early, mark later rounds as coached. Basic saved/playable status can be shown between rounds without presenting corrections.

Afterward show three players, actual/target durations and comparable pause metrics per minute. Ask whether the key message survived compression. Optional AI comparison must cite each referenced round, assess retained content and intelligibility, and never call faster speech automatically better. Keep per-round independent assessments unchanged. An incomplete run is still retained; restarting a round creates a new attempt with lineage.

### DELE A2 oral task 2: image description

The current [Instituto Cervantes A2 guide, pages 17–18](https://examenes.cervantes.es/sites/default/files/DELE_A2_v2020_Gu%C3%ADa_de_examen_0.pdf) specifies a prepared 2–3 minute description of an everyday photograph. The 12-minute preparation allowance belongs to the whole oral test, not this task alone. Task 3 is a separate 3–4 minute simulated conversation based on the photograph.

Ship 12–20 locally bundled, clearly licensed photographs of practical everyday scenes, each with provenance and manually reviewed Spanish prompts. Do not redistribute exam PDFs or photographs without appropriate permission. Offer a user-selected image too; resize to a bounded upload resolution, strip location metadata, and freeze the chosen image for all attempts.

Provide two settings: guided practice with optional prompts and a configurable preparation timer (default two minutes, explicitly an app practice choice), and task simulation with task instructions/photo, a 2-minute cue and a 3-minute stop. This is task-2 practice, not a complete official exam simulation. Notes are allowed, persisted as assistance metadata, and never mistaken for spoken words.

Original practice prompts can cover place, people, clothing/appearance, positions, actions and relevant scene details. Keep the image visible and zoomable while recording. Do not show a model answer beforehand. Afterward offer a coverage checklist and one useful retry target.

Image-related coaching must receive the actual image together with audio/task data through a verified multimodal provider contract. Test this extension separately from the successful audio-only adapter. Do not claim visual correctness from transcript alone; if image processing fails, publish general language/audio feedback with visual coverage unavailable. Treat ambiguous details and plausible qualified guesses cautiously. No predicted DELE pass/fail score.

**Completion check:** complete and restart each mode offline, preserving task/assets and sequence. Verify 240/180/120-second stops, early finish, interrupted rounds, feedback gating and the task-2 timer/visible image on real phones.

## 5. Algorithmic delivery analysis

Implement a versioned deterministic worker stage independently of Gemini. Initial placement is the bounded server worker; offline recordings display “Analysis pending connection.” This first release does not promise offline acoustic analysis. Once the contract and fixtures are stable, evaluate running the same VAD locally with an ONNX native module; avoid shipping two disagreeing algorithms without provenance.

Decode once into bounded mono 16 kHz PCM for analysis, preserving a mapping to original playback time, including codec priming/resampling offsets. Prototype pinned [Silero VAD](https://github.com/snakers4/silero-vad), which supports 8/16 kHz inputs, via ONNX runtime. Keep the model artifact checksum, license and exact processor configuration. Runtime integration and thresholds need a spike; use a subprocess worker if necessary rather than complicating the capture app.

Persist speech intervals and derive non-speech gaps between adjacent intervals. Starting parameters for evaluation: probability threshold 0.5, minimum speech 150 ms, minimum silence 200 ms; validate windowing/hysteresis supported by the pinned model. Calculate metrics on documented unpadded intervals; padding is only for replay. These parameters and the following 500 ms / 2 s thresholds are product hypotheses, not universal language standards.

| Metric | Definition and display rule |
| --- | --- |
| Recording duration | Decoded sample count / sample rate; keep client elapsed separately |
| Response span | First speech onset to last speech offset; undefined when no speech is detected |
| Speech-active duration | Sum of VAD speech intervals; label as detector estimate |
| Internal pauses | Gaps ≥500 ms between speech intervals; exclude leading/trailing silence |
| Long pauses | Subset of internal gaps ≥2 seconds |
| Pause burden | Sum of qualifying internal gaps / response span; null for zero span |
| Pause frequency | Internal pause count / response-span minutes |
| Typical/longest pause | Median and maximum qualifying gap; absent if there are no qualifying gaps |
| Speech rate, optional | Transcript words / response-span minutes, including internal pauses; requires usable transcript |
| Words per speech-active minute, optional | Transcript words / VAD speech minutes; explicitly a hybrid estimate, not a syllabic articulation-rate score |

VAD distinguishes likely speech from non-speech, not thinking from breathing. UI copy says “Detected pauses,” never a measured hesitation diagnosis. Quiet speech, background speakers, music and noise can corrupt results. Start with three visible values (pause count, long-pause count, pause time), an expandable method/limitations panel, and a compact timeline with tappable spans. Replay with roughly 0.5 seconds of speech context on either side. Silence produces “No reliable speech detected,” not zero pauses interpreted as perfect delivery.

Separate transcript-derived candidates: fillers such as `eh`/`em`, repeated words and false starts. Require a disfluency-preserving transcript and trustworthy alignment for localized counts. `Bueno`, `pues` and `o sea` are not automatically mistakes. Gemini text may omit or normalize these; with the current unaligned transcript, keep qualitative comments and do not display exact filler counts. Add an independently evaluated ASR/alignment stage only when needed to unlock these metrics. Never derive pause times from punctuation.

Signal checks include near-silence, clipped-sample fraction and gross input-level anomalies; label them indicators, not a precise SNR estimate. Analysis never trims the archived original. Findings carry recording hash, processor/model version, units, threshold configuration and evidence method. Preserve approximate Gemini moments as approximate; they cannot become measured spans merely by falling inside the recording.

**Completion check:** annotate at least 20 consented/owned Spanish clips, including quiet speech, normal phrase breaks, restarts, fillers, noise, silence and Bluetooth capture. Proposed release gates: internal gaps ≥500 ms have precision and recall ≥0.90 with 200 ms boundary tolerance on clean fixtures; report noisy results separately and suppress unreliable outputs. Inspect each failed case. Compare repeated runs for deterministic output and test playback localization after AAC decoding. Do not tune and report only on the same clips: hold out a subset.

## 6. Turn the successful feedback call into reliable mobile processing

Add authenticated HTTPS app routes backed by durable jobs and streaming uploads, reusing existing Rust conventions. Native login uses system-browser authorization with PKCE and secure token storage; inspect existing auth before adding a separate flow. Check resource ownership on every recording, job and deletion. Keep API keys server-side. Preserve existing MCP request limits; media decoding runs in a separately limited worker.

Minimum API contract:

| Operation | Contract |
| --- | --- |
| Create upload | Client recording UUID/hash/size/actual MIME; idempotency key; returns upload ID and limits |
| Upload/finalize | Bounded streamed file, verified size/hash/container/duration; retry same upload without duplicate records |
| Request analysis | Recording + immutable task/image hashes + processor versions; explicit requested stages and spending reservation |
| Read job | Durable queued/running/partial/ready/failed status and versioned results; resume polling on foreground |
| Cancel/delete | Tombstone and stop publication; idempotent cleanup of staged input/results |

Implement named routes and matching generated Rust/TypeScript schemas in this milestone. Bound audio to 10 minutes and 10 MiB initially, image uploads to 5 MiB plus decoded pixel limits, and reject decompression bombs by decoded duration/dimensions. Revisit limits only with device evidence. Preserve originals locally when the server rejects media. Poll with backoff while foregrounded; resume uploads/jobs after restart. Do not claim background execution guarantees from mobile JavaScript timers.

Worker stages: validate/decode → deterministic measurements → optional Gemini audio/image coaching → validated result publication. Stage failures retain successful results. No cloud call is required to open a task or record. The learner explicitly requests paid analysis, with a visible estimate and server-enforced configurable monthly cap; 4–3–2 shows the total for the selected rounds. Analysis settings and model/prompt versions are stored with every result.

Use unique constraints for jobs, leases for workers, payload-hash conflicts for idempotency misuse and cancellation checks before publication. Persist operation state before network calls. A timeout after provider acceptance has unknown billing: mark that state and require an explicit paid retry unless the provider supports verified idempotency/retrieval. HTTP/client retry cannot guarantee exactly-once model charging. Bound concurrency, output tokens and cost reservations server-side.

Extend the feedback contract additively with task coverage, typed measured evidence and coverage/limitations. Continue at most two strengths and two improvements. Audio coaching receives actual audio; visual coaching actual image. A retry comparison is optional and cannot block individual results. Results download into local SQLite before the upload is unprotected from retention.

No automatic FSRS reviews or learning-store writes in this release. Later projection must follow the existing evidence, dispute and idempotency rules in [backend](./backend.md). Transcript corrections preserve the original revision, invalidate dependent findings/counts, and offer reassessment; they do not change the audio.

**Completion check:** expired credentials, airplane mode, double submission, crash/restart, worker death, partial analysis, provider timeout and deletion during processing preserve the correct recording/results. No duplicate job or resurrected deleted take. Exercise the actual Gemini audio+image contract on explicitly selected test media.

## 7. Make daily use comfortable

Practice opens on three clear choices: Free speaking, 4–3–2, Describe a photo; show Resume when a run is unfinished. History groups rounds and retries, searches topics/dates and displays saved/processing/audio-expired status. Feedback and recordings remain readable offline once downloaded.

Player essentials: seek, skip back five seconds, adjustable replay speed, repeat a selected excerpt, and paired take switching. Metric comparison requires matching task mode, processor settings and usable audio; show raw denominators, and avoid presenting shorter 4–3–2 takes as deterioration because word totals fell. Across days, offer a fresh prompt on the same communicative theme; label it separately from same-task improvement.

Settings exposes actual audio usage, N GB, approximate hours, pending protected bytes, explanation language, connectivity preferences, API spend, export and deletion. Export a selected recording or whole run with audio, task/image provenance, transcript, metrics and feedback JSON in a shareable package. Distinguish “Remove audio” from “Delete practice”; whole-run deletion removes metadata/results and queues server tombstones. Explain when server deletion is pending offline. Pause new queued submissions immediately when a run is deleted.

Support screen readers, large text, 44-point or larger controls, sufficient contrast and visual equivalents for sounds/haptics. Do not announce the timer every second. Describe-photo accessibility needs care: an optional image description helps access but also supplies linguistic/content assistance, so mark that practice accordingly rather than silently scoring it as an unaided attempt.

Useful follow-ups after the required release, in order: a one-tap fresh transfer task after a few days; DELE task 1 prepared monologues; task 3 turn-based role-play using the same scene; user-selected excerpt shadowing with separate imitation provenance. Full exam simulation, live voice conversation, push reminders, cloud archive/sync and automatic pronunciation scoring are separate projects.

## 8. Delivery sequence and release gates

Each milestone should be a reviewable change with an end-to-end demonstration. Do not build all infrastructure before testing native capture.

1. **Native foundation:** device builds, media adapters, durable SQLite/files, history/player and journal recovery. Gate: capture/restart/replay on iOS and Android.
2. **Bounded storage:** Opus/AAC device comparison and selected encoding preset, quota reservations/eviction, export/delete and storage screen. Gate: tiny-budget and low-disk tests, actual output quality/size report.
3. **Practice modes:** persistent rounds, timers, feedback exposure, bundled image tasks. Gate: all three modes usable offline, interruption/resume and 4–3–2 withholding verified.
4. **Reliable online processing:** authenticated endpoint, durable jobs/outbox, temporary server retention, Gemini reuse and image input. Gate: real device submits/retrieves feedback across a restart; failure and cost controls pass.
5. **Delivery analysis:** VAD, metric definitions, measured replay, quality flags and comparisons. Gate: annotated/held-out fixture results and honest limitations; no invented hesitation measurements.
6. **Daily-use beta:** accessibility, transcript correction, deletion races, diagnostics, export/restore validation and distribution. Gate: seven days of ordinary use on physical devices, including offline sessions and deliberate failures, with no lost finalized recordings.

Required automated checks focus on durable invariants: quota ordering/accounting, journal reconciliation, timer/round transitions, immutable snapshots and assistance labels, idempotent uploads, owner authorization, deletion races, metric formulas/no-speech behavior, timestamp bounds and schema validation. Use a stub provider in routine tests. Run Rust formatting/tests, frontend typecheck and web export; add native build checks appropriate to the new modules. Do not equate a passing build with recording verification.

In `mobile-validation.md`, record device/OS/build versions, requested versus actual codec/bitrate/channels, file size per minute, cold-start and seek behavior, battery use for a 4–3–2 session, upload/processing latency, API usage, metric fixture outcomes, privacy/retention settings and untested cases. Backend deployment requires separate measured memory/CPU/disk limits, authenticated TLS, migration backup/restore, cleanup monitoring and sanitized logs without audio/transcripts/tokens. Device export is the initial user backup path; test re-import or document a portable export format before promising restore.

Distribute signed beta builds through the appropriate iOS/Android channels before store release. Validate signing, microphone permission text, privacy disclosures and release-build behavior. Repository releases use `scripts/release.sh` per `AGENTS.md`; inspect it before use and add required mobile/backend steps deliberately. The current release has been deployed, while no paid provider request was made during validation.

### Build, install, and release commands

From the repository root, install the Expo dependencies and build the Android
client with the standalone APK workflow:

```sh
(cd apps/practice && npm install)
scripts/build-install-practice-android.sh --build --phone
```

The script follows the device-selection and install behavior used by
`../fitsync/scripts`: `--phone` filters out emulators, `--device SERIAL`
selects a target, `--debug` selects a debug APK, `--release` selects the
standalone release APK, `--apk PATH` overrides the artifact, and `--api-url`
sets the production or staging API bundled into the app. The default build
limits are one Gradle worker, one CMake/Ninja compile job, a 2048 MB Gradle
heap, a 1024 MB metaspace limit, disabled Gradle parallelism, and the ABI of
the selected target. Override those values only when the build host has the
required memory with `PRACTICE_GRADLE_MAX_WORKERS`,
`PRACTICE_NATIVE_COMPILE_JOBS`, `PRACTICE_GRADLE_HEAP`,
`PRACTICE_GRADLE_METASPACE`, and `PRACTICE_ANDROID_ARCHITECTURES`.

The production API and worker use the repository release path:

```sh
scripts/release.sh
```

The release script builds and transfers the image, deploys the MCP service and
practice worker with the production Compose profile, and checks health,
readiness, OAuth metadata, and the unauthenticated challenge. It reads
`apps/practice/.env.mcp` for `GEMINI_API_KEY` and `GEMINI_MODEL`; set
`APRENDIENDO_PRACTICE_ENV_FILE` to use another ignored env file. Provider keys
remain server-side and must never be bundled in an `EXPO_PUBLIC_*` variable.
Pocket ID is configured with the public PKCE client `aprendiendo-practice-mobile`,
callback `aprendiendo-practice-mvp://oauth/callback`, resource
`https://mars.timdumol.com`, and user delegated `learning:access` access. The
client uses issuer `https://auth.aries.timdumol.com`, authorization endpoint
`/authorize`, and token endpoint `/api/oidc/token`.

The idempotent Pocket ID setup command is:

```sh
scripts/configure-practice-pocket-id.sh
```

It reads the admin key from the ignored root file `.pocket-id-api-key` and
prints only the resulting client/resource summaries.

The production release has been deployed per this procedure. No paid Gemini
request was made during validation; the remaining device gate is actual
microphone capture, interruption/recovery, authentication, upload, and job
completion evidence on physical devices.

## Copyable implementation instruction

> Implement `docs/recording-app/mobile-plan.md` in milestone order, extending the successful Expo/Rust MVP. First prove durable native capture and bounded newest-audio retention, then add free speaking, 4–3–2 and DELE A2 image description. Reuse Gemini behind authenticated durable jobs, and implement versioned measured pause analysis separately from model interpretations. Preserve existing work and the web MVP. Complete each milestone's checks, document actual device/codec/analysis results, and never claim offline analysis, exact hesitation detection, native reliability or exam scoring without the corresponding evidence.
