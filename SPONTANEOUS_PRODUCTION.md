# Aprendiendo MCP: spontaneous-production implementation handoff

Date: 2026-09-06  
Status: proposed design, ready for codebase investigation and implementation. No server changes have been made.  
Audience: an implementation agent with access to the Aprendiendo codebase.

## 1. Objective and scope

Make spontaneous Spanish production a reliable default for this learner. The learner should decide what to say and retrieve suitable language independently, while the system uses their history to create useful communicative opportunities and assess the resulting evidence honestly.

The failure to fix is concrete: selecting “production focused” practice currently allows grammar transformations and strongly cued situational questions. Preferences recorded in session notes are not reliably enforced in future briefs. Success on these tasks can also be confused with independent retrieval.

Implement durable preferences, preference-aware planning, prompt-quality checks, explicit evidence recording, conservative review eligibility, and useful audit reporting. Reuse existing activity, observation, taxonomy, and FSRS infrastructure wherever possible.

This document specifies desired behavior, not a presumed database architecture. Inspect existing models, migrations, validators, tool registration, tests, and scheduling rules before choosing implementation details. Proposed field and action names may be adapted to repository conventions if their semantics survive. Do not assume that a weak exposed tool schema means the server has no internal validation.

Do not change FSRS parameters, reset existing schedules, delete historical practice, or automatically regrade historical responses as part of this work. This is also not a request to add audio analysis, a new LLM service, or a new frontend.

## 2. Verified starting point

These observations came from the live MCP on 2026-09-06, rather than source-code inspection.

### Existing capabilities

- `get_data_status` reported schema version 9, single-learner storage, FSRS-6, 25 sessions, and 45 active weaknesses.
- `get_practice_brief` accepts `drill_mix`, `allowed_drill_types`, activity type/configuration, target filters, a count, and duration settings.
- Available activity types already include picture narration, role-play complications, question-answer sprints, voice diaries, retelling, and corrective conversation.
- Records already expose observation fields including `assessment_phase`, `evidence_strength`, `hint_level`, `evidence_source`, `role`, and `outcome`.
- Activity plans include `intended_evidence_strength`; recording already supports attempts, observations, activity runs, stimuli, and timing provenance.
- `record_practice_session` promises atomic recording and idempotent replay. Its exposed input schema uses `Array<unknown>` for several important collections and `unknown` for `exercise_type_key`. Verify the raw MCP schema and internal model before deciding where to fix this.
- No dedicated preference read/write action was present in the exposed Aprendiendo actions.

### Historical examples

| Session | Evidence | Implication |
|---|---|---|
| 24, September 6 | “Exprésalo con llevar + tiempo + gerundio” and “Une las dos ideas usando para que.” | Explicit controlled production, despite session classification as `production_drill`. |
| 24 notes | Learner explicitly requested no more sentence-transformation/grammatical-manipulation exercises. | This preference needs durable structured storage. |
| 25, September 6 | “¿Cuánto tiempo llevas con eso?” and a situation containing “para que no tenga problemas”. | Situational wording can still supply the target construction. |
| 25 notes | Learner objected to strongly cued prompts and requested short written batches, roughly 3–5 sentences of total output. | Format names alone cannot enforce the objective. |
| 23, September 6 | Picture description with sustained description and speculation. | Preserve tasks that let the learner choose content and language. |
| 19, September 1 notes | Learner requested questions about their actual day/recent experiences to reduce invention overhead. | Personal relevance should not mean repeatedly inventing elaborate scenarios. |

### Reproduction A: current mismatch

```json
{
  "count": 3,
  "drill_mix": "production_focused",
  "recent_prompts_per_weakness": 2
}
```

For both selected targets (`para_que_subjunctive`, `duration_llevar_gerund`), the response ranked `sentence_transformation` at 1.0, `sentence_completion` at 0.9, and `situational_response` at 0.8. The brief did not include the learner's recorded objections as constraints. Its rules included generating exactly three exercises and using two materially varied cues when giving an FSRS rating.

