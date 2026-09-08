# Product and UX

[Handoff index](./README.md) · [Backend](./backend.md) · [Assessment and delivery](./assessment-and-delivery.md)

## Main experience

Open Aprendiendo and see one primary action: **Record something**. Under it, offer **Give me a topic** and a recent unfinished recording, if one exists. Avoid a dashboard full of weaknesses before the learner has spoken.

Use two main destinations, Practice and History, with Settings in the header. A recording run has its own stack: task → recorder → review/submission → feedback → optional retry/comparison. On desktop, retain the same actions but use a two-column feedback view: recording and transcript on the left, coaching on the right. On narrow screens, place the player above feedback, with transcript in an expandable section.

### Example session

1. The learner chooses **Give me a topic**. The app displays: “Cuenta algo que cambió tus planes esta semana. ¿Qué pasó y cómo reaccionaste?” It suggests about 90 seconds. Another topic is available before recording.
2. They tap Record. A brief countdown leads to a timer, input-level indicator, task text, and a large Stop button. No live transcript, corrections, model answer, or target grammar is visible.
3. On Stop, the app saves locally before navigating. The review screen offers playback, **Get feedback**, **Keep for later**, and **Discard**. A compact optional reflection asks “Did you use notes or help?” with “No / Yes / Prefer not to say”; absence remains unknown.
4. Get feedback uploads and queues processing. Show factual states: “Uploading”, “Listening to your recording”, “Preparing feedback”. If one stage fails, show what is already saved and the action to retry it.
5. Feedback starts with a task-specific summary, e.g. “You explained the change of plans and what you did next.” This is illustrative copy, not a predetermined model response.
6. One improvement might quote a verified phrase with **Play 0:24–0:31**, explain the problem, and offer a minimal correction. An optional **Another natural way** is clearly an alternative, not a statement that the original was wrong.
7. **Try again** returns to the same task. The app explains once: “This attempt is practice after feedback.” After recording, compare original and retry using paired players and specific changes. Finish returns to History.

No answer is graded while the learner is speaking. A quiet or unsuccessful attempt should lead to a recoverable recording, not a demoralizing grade.

## Screen contracts

| Screen | Required information | Primary action | Other actions |
| --- | --- | --- | --- |
| Practice | Start recording, generate topic, unfinished draft | Record something | History, settings |
| Task | One communicative prompt, suggested length | Record | Another topic, own topic |
| Recorder | Elapsed time, input activity, task, recording state | Stop | Confirm discard/back |
| Recording review | Player, duration, saved/upload state, optional assistance reflection | Get feedback | Keep for later, discard |
| Processing | Durable stage, ability to leave safely after upload | Return to practice | Retry failed stage when available |
| Feedback | Summary, strengths, improvements, player, evidence sources | Try again | Finish, transcript, dispute finding |
| Comparison | Original/retry players, supported changes, assistance labels | Finish | Open either full assessment |
| History | Date, topic, duration, processing state; grouped run/attempts | Open run | Delete run |
| Settings | Explanation language, audio retention, monthly API cap/usage | Save changes | Sign out |

Initial topic generation uses existing saved preferences and exclusions. A learner choosing a spoken activity is an explicit run-scoped modality choice; it must not overwrite their durable defaults. Adapt duration and turn counts to this screen rather than blindly using written sentence budgets. If the policy forbids the activity, show an actionable explanation and allow an explicit run override; do not silently switch activities.

If a hidden learning target informs a topic, keep it server-side. Give the learner a meaningful situation, never “use the subjunctive”. Own-topic diaries need no model call before recording. If topic generation fails, offer own-topic recording immediately.

## Feedback presentation

Use three sections: **What came across**, **Keep doing this**, **Work on this next**. Display at most two strengths and two improvements in total, with one improvement visually prioritized. Fewer findings are preferable to filler. No numeric overall grade, leaderboard, or inferred CEFR level.

Each finding contains:

- A concise claim with a source label: “Language”, “Audio”, or “Your note”.
- A verified quote or a playable audio span, depending on source.
- A short explanation and one suggested action; a correction only when warranted.
- **That’s not what I said** / **I disagree** in an accessible menu.

An audio issue caused by noise says “This part is hard to hear”, not “Your pronunciation is poor”. A valid regional expression or alternative construction must not become an error. A target never attempted is not automatically a failure.

The transcript is supporting evidence, collapsed by default. Make text selectable. Mark uncertain regions if the processor provides usable uncertainty; never invent confidence percentages. Editing a transcript is a separate “Correct transcript” action that preserves its previous version. Say “Feedback needs updating” and offer reassessment after edits. Never let a polished rewrite replace what was actually spoken.

Opening feedback marks the baseline assessment as exposed before revealing its contents. If this cannot be saved, keep feedback hidden and offer retry. Any later recording in the same run is still conservatively classified as repetition; exposure adds the coached label. This prevents a connectivity race from making a retry look independent.

## Recording reliability

- Ask for microphone permission when starting the first recording, with a brief explanation of its purpose. Before the first submission, explain that audio is sent to the configured AI service for feedback.
- Foreground-only in v1. On interruption or backgrounding, stop and finalize if possible; display “Recording interrupted” and let the learner replay and submit the partial take. Never silently merge separated takes.
- Persist native recordings outside disposable cache. Browser drafts use durable browser storage where available, with a download fallback if saving fails. Do not claim browser drafts survive storage eviction or an interrupted capture.
- Distinguish “Saved on this device” from “Uploaded”. Keep a local copy until server receipt and hash verification. On failed upload, retry the same artifact; do not ask for a new take.
- No pause button in v1; a continuous take keeps duration meaningful. Permit early stop. Warn before the five-minute limit and finalize at the limit. Captures shorter than ten seconds may be saved/replayed but are not submitted for assessment.
- During recording, inhibit sleep where supported. Device termination can still lose an in-progress take; crash recovery restores finalized local files and metadata.
- Silence detection provides a recording-quality warning and a chance to replay. Do not manufacture a transcript or count silence as a linguistic failure.

The app must support readable Dynamic Type/font scaling, adequate contrast, screen-reader labels and announced state changes, visible recording indicators beyond color, and touch targets of at least 44 logical pixels. Timers use tabular numerals. Keyboard actions work on web without overriding text-entry controls. No auto-playing recordings or motion required to understand feedback.

## Failure and partial-result behavior

| Condition | Learner experience |
| --- | --- |
| Permission denied | Explain how to enable microphone access; preserve task |
| Device storage full | Stop safely, offer export if possible; never show “Saved” prematurely |
| Offline before upload | Keep local draft; feedback waits for connection |
| Network lost during upload | Keep recording, show Retry upload |
| Upload accepted, app closed | Worker continues; History restores processing status |
| Transcript succeeds, audio analysis fails | Show language feedback with “Audio feedback unavailable”; retry only failed stage |
| Transcript is materially uncertain | Play uncertain region and request correction, or provide only supported audio/task feedback |
| Provider budget reached | Recording and replay remain available; feedback is saved for later |
| Invalid model output | Preserve artifact and retry within bounded policy; never display raw broken output |
| Another device changes a transcript | Explain conflict and load current revision before resubmission |
| Audio retention expires | Preserve text feedback; disable player with “Recording removed”; no broken playback links |

## Comparison and later activities

Compare concrete evidence: a corrected phrase, a clearer explanation, or a measured change in delivery. Say “In your retry…” rather than “You have mastered…”. Shorter duration is not inherently better. Comparisons must reference both attempt IDs and the versions assessed.

After the core release, add a fresh-topic follow-up days later. Label it as new-task evidence and keep task difficulty/context visible. Scheduling a target does not require revealing it.

Then add 4–3–2 as a separate run policy: same topic, three timed recordings, feedback withheld until all rounds finish or the learner explicitly exits the sequence. Repetition without corrections is still repetition, not fresh independent transfer. A partial sequence is retained without inventing missing rounds.
