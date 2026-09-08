# Aprendiendo recording app: implementation handoff

Design date: 2026-09-07. Status: MVP successful; mobile implementation foundation present, physical-device validation pending.

**Next: [Mobile implementation plan](./mobile-plan.md).** Extends the successful MVP with durable native recording, configurable N GB audio retention, measured pause analysis, 4–3–2 practice and DELE A2 image description. Its scope and milestone order control the next implementation; the documents below retain the broader design context.

**For the immediate UX/model experiment, start with the [small MVP implementation plan](./mvp-plan.md).** It intentionally narrows the production design below to a local Expo web app, one Gemini audio call, and an in-session retry. It includes ordered steps, completion checks, and a copyable instruction for a smaller coding LLM.

## Product decision

Build a self-contained Spanish speaking practice app: **record → replay → receive evidence-linked feedback → try again**. Use Expo Router for iOS, Android, and desktop web, backed by the existing Rust server. The app sends recordings to model APIs through the server. A ChatGPT subscription is not part of this runtime; there is no required ChatGPT handoff and no dependency on ChatGPT being able to consume audio over MCP.

The first useful release must process actual audio, not merely present a transcription as a speaking assessment. Its value is a reliable record of what the learner said, a small amount of actionable feedback, and direct comparison with a subsequent attempt. Keep the existing MCP integration available for optional access to learning history.

## Read in this order

1. [Product and UX](./product.md): screens, example session, recording behavior, feedback, failure states.
2. [Backend and API](./backend.md): Rust integration, persistence, HTTP contracts, jobs, authentication, retention.
3. [Assessment and delivery](./assessment-and-delivery.md): model inputs and outputs, evidence rules, evaluation gates, implementation sequence.
4. [Models and costs](./models-and-costs.md): concrete evaluation candidates, disfluency evidence, and approximate costs for 4–3–2 recordings.

These files specify behavior and architectural contracts. They are not executable migrations or an OpenAPI specification. The implementation should generate the latter from its typed request/response models.

## Scope and defaults

| Decision | First release |
| --- | --- |
| Primary activity | A short voice diary or a response to one communicative prompt |
| Length | Suggested 60–120 seconds; allow 10 seconds to 5 minutes |
| Feedback | Task response, up to two strengths and two improvements, linked to evidence |
| Audio feedback | Intelligibility and delivery only where supported by the recording; no overall pronunciation score |
| Retry | One optional coached retry of the same task, preserved separately |
| Progress | Session history and first-attempt/retry comparison; no overall CEFR level or mastery percentage |
| Provider | Evaluate OpenAI, Gemini and Scribe directly using the [model shortlist](./models-and-costs.md); pin tested versions where available |
| Backend | Axum + existing SQLite services; bounded durable worker jobs; private audio files |
| Frontend | Expo Router, `expo-audio`; shared mobile/web core with platform-specific media and auth adapters |
| Interaction | Foreground recording, asynchronous feedback after submission, no live conversation |
| Cost | Separately billed API use; visible monthly usage and configurable server-enforced cap |
| Language | Spanish task prompts, English explanations by default; persistent explanation-language preference |

Deferred: live speech-to-speech chat, a general chat screen, background recording, push notifications, Anki, tutor sharing, expression inbox, and large progress dashboards. Timed 4–3–2 rounds and delayed transfer tasks are the next increments after the short recording loop works. Offline capture is supported as a recoverable local draft; offline AI feedback is not.

## Existing foundations and required changes

Repository inspection found Axum routes in [main.rs](../../src/main.rs), the MCP adapter in [server.rs](../../src/server.rs), and `LearningStore` / `SqliteStore` in [db.rs](../../src/db.rs). The server already supports `voice_diary`, `spoken_transcript`, measured durations, attempts, observations, durable practice preferences, prompt validation, and conservative FSRS eligibility. See [current implementation notes](../../SPONTANEOUS_PRODUCTION_IMPLEMENTATION.md).

The implemented MVP now provides an Expo frontend and a separate synchronous localhost feedback API. The mobile foundation adds private local media, SQLite journals, an authenticated upload/job API, a separately limited worker binary, an outbox, and bounded delivery analysis. The current small MCP request limits and 96 MiB container profile remain unsuitable for buffering or decoding audio. Preserve MCP limits and use shared domain services rather than making the app call MCP internally.

The current recording operation atomically creates a complete session; it is not a mutable draft API. New app draft/run tables therefore own recording and processing state. A validated assessment is projected into the learning history exactly once, with explicit provenance and links back to its audio evidence. Do not repeatedly resubmit changed payloads under the existing idempotency key.

## Definition of a successful first release

A learner on a phone or desktop can record a 90-second answer, survive an upload or processing failure without rerecording, receive useful feedback tied to replayable evidence, record a coached retry, compare both, and return later to the same artifacts. No ChatGPT window is involved. No model can silently overwrite an original answer, invent timing measurements, or turn a coached retry into independent mastery evidence.

## Sources and decisions to verify during implementation

Official documentation checked on the design date:

- [OpenAI audio guide](https://developers.openai.com/api/docs/guides/audio): transcription and audio-input API capabilities. Use an audio-capable endpoint for actual listening; do not assume every text endpoint or model supports audio or strict structured outputs.
- [Expo audio](https://docs.expo.dev/versions/latest/sdk/audio/): recording/playback, permissions, platform behavior, and persistence considerations. Browser capture requires a secure context; codec and interruption behavior require device testing.
- [Expo web](https://docs.expo.dev/workflow/web/): desktop browser target and shared components.

The model pair, accepted codec matrix, timestamp alignment method, latency targets, and per-session costs must be measured in milestone 0. These are explicit release gates, not reasons to defer the rest of the design. Provider audio support is not evidence that its Spanish pronunciation judgments are reliable.