This verifies the returned behavior. Whether it originates in taxonomy inheritance, ranking, defaults, or missing policy resolution must be established in the codebase.

### Reproduction B: existing workaround

```json
{
  "count": 3,
  "drill_mix": "production_focused",
  "allowed_drill_types": ["situational_response", "question_answer", "micro_story"],
  "activity_type": "role_play_complications",
  "activity_config": {
    "unpredictable_followups": true,
    "complication_count": 1
  },
  "recent_prompts_per_weakness": 2
}
```

The response excluded transformations/completions from recommendations and returned initial, follow-up, and complication turns, each with intended spontaneous-production evidence. This demonstrates reusable filtering and activity infrastructure. It does not verify the quality of the eventual learner-facing wording.

## 3. Behavioral contract

1. An unqualified practice request uses saved preferences automatically.
2. A current explicit learner request can override preferences for that session without changing saved defaults.
3. Grammar targets guide the tutor privately. Learner-facing prompts ask for communication, not a named construction.
4. Valid alternative wording is accepted. Failure to use the intended construction means that target was not observed, not that the answer was incorrect.
5. One prompt is delivered at a time. Follow-ups depend on the actual response rather than a fully predetermined script.
6. Written rounds initially aim for 3–5 sentences of total initial learner output. Correction attempts are recorded separately and do not silently expand the planned initial budget.
7. Correction follows the short communicative round, except for a breakdown that prevents continuation. Quote the wrong sentence and give an indirect hint first; if the learner is still wrong, show their wrong sentence and a correction together.
8. Initial production, assisted retry, fresh transfer, and later independent retrieval remain distinct evidence.
9. Prompt labels and elapsed chat time never prove spontaneity, speaking speed, pronunciation, or measured timing.
10. Failure to obtain a schedulable target observation must not force additional drills or penalize the learner.

“Spontaneous” here is an operational practice category, not a claim that internal cognitive processes can be measured. Typed production can be independent; it is not evidence of oral fluency. Repeated retelling can practice fluency but is not automatically fresh independent retrieval.

## 4. Durable practice preferences

### Proposed actions

`get_practice_preferences()` returns current preferences, version, update time, and provenance. `update_practice_preferences(patch, expected_version, source)` performs a validated partial update. Use the repository's existing concurrency approach if equivalent.

Illustrative preference object, to be translated into explicit JSON Schema:

```json
{
  "version": 1,
  "default_practice_mode": "spontaneous",
  "excluded_drill_types": [
    "translation", "sentence_transformation", "sentence_completion",
    "sentence_combining", "minimal_pair_choice", "error_correction"
  ],
  "prompt_policy": {
    "allow_required_constructions": false,
    "allow_sentence_starters": false,
    "allow_model_answer_before_attempt": false,
    "prefer_actual_recent_experiences": true,
    "one_turn_at_a_time": true,
    "adaptive_followups": true
  },
  "written_round_budget": {
    "unit": "sentences",
    "minimum": 3,
    "maximum": 5,
    "scope": "initial_responses"
  },
  "correction_policy": {
    "timing": "after_round",
    "allow_communication_breakdown_exception": true,
    "self_correction_attempts_before_model": 1,
    "quote_original_on_hint": true,
    "show_original_with_model_correction": true
  }
}
```

Excluding `error_correction` as an initial drill must not prohibit feedback and self-correction after genuine production. Likewise, a dialogue-completion format with a blank is inappropriate, while an open response to a conversational turn can be suitable. Enforce semantics as well as enum values.

### Resolution rules

Resolve current explicit learner request, then saved learner preferences, then system defaults. Tool parameters are agent-supplied and must not silently erase a saved exclusion. Require an explicit session override with a concise source statement when changing excluded formats; this records the learner's intent but is not cryptographic proof of it.

Return `effective_preferences`, `preference_version`, `session_overrides`, and any `conflicts` in briefs. Include the compact effective policy in `get_learning_context` regardless of how many recent sessions are requested. Fetching more history should not be necessary to discover enduring constraints.

