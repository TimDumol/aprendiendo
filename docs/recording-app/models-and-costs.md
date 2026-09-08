# Model evaluation shortlist and 4–3–2 costs

Checked: 2026-09-07. USD, approximate API usage costs, not measured invoices.

[Handoff index](./README.md) · [Assessment and delivery](./assessment-and-delivery.md)

## Evaluation candidates

| Configuration | Role and reason to evaluate | Important limitation |
| --- | --- | --- |
| `gpt-audio-1.5` | Native audio input through OpenAI Chat Completions; request text observations about delivery, intelligibility, repetitions and self-corrections | No Structured Outputs support; validate/format results separately. Listening capability does not prove assessment accuracy. [Model](https://developers.openai.com/api/docs/models/gpt-audio-1.5) |
| `gemini-3.8-flash` | Native audio understanding and structured responses; candidate for direct audio coaching | Generated timestamps and judgments must be checked against recordings. [Audio guide](https://ai.google.dev/gemini-api/docs/audio) |
| `scribe_v2` → `gpt-5-mini` | Verbatim transcription with word timing, followed by text-based language feedback | Text model cannot hear what ASR omitted. [Scribe](https://elevenlabs.io/docs/overview/capabilities/speech-to-text), [GPT-5 Mini](https://developers.openai.com/api/docs/models/gpt-5-mini) |
| `whisper-1` → `gpt-5-mini` | Optional OpenAI-only timestamped transcription baseline | Do not assume complete preservation of fillers or partial-word repetitions. [Transcription guide](https://developers.openai.com/api/docs/guides/speech-to-text) |

Use **Silero VAD** alongside each configuration for speech/silence intervals. It is an open-source detector with ONNX support; running it locally has no model API fee, but consumes worker compute. It does not identify the reason for a pause or diagnose stuttering. [Project](https://github.com/snakers4/silero-vad)

For Scribe, use batch `scribe_v2`, keep `no_verbatim` disabled, request word timestamps, and enable audio-event tagging. Enabling `no_verbatim` removes fillers, false starts and disfluencies. Spanish is supported, but Spanish filler preservation and partial-word repetition recall need evaluation; generic audio tags do not guarantee a stutter annotation. Do not bias transcription toward the expected correction with keyterm prompting. [Capabilities](https://elevenlabs.io/docs/overview/capabilities/speech-to-text)

GPT-5 Mini is a concrete inexpensive text baseline, not a claim that it is the newest or best text model. Hold it fixed initially to isolate differences in the audio evidence. These recommendations are untested on the learner's recordings; access and model availability must be checked before implementation.

## What to measure versus interpret

| Feature | Evidence source |
| --- | --- |
| Silent interval duration | Decoded audio + versioned VAD; word gaps are supporting evidence, not exact silence measurements |
| Fillers and repeated whole words | Verbatim ASR checked against audio |
| Self-correction and abandoned wording | Verbatim transcript; retain both original and corrected phrase |
| Partial-word repetition, e.g. “pe-pe-pero” | Native audio analysis with replay verification; ASR may collapse it |
| Sound prolongation, rhythm, intonation | Actual audio, with qualified observations |
| Grammar, vocabulary, task response | Faithful transcript; audio resolves uncertain wording |

Store measured statistics separately from model interpretations. “Silence from 2.31–3.48 seconds” can be measured; “searching for a word” is an interpretation. Do not treat all pauses, repetitions, or regional pronunciation as errors. Use descriptive disfluency labels rather than a medical diagnosis or an aggregate stutter score.

## Cost assumptions

One 4–3–2 exercise is **three recordings totaling nine minutes**. Each recording is processed once in a fresh request, then one text-only comparison summarizes all three. Feedback is withheld until the rounds finish; internal per-recording processing may happen earlier.

The calculations assume:

- Native-audio assessment: 1,000 text-input tokens and 1,000 billed text-output tokens per recording, in addition to audio. This is a concise assessment budget, not a full verbatim transcript plus unlimited reasoning. No generated speech.
- ASR → text assessment: 1,000 instruction/metadata tokens plus 250 transcript tokens per audio minute, and 1,000 billed output tokens per recording. Compact timing summaries, not a large word-level JSON dump.
- Comparison: GPT-5 Mini with 3,000 input and 1,000 billed output tokens, adding **$0.00275** per exercise. This is a comparison budget, not a resend of the original audio.
- Billed output includes hidden reasoning/thinking where applicable; the assumed token counts are not guarantees. Each extra 1,000 billed output tokens adds $0.010 for GPT-Audio-1.5, $0.00375 for Gemini at the current rate, or $0.002 for GPT-5 Mini.
- No retries, prompt caching discounts, free quotas, batch discounts, paid tools, keyterm/entity extras, storage, bandwidth, worker hosting, tax or currency conversion. Provider plan minimums/commitments are separate from usage valuation. A partially unused subscription may make the effective cost per exercise much higher.

These are small evaluation configurations. The full multi-stage production pipeline may make additional calls; account for those explicitly rather than treating this table as its guaranteed total.

### Rates and conversions

| Component | Rate used | Duration conversion |
| --- | --- | --- |
| GPT-Audio-1.5 | Audio input $32/M tokens; text input $2.50/M; text output $10/M | **Assume** 600 input audio tokens/minute → $0.0192/minute |
| Gemini 3.8 Flash | Input $0.75/M; output including thinking $3.75/M through 2026-12-31 | 1,920 audio tokens/minute → $0.00144/minute |
| Scribe v2 | $0.22/hour, no paid extras | $0.0036667/minute |
| Whisper | $0.006/minute | $0.054 for nine minutes; [model pricing](https://developers.openai.com/api/docs/models/whisper-1) |
| GPT-5 Mini | Text input $0.25/M; output $2/M | Depends on transcript/feedback lengths |

Sources: [GPT-Audio-1.5 rates](https://developers.openai.com/api/docs/models/gpt-audio-1.5), [GPT-5 Mini rates](https://developers.openai.com/api/docs/models/gpt-5-mini), [Gemini pricing](https://ai.google.dev/gemini-api/docs/pricing), [ElevenLabs API pricing](https://elevenlabs.io/pricing/api).

**GPT-Audio conversion caveat:** OpenAI documents one input audio token per 100 ms for Realtime. This design uses that as a planning proxy for GPT-Audio-1.5 file input; the retrieved documentation does not explicitly guarantee the same conversion for that model/endpoint. Verify returned audio-token usage before treating the estimate as a measured rate. Special tokens can add overhead. [OpenAI token accounting](https://developers.openai.com/api/docs/guides/realtime-costs)

Google documents 32 audio tokens/second. Its current price table lists a general input rate for Gemini 3.8 Flash; this estimate applies that rate to audio tokens. Check actual billing on a sample. The published input/output rates double on 2027-01-01, making the Gemini-only exercise about $0.0557 including the unchanged comparison call. [Audio tokenization](https://ai.google.dev/gemini-api/docs/audio), [scheduled pricing](https://ai.google.dev/gemini-api/docs/pricing)

### Estimated processing cost by recording

| Configuration | 4 minutes | 3 minutes | 2 minutes | All recordings + comparison |
| --- | ---: | ---: | ---: | ---: |
| GPT-Audio-1.5, direct feedback | $0.0893 | $0.0701 | $0.0509 | **$0.2131 (~21¢)** |
| Gemini 3.8 Flash, direct feedback | $0.0103 | $0.0088 | $0.0074 | **$0.0292 (~3¢)** |
| Scribe v2 → GPT-5 Mini | $0.0172 | $0.0134 | $0.0097 | **$0.0431 (~4¢)** |
| Whisper → GPT-5 Mini | $0.0265 | $0.0204 | $0.0144 | **$0.0641 (~6¢)** |

Formula for a recording of `m` minutes:

```text
GPT-Audio = m × 600 × 32 / 1,000,000 + (1,000 × 2.50 + 1,000 × 10) / 1,000,000
Gemini   = m × 1,920 × 0.75 / 1,000,000 + (1,000 × 0.75 + 1,000 × 3.75) / 1,000,000
Scribe+text = m × 0.22 / 60 + ((1,000 + 250m) × 0.25 + 1,000 × 2) / 1,000,000
Whisper+text = m × 0.006 + ((1,000 + 250m) × 0.25 + 1,000 × 2) / 1,000,000
Exercise = recording(4) + recording(3) + recording(2) + 0.00275
```

For durable timestamped transcripts alongside direct audio coaching, add a Scribe pass: $0.0147 / $0.0110 / $0.0073 for the respective recordings, **$0.033 total**. Scribe + GPT-Audio + comparison becomes approximately **$0.2461 (25¢)**; Scribe + Gemini + comparison becomes **$0.0622 (6¢)**. These additions do not include a further separate text-grading pass; add roughly $0.0073 per exercise for that pass under the above assumptions, plus any additional reconciliation calls.

At 30 exercises/month (270 recorded minutes), the usage equivalents are approximately $6.39 for GPT-Audio, $0.88 for Gemini, $1.29 for Scribe → GPT-5 Mini, $7.38 for Scribe + GPT-Audio, or $1.87 for Scribe + Gemini. These exclude subscription minimums and all other costs listed above; they are not predicted monthly invoices.

Do not carry previous recordings forward as audio conversation history. Sending 4 minutes, then 4+3, then 4+3+2 bills 20 audio minutes instead of nine. Compare stored findings/transcripts after independent assessment. If direct paired listening is later required, count the additional audio pass explicitly.

## Evaluation order and implementation decision

1. Compare GPT-Audio-1.5 and Gemini directly on the same unprompted Spanish recordings, without transcript anchoring.
2. Compare Scribe + VAD → GPT-5 Mini on the same fixtures. Include Whisper as an optional timestamp baseline.
3. Measure false corrections, lost disfluencies, ASR normalization of grammatical errors, pause-boundary error, replay localization, feedback usefulness, latency, and actual invoice usage. Include clear speech, repeated syllables, fillers, self-corrections, noise and natural rhetorical pauses.
4. Test whether native audio adds useful supported findings beyond the transcript/timing pipeline. Select the smallest workflow that passes the existing human-reviewed release gates.

Working preference: **Scribe + measured timing as the durable evidence layer, with the better native-audio model coaching from the original recording**. The single-model route can win if equally reliable. The transcript-only configuration is an evaluation baseline/fallback and must be labeled accordingly; it cannot satisfy the full audio-feedback release requirement by itself.

This shortlist supersedes the handoff's earlier OpenAI-only evaluation assumption. Use direct provider APIs for these comparisons, without adding OpenRouter yet. No provider spending or model evaluation was performed while writing this document.
