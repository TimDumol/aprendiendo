# Assessment pipeline and implementation plan

[Handoff index](./README.md) · [Product](./product.md) · [Backend](./backend.md)

## Assessment contract

Use a bounded pipeline rather than a free-running agent. The coordinator owns task selection, persistence, retry rules and evidence publication. Evaluate direct provider APIs using the [concrete model shortlist and cost estimates](./models-and-costs.md), with independently configured model IDs for transcription, direct audio assessment, and text feedback. Pin tested versions where available and record the exact returned model/version. Do not select a model merely because its name says “audio”; verify the endpoint, accepted duration/format, timestamp capabilities, and output schema behavior against current official documentation during milestone 0.

No custom ChatGPT integration is required. The server sends actual recording bytes to an audio-input API and stores results; the app renders those results directly. A transcript-only fallback must be visibly marked as such. The documented [OpenAI audio workflows](https://developers.openai.com/api/docs/guides/audio) establish input mechanisms, not assessment accuracy.

### Pipeline

1. **Validate media.** Verify receipt hash, decode limits and duration. Record format/sample information and interruption metadata. Run a bounded signal-quality check; reject undecodable media, and flag silence/noise without assigning language errors. Use decoded duration as playback timeline; retain client elapsed time separately.
2. **Transcribe.** Request Spanish transcription that preserves errors, restarts and incomplete phrases where supported. Do not send a target answer or grammar correction examples to the transcriber. Store raw response/text and any supported timestamps/uncertainty. ASR can normalize learner errors; this limitation must be evaluated explicitly.
3. **Listen.** Send actual audio, the task, and a narrow rubric to the audio-capable model. Ask for supported observations about intelligibility and delivery with evidence. Do not give it the transcript on its first pass, to reduce anchoring. Timing statistics come from a signal processor, not model guesses. Exact spans are optional until verified.
4. **Assess language.** Give the text model the task, versioned transcript, private optional target context, and relevant assistance metadata. Ask for task completion, supported strengths/errors, and minimal corrections. Preserve valid alternatives; distinguish unused target from failed communication.
5. **Reconcile.** Combine language and audio candidates. Contradictory transcriptions invalidate findings that depend on disputed words. Check quoted substrings, taxonomy references, evidence bounds and source types. Suppress unsupported findings rather than asking a model to invent citations. Produce at most two strengths and two improvements total.
6. **Publish.** Save versioned feedback, then the validated learning projection. Mark ready only when publication is recoverable and public results are schema-valid. Audio-stage failure may publish a partial language assessment; failed publication is a separate retryable stage.
7. **Compare on retry.** Generate a small comparison using both assessments and their evidence, only after both current versions exist. It is optional output: failure must not hide either attempt's feedback. Any new comparison claim must cite both sides, or explicitly say that a feature was only observed on one side.

Stages 2 and 3 may execute concurrently. Stage 4 depends on transcription; reconciliation depends on available candidates. Avoid resending audio to a text model when it cannot consume it. Use a schema-constrained text formatter only if the audio endpoint lacks the required structured-output support; formatting cannot add assessment facts.

## Rubric and evidence rules

| Dimension | Evidence needed | Permitted feedback | Do not infer |
| --- | --- | --- | --- |
| Task completion | Task + usable transcript/audio | Which requested ideas were conveyed, with examples | Failure because an optional target was unused |
| Grammar/vocabulary | Reliable original wording | Specific issue, minimal correction, valid alternatives | Errors from uncertain ASR text |
| Intelligibility | Actual audio | A specific stretch was easy/hard to understand, with qualification | A pronunciation diagnosis from a transcript |
| Delivery | Actual audio; measured statistics if cited | Supported restarts, phrasing or audible interruptions | Pause duration from punctuation |
| Pronunciation | Actual audio and sufficiently reliable localized evidence | Cautious, actionable observation if evaluation supports it | Phoneme scores, native-likeness or overall proficiency |
| Improvement | Two referenced attempts and known assistance context | A concrete change in this retry | General mastery from same-task repetition |

No aggregate numeric grade in v1. Do not conflate accent with error. If evaluators cannot reliably distinguish an accent difference, noise, ASR normalization, and a learner mistake, suppress the diagnosis. “Not enough evidence” is a valid result, not a failed job.

Measured statistics are optional after evaluation. If included, define them precisely: duration from decoded samples; speech-active duration from a versioned VAD algorithm; internal silence spans above a documented threshold excluding leading/trailing silence. A VAD “silence” is not automatically cognitive hesitation. Words per minute requires a usable transcript and a stated denominator; do not present it as a proficiency score or compare incompatible processors.

## Typed feedback shape

Proposed domain result, independent of provider response format:

```json
{
  "assessment_id": "asmt_01",
  "recording_id": "rec_01",
  "transcript_revision": 1,
  "status": "ready",
  "coverage": {"language": "assessed", "audio": "assessed"},
  "summary": "You explained the change of plans and what you did next.",
  "strengths": [
    {
      "id": "finding_01",
      "category": "task_completion",
      "source": "transcript",
      "claim": "You explained why the plan changed.",
      "evidence": [{"kind": "transcript_quote", "quote": "porque empezó a llover", "occurrence": 1}],
      "action": null
    }
  ],
  "improvements": [],
  "limitations": [],
  "provenance": {"rubric_version": "voice-v1"}
}
```

Illustrative example only, not evidence from this learner. IDs in implementation are UUIDs. Public provenance can be concise; the backend stores complete model/prompt/version metadata.

Implement `evidence` as a tagged union: `transcript_quote` (revision, exact quote, occurrence), `audio_span` (recording hash, start/end milliseconds, verified localization method), or `measured_statistic` (processor/version, units, value, scope). Nullable correction and action fields are bounded text. Coverage values: `assessed`, `unavailable`, `insufficient_evidence`. Status values: `ready`, `partial`. The backend, not the provider, assigns IDs and joins records.

Validate each quote against its referenced transcript revision and locate character offsets server-side. Validate audio ranges against decoded duration; mere in-bounds timestamps do not prove correct localization. If segment timestamps are unavailable, provide whole-recording replay and transcript evidence rather than inventing precision. Release requires reliably localized playback for at least transcript-linked language findings; establish alignment in milestone 0. Audio observations without verified localization may use whole-recording replay with an explicit general observation.

An empty findings list with a useful uncertainty explanation is acceptable. Reject unknown keys, excessive output lengths, invalid concept references, and unsupported source/category combinations. Permit one bounded schema-repair attempt without changing source facts; otherwise mark stage failed. Model output and spoken instructions inside the recording are untrusted data, never instructions to change rubric, write SQL, or invoke tools.

## Independent evidence and revisions

Persist distinctions: first attempt with no recorded assistance, learner-reported help, unknown help, same-task repetition before feedback, and retry after feedback. Never infer “independent” merely from sequence number. The UI can say “First attempt”; a stronger independent-production classification needs the existing evidence policy and honest provenance.

Hidden target outcomes distinguish: target used with supported success; valid alternative expressed meaning; target attempted with difficulty; no useful opportunity; insufficient evidence. Map these through existing production validation rather than inventing new FSRS ratings. V1 app assessments make no scheduler reviews.

When a learner disputes feedback or corrects transcription, immediately exclude affected evidence from active learning summaries. Reassessment creates a new version and supersedes the old one. Human edits are attributed to the learner, not relabeled as ASR output. Old recordings and assessment versions remain inspectable until explicitly deleted under retention policy.

## Implementation milestones

### 0. Prove the audio path and select models

Deliver a disposable end-to-end spike: record on a real phone and desktop browser, upload to a development Rust endpoint, decode, transcribe, send actual audio for assessment, and seek to a verified feedback excerpt. Use explicitly provided test clips or consented recordings; no production learner data is sent merely to test a provider.

Record a capability/cost matrix: platform codec, duration, model/endpoint IDs, transcript fidelity, segment alignment, schema success, latency and cost per 90-second take. Test iOS Safari, desktop Chrome/Firefox/Safari where available, and native iOS/Android. Document untested combinations. Use `expo-audio`, start native prototyping in Expo Go where supported, and use development builds when auth/media configuration requires them. Consult current Expo docs rather than assuming the development client and release builds behave identically.

Select a known-duration Spanish fixture set of at least 20 clips covering clear speech, authentic mistakes, restarts, quiet/noisy recordings, valid regional forms, supplied-form repetition, and valid alternatives to a target. A Spanish-competent human annotates expected issues and uncertainty. This small set is a release gate, not a statistical claim about general accuracy.

Suggested go/no-go criteria: all transcript quotes validated; no fabricated precision; at least 90% of displayed corrective findings supported by human review; zero critical cases where a known valid alternative is called wrong or a transcript-only result claims pronunciation evidence. Verify enough useful feedback is produced on annotated clear-error clips to prevent passing by suppressing everything. Set a minimum of 80% of those clips receiving at least one supported actionable finding. Report counts and disagreements alongside percentages. Seek points should locate the intended phrase within approximately one second on supported fixtures. If audio judgments fail, narrow the rubric and re-evaluate; do not silently ship transcript-only grading as full audio assessment.

Target feedback latency: median under 30 seconds and p95 under 60 seconds after upload for 90-second recordings, measured over repeated runs. Targets are not promises; report actual results and choose a documented timeout/resource/cost policy before rollout. Do not hide poor latency behind fake progress percentages.

### 1. Durable recording and playback

Implement Expo routes and media adapters, dedicated auth clients, app API authentication, run/recording tables, streaming uploads, authorized playback and History. No model output needed yet. Demonstrate local-save-before-navigation, upload retry with identical hash, app restart recovery for finalized takes, and private cross-device playback. Test permission denial, interruption, expired login, unsupported codec, and storage failure. Establish resource limits and media backup/restore.

### 2. Feedback vertical slice — first usable release

Implement queue leases, bounded provider adapters, budget reservation, transcription/audio/language/reconciliation stages, evidence-linked feedback UI, and exactly-once canonical projection. Add partial results, disputes, versioned transcript edits, deletion and retention. Publish first attempts without FSRS review proposals. Run the milestone-0 fixtures against the integrated path. This milestone must include actual audio analysis; the previous milestone is infrastructure, not the finished product.

### 3. Coached retry and comparison

Add exposure recording, parent/sequence enforcement, second take and paired playback. Show changed phrases and qualified audio observations. Test that repeating the same task is never promoted to fresh transfer evidence, including offline and multi-device races. Group canonical sessions into one app history run.

### 4. Delayed transfer and 4–3–2

Add fresh communicative tasks informed by prior evidence, respecting saved exclusions and hidden-target policy. Add the timed three-round policy separately, with feedback withheld until the sequence ends. Evaluate whether any evidence is strong enough to support explicit scheduler reviews under the existing rules; keep this separate from adding the activity UI.

## Required verification before release

Use unit tests for meaningful domain invariants, integration tests with a stub provider for failure/retry paths, and real device/browser checks for media behavior. Required cases:

- Double submission creates one job/projection; changing an idempotent payload conflicts.
- Worker dies before and after provider completion and before/after canonical recording commit; restart does not duplicate learning evidence.
- Expired leases, budget races, provider timeouts, invalid JSON, partial stage success, silence, corrupt files and oversized decoded media behave as specified.
- Original audio/transcript are immutable; edit/dispute/supersession removes affected evidence from active app and MCP summaries, without losing audit records.
- Exposure and same-task repetition remain conservative under offline/retry/multi-device ordering.
- Unauthorized users cannot enumerate or play recordings; web mutation CSRF and native token audience checks work; logs contain no payloads/secrets.
- Deletion during processing prevents late result resurrection; audio expiry retains usable text history; backup restore recovers metadata and referenced files.
- Existing `cargo test` and `cargo fmt --check` pass, plus frontend type checking and relevant interaction checks. Follow repository migration checks using disposable copies before production.

## Handoff completion checklist

The implementer should leave model/capability evaluation results, generated API schemas/types, migration/rollback notes, environment-variable documentation, device test results, and measured cost/latency alongside this design. No production migration, model spend, release, or frontend implementation was performed as part of writing this handoff.