Do not silently convert every historical note into policy. Seed this learner's preferences from the explicit requirements in this handoff using the normal preference service, with provenance. For other learners retain current defaults until preferences exist. Make the seed idempotent and avoid overwriting later edits.

## 5. Practice brief and activity planning

### Extend the existing action

Add `practice_mode` with clear semantics such as `spontaneous`, `controlled`, and `mixed`. Reuse existing terminology where possible. Keep `drill_mix` backward compatible; do not simply redefine `production_focused` for every existing client.

The learner's saved spontaneous mode should constrain older calls using `drill_mix="production_focused"`. Contradictory explicit parameters should return a conflict rather than silently relaxing the preference. Document precedence between mode, mix, activity type, and allowed formats.

Planning order:

1. Resolve policy and overrides.
2. Select a small number of useful target opportunities from the existing queue/history.
3. Filter disallowed drill and activity types, including taxonomy-inherited recommendations.
4. Rank the remaining choices.
5. Allocate communicative turns within the output budget.
6. Return the effective policy and reasons for selection or exclusion.

If no compatible activity remains, return `no_compatible_activity` with actionable conflicts. Never fall back to transformations merely to fill a count.

### Change target allocation semantics

Current per-item target allocations can encourage a hidden requirement to produce one exact form on every turn. Represent `target_opportunities` instead, with a communicative function and `elicitation_requirement="optional"` in spontaneous mode. The tutor may elicit the function, but must accept alternate constructions.

A three-turn round need not yield three distinct schedulable observations. Counts are planning bounds, not quotas that override the learner's response or completion. If preserving an existing exact-count field is necessary, introduce a separate bounded-round configuration instead of changing that field silently.

Return separate `tutor_context` and `learner_task_constraints` sections. Tutor context may contain target patterns and past errors. This separation reduces accidental copying; it is not a security boundary because the conversational agent sees both.

Plans may specify an initial task and follow-up strategy, such as “respond to the learner's proposed solution with one plausible complication.” They should not manufacture follow-up wording before the learner has answered.

### Good and bad prompt examples

Bad: “Empezaste hace dos horas. Exprésalo con llevar + gerundio.”  
Bad: “¿Cuánto tiempo llevas arreglando el problema?” when assessing independent retrieval of `llevar`.  
Better: “Tu compañero necesita una actualización del problema que estás investigando. ¿Qué le dices?”

Bad: “Explica lo que harás usando para que.”  
Better: “Mañana otra persona se encargará de una tarea que conoces bien. ¿Cómo organizarías el relevo?”

These better prompts offer opportunities, not guaranteed elicitation. A natural reply without the selected construction may be fully successful. Avoid repeatedly using work, dogs, restaurants, and the same personal relations just because those appeared in recent records.

## 6. Prompt validation

Add `validate_practice_plan`, or an equivalent extension to an existing validation action. It should accept the effective policy/version, proposed learner-facing turns, private target opportunities, and relevant preceding context. Support validating a single adaptive follow-up without requiring future turns.

Return structured findings:

```json
{
  "status": "revision_required",
  "policy_version": 1,
  "checks": [
    {
      "code": "target_construction_supplied",
      "turn_no": 1,
      "severity": "error",
      "evidence_span": "Cuánto tiempo llevas",
      "reason": "The prompt supplies the construction being assessed.",
      "method": "rule"
    }
  ],
  "semantic_review": "required"
}
```

Structural checks: excluded formats, explicit construction instructions, sentence starters, model answers, impossible budgets, invalid references, and contradictory configuration.

Semantic checks: disguised transformations, supplying the proposition and its syntax almost verbatim, an effectively single acceptable answer, unnecessary invention burden, and repetitive scenarios. Rules can flag candidates but cannot prove that a prompt is spontaneous. Do not add an external LLM dependency solely for this validator. If the repository has no semantic review service, return rule results and an explicit tutor-review requirement, and document its limits.

Target-word overlap alone is not sufficient to reject a prompt. Common function words will appear naturally. The problem is furnishing the form or answer needed for the intended assessment. Use spans and reasons, not an unexplained numerical spontaneity score.

A validation pass before delivery is not proof that the same text was ultimately shown. Store the actual delivered prompt, optionally linked by hash to the validated text; validate and classify actual recorded evidence again. Never throw away completed practice just because its prompt was poor.

## 7. Explicit recording and evidence model

Expose complete typed schemas for items, attempts, observations, activity runs, reviews, and new weaknesses. Reuse existing types and canonical enums. Inspect raw `tools/list` output to ensure definitions survive serialization and are visible to clients, rather than only existing in internal code.

### Required relationships

- Each attempt links to the actual prompt/item.
- Each observation links to the attempt that supports it.
- Each retry links to its original attempt.
- Each intervention records what hint/model was shown and when relative to attempts.
- Each proposed review links to supporting observation numbers.
- Preserve session-wide observation numbering and existing idempotency behavior.

Where current records store a wrong original response with an outcome representing successful correction, new records must use separate attempts. A corrected-response field is tutor feedback, not learner-produced evidence.

### Proposed additional semantics

| Field/concept | Meaning |
|---|---|
| `attempt_kind` | Initial, self-correction, fresh transfer, or later review; map to existing assessment phases where possible. |
| `prompt_cueing` | None detected, indirect, target form supplied, model supplied, or unknown. Distinct from a hint given after the response. |
| `prior_target_exposure` | Whether relevant wording/model was recently shown; record source and scope where known. |
| `communicative_outcome` | Successful, partially successful, unsuccessful, or not assessed. |
| `target_realization` | Used, attempted, not observed, or ambiguous. |
| `target_accuracy` | Correct, partially correct, incorrect, or not assessable. |
| `assessment_provenance` | Tutor judgment, learner report, deterministic rule, or legacy import, with short supporting evidence. |
| `observed_evidence_strength` | Actual evidence classification, preserving existing categories if adequate. |
| `policy_version` / `validator_version` | Reproduce the applicable policy and validation behavior later. |

Avoid redundant contradictory fields: derive where feasible, and validate combinations. For example, `target_realization="not_observed"` cannot support `target_accuracy="incorrect"`. Do not infer deliberate avoidance from a valid alternate expression. Incidental targets may be recorded when genuinely present, without requiring them to have been preselected.

Prompt cueing and hint level must be separate. An initial response may have `hint_level="none"` while the initial prompt explicitly names the target; that is still controlled/cued evidence.

Retain unknown values rather than assuming missing cueing data means no cue. Keep recorded timing nullable and tied to explicit learner report or external measurement. A spoken transcript does not permit inferred pronunciation, pausing, speed, or response latency.

## 8. Review eligibility and FSRS integration

First inspect the current implementation. The live brief already requires two materially varied cues when a target receives an FSRS rating; extend or clarify existing eligibility rules rather than replacing them speculatively.

Separate three decisions: save the practice evidence, determine whether it supports a review, and determine an appropriate rating. The server can enforce structural eligibility; linguistic correctness still requires a supported assessment. Make that boundary explicit.

Minimum rules for spontaneous retrieval evidence:

- A used/attempted target must have an actual linked learner response.
- No supplied target form/model or relevant assistance may be treated as independent success.
- A corrected retry must not replace the original outcome or count as an independent second cue.
- A same-round fresh prompt following an explicit model remains exposed evidence; do not automatically call it cold retrieval.
- Target not observed or ambiguous means no target rating from that observation.
- Preserve any existing varied-cue minimum. Superficial changes of nouns or times do not automatically establish meaningful variation.
- An eligible independent failure must not be hidden by a later assisted success. Mixed evidence must follow an explicit, documented aggregation policy.
- Do not infer Easy versus Good from correctness alone or transcript timing. Inspect and preserve the existing rating policy; return insufficient rating evidence where necessary rather than inventing effort.

Controlled-practice observations remain valuable records. Whether an existing scheduler accepts them under a separate evidence policy is a codebase decision that must be documented; they must never be reported as spontaneous mastery. Avoid introducing duplicate FSRS cards per skill unless a separate design justifies it.

Return a decision per reviewed target, including `eligible`, reason codes, supporting observation numbers, excluded observation numbers, policy version, and any applied rating. Examples: `target_not_observed`, `prompt_supplied_target`, `assisted_retry`, `insufficient_varied_evidence`, `unknown_assistance`, `invalid_reference`.

For structurally invalid payloads, retain atomic rejection. For structurally valid sessions with ineligible review proposals, save evidence and skip the ineligible updates with explicit decisions in the same atomic transaction. Confirm compatibility with existing behavior and version this response/behavior if necessary. Never silently apply a rejected review, partially update a target, or duplicate a review on replay.

## 9. Audit and retrieval

Extend `get_recent_practice` and compact context summaries before adding a separate analytics action. Report:

- Initial learner turns classified as independent, cued/controlled, assisted, or unknown.
- Policy violations by type, including excluded formats and supplied constructions.
- Selected target opportunities versus targets actually observed.
- Communicative success independently of target-form accuracy.
- Review updates applied/skipped and reasons.
- Retry and later transfer outcomes separately.
- Scenario/topic repetition using simple stored metadata initially.

Define every denominator. For example, independent initial turns divided by all recorded initial turns, with unknowns explicitly shown; never quietly exclude unknown records to inflate a percentage. Do not summarize session labels as actual spontaneous-production rates.

Exact prompt fingerprints are useful but will not detect semantic repetition. Start with topic/scenario tags and recent prompt excerpts, and explain that these are approximate. Avoid embedding infrastructure unless already available and warranted.

## 10. Migration and backward compatibility

1. Inspect the current schema version; version 9 is an observed starting point, not an instruction to force a specific next number.
2. Add preferences and evidence fields using the repository's normal migrations. Keep existing session/attempt IDs, histories, review events, and schedules intact.
3. For legacy records, leave new provenance/cueing fields unknown unless deterministic reconstruction from explicit stored data is possible. Label reconstructed values and preserve their source.
4. Do not infer missing attempts, retroactively claim spontaneous mastery, or automatically replay historical scheduler updates.
5. Preserve old action parameters and canonical exercise keys. Introduce new semantics additively or through an explicit contract version.
6. Seed this learner's approved defaults without modifying other learners' defaults or later preference edits.
7. Verify existing idempotency hashing/replay remains stable across deployment. Store effective policy with the session; retrying the same request after preferences change must replay the original transaction.
8. Document rollout and rollback using repository conventions, including how clients lacking new fields are classified. Unknown evidence should not be silently promoted to independent evidence.

## 11. Acceptance tests

These are behavioral regression tests. They should exercise public tool contracts and real service logic, not merely mirror helper implementations.

| Case | Expected behavior |
|---|---|
| Saved spontaneous preference; ordinary production-focused brief | No transformations/completions in recommendations or allocations; effective policy included. |
| Excluded formats inherited from a taxonomy concept | Filtered before ranking and allocation. |
| Agent passes an excluded activity without a session override | Explicit conflict; no silent preference relaxation. |
| Learner explicitly requests translation today | Session override honored; saved spontaneous default unchanged. |
| All activities filtered out | `no_compatible_activity`; no disallowed fallback. |
| “Exprésalo con llevar + tiempo + gerundio” | Validation requires revision in spontaneous mode. |
| “¿Cuánto tiempo llevas con eso?” targeting llevar | Flag supplied construction even though drill label is situational. |
| Ordinary incidental word overlap | No automatic rejection solely for overlap. |
| Open coworker-update prompt | Can pass structural checks; semantic review provenance remains explicit. |
| Valid response uses another duration construction | Communicative success permitted; llevar not observed; no failure rating. |
| Initial error, indirect hint, successful retry | Two attempts; original error retained; retry classified as assisted. |
| New prompt immediately after model answer | Exposure recorded; not automatically cold retrieval. |
| Role-play label with supplied answer template | Observed evidence remains cued; intended label does not override it. |
| Unrelated article error alongside correct subjunctive | Target accuracy separated from overall sentence correctness. |
| Only one eligible observation when existing policy needs two | Evidence saved; review skipped with reason. |
| Invalid observation/attempt reference | Atomic rejection, no partial records or schedule updates. |
| Valid session, ineligible proposed review | Session saved; review skipped explicitly according to versioned contract. |
| Identical recording replay after preferences change | Same original result; no duplicate session/review. |
| Changed request with same idempotency key | Existing conflict behavior preserved. |
| Legacy missing cueing metadata | Unknown classification; no invented independent-production evidence. |
| Spoken transcript with no timing measurement | No generated latency, speed, or pronunciation claims. |
| Short written round and correction attempts | Initial output budget respected; retries reported separately. |
| New session with no recent history requested | Persistent preferences still available. |
| Preference update with stale version | Explicit concurrency conflict; no lost update. |

Include a small end-to-end example: resolve preferences, generate brief, validate initial turn, record an independent attempt, record a hint and retry, return review decisions, and fetch the session with the same relationships intact. Do not write artificial practice into the live learner database as a test.

## 12. Suggested implementation sequence and deliverables

**Phase 1: inspect and fix planning.** Locate current contracts, filtering/ranking, taxonomy inheritance, and preference facilities. Add durable preferences and effective-policy resolution. Apply exclusions before ranking. Add regression tests for the two live reproductions. This is the smallest useful release.

**Phase 2: improve evidence contracts.** Expose typed schemas, link attempts/observations/interventions, distinguish cueing from hints, and implement explicit eligibility decisions while preserving atomicity and idempotency.

**Phase 3: prevent and detect drift.** Add prompt validation, actual-prompt auditing, compact metrics, and client/tutor instructions. Keep semantic limitations visible.

Deliver implementation, migrations, tests, example tool requests/responses, updated action descriptions, and a short explanation of any deviations from this proposal. Include rollout/backward-compatibility notes and the actual checks run. Investigate whether action schemas are generated from models so fixes land at the source rather than in generated output.

Questions to resolve from the codebase without blocking routine progress:

- Where does `production_focused` currently affect selection, and why do controlled recommendations survive?
- Are preference or learner-profile facilities already present but not exposed?
- Are existing observation phase/evidence enums sufficient to express the proposed distinctions?
- Is the raw MCP input schema weak, or only the connector's representation?
- What evidence currently makes an FSRS review eligible, and how are mixed outcomes rated?
- How does session recording handle an ineligible review today?
- Can the existing activity planner support adaptive turn bounds without changing legacy count semantics?

Keep any unresolved design decision explicit. In particular, do not claim that server validation proves spontaneous cognition or that a scheduling update establishes conversational mastery.

## 13. Tutor-facing integration guidance

Update tool descriptions and any existing plugin instructions so the conversational agent follows this sequence:

1. Retrieve a brief with resolved preferences; inspect history for variety, not for rediscovering permanent policy.
2. Construct a communicative prompt without exposing private target wording.
3. Validate it and address findings before delivery.
4. Present one turn and wait for the learner.
5. Adapt follow-ups to the response and remaining output budget.
6. Complete the brief round, then offer contextualized self-correction before supplying the model correction.
7. Record actual prompts, attempts, interventions, and evidence; inspect returned review decisions.

Project instructions remain a complementary behavioral layer, while MCP preferences and validation make that layer durable and auditable. The original ChatGPT project instructions must be updated at their source; this local project is a mirror and its synced files are read-only reference material. Official project-context reference: https://learn.chatgpt.com/docs/projects .

Success means a routine “let's practice” request reliably yields independent communication opportunities, preserves the learner's choices, and records exactly what the evidence supports—even when a planned grammar target never appears.

