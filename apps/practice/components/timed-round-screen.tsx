import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Pressable, ScrollView, StyleSheet, Text, View } from "react-native";

import { ApiError, requestFeedback } from "@/lib/api";
import { buildTaskSnapshot, FOUR_THREE_TWO_SECONDS } from "@/lib/domain/tasks";
import type { PracticeMode } from "@/lib/domain/tasks";
import { AudioPlayerCard } from "./audio-player-card";
import type { SeekRequest } from "./audio-player-card";
import { DeliveryAnalysisPanel } from "./delivery-analysis-panel";
import { FeedbackPanel } from "./feedback-panel";
import { RecordingPanel } from "./recording-panel";
import { TopicPicker } from "./topic-picker";
import { topicById } from "@/lib/topics";
import { shareRecordingArtifact, uriForRelativePath } from "@/lib/media/recording";
import { errorMessage as diagnosticErrorMessage, logError } from "@/lib/logging";
import type { Feedback, RecordingArtifact, StoredPracticeRun, TopicId } from "@/lib/types";
import {
  createRound,
  createRun,
  listRecordings,
  listRounds,
  listRuns,
  parseSnapshot,
  saveRecording,
  updateRoundStatus,
  updateRunStatus,
} from "@/lib/storage/repository";

type TimedRoundScreenProps = { onBack: () => void; resumeRunId?: string };

const MODE: PracticeMode = "four-three-two";

function formatDuration(ms: number): string {
  const seconds = Math.max(0, Math.round(ms / 1000));
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
}

function errorMessage(error: unknown): string {
  if (error instanceof ApiError) return error.message;
  return `${diagnosticErrorMessage(error, "The coaching request failed.")} The saved takes are still available for a manual retry.`;
}

function storedRecording(artifact: RecordingArtifact, roundId: string) {
  return {
    id: artifact.id,
    roundId,
    relativePath: artifact.relativePath ?? null,
    mimeType: artifact.mimeType,
    container: artifact.container ?? "reported-by-device",
    codec: artifact.codec ?? "reported-by-device",
    bytes: artifact.bytes,
    hash: artifact.hash ?? `unknown-${artifact.id}`,
    decodedDurationMs: artifact.decodedDurationMs ?? null,
    clientDurationMs: artifact.clientDurationMs ?? artifact.durationMs,
    createdOrder: artifact.createdOrder ?? Date.now(),
    interrupted: artifact.interrupted ?? false,
    mediaAvailability: artifact.mediaAvailability ?? "ready",
    warning: artifact.warning ?? null,
    feedbackJson: null,
    model: null,
    usageJson: null,
  };
}

function artifactFromStored(recording: Awaited<ReturnType<typeof listRecordings>>[number]): RecordingArtifact {
  return {
    id: recording.id,
    uri: recording.relativePath ? uriForRelativePath(recording.relativePath) : "",
    relativePath: recording.relativePath ?? undefined,
    mimeType: recording.mimeType,
    container: recording.container,
    codec: recording.codec,
    durationMs: recording.decodedDurationMs ?? recording.clientDurationMs,
    decodedDurationMs: recording.decodedDurationMs,
    clientDurationMs: recording.clientDurationMs,
    bytes: recording.bytes,
    hash: recording.hash,
    createdOrder: recording.createdOrder,
    interrupted: recording.interrupted,
    mediaAvailability: recording.mediaAvailability,
    warning: recording.warning ?? undefined,
  };
}

export function TimedRoundScreen({ onBack, resumeRunId }: TimedRoundScreenProps) {
  const [selectedTopic, setSelectedTopic] = useState<TopicId>("change-of-plans");
  const [ownTopicText, setOwnTopicText] = useState("");
  const [runId, setRunId] = useState<string | null>(null);
  const [roundIds, setRoundIds] = useState<Array<string | null>>([null, null, null]);
  const [recordings, setRecordings] = useState<Array<RecordingArtifact | null>>([null, null, null]);
  const [played, setPlayed] = useState<boolean[]>([false, false, false]);
  const [feedback, setFeedback] = useState<Array<Feedback | null>>([null, null, null]);
  const [feedbackErrors, setFeedbackErrors] = useState<Array<string | null>>([null, null, null]);
  const [loadingFeedback, setLoadingFeedback] = useState<boolean[]>([false, false, false]);
  const [activeRound, setActiveRound] = useState(0);
  const [recordingActive, setRecordingActive] = useState(false);
  const [frozenTask, setFrozenTask] = useState<string | null>(null);
  const [seekRequest, setSeekRequest] = useState<SeekRequest | null>(null);
  const [initializationError, setInitializationError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const runRef = useRef<string | null>(null);
  const roundRefs = useRef<Array<string | null>>([null, null, null]);

  const task = useMemo(
    () => (selectedTopic === "own-topic" && ownTopicText.trim() ? ownTopicText.trim() : topicById(selectedTopic).prompt),
    [ownTopicText, selectedTopic],
  );

  useEffect(() => {
    let cancelled = false;
    setInitializationError(null);
    void (async () => {
      if (resumeRunId) {
        const [run, rounds, storedRecordings] = await Promise.all([
          listRuns().then((runs) => runs.find((candidate) => candidate.id === resumeRunId)),
          listRounds(resumeRunId),
          listRecordings(resumeRunId),
        ]);
        const snapshot = run ? parseSnapshot(run as StoredPracticeRun) : null;
        if (!run || !snapshot) throw new Error("The saved exercise snapshot could not be opened.");
        const orderedRounds = rounds.sort((a, b) => a.sequence - b.sequence);
        const resumedRecordings: Array<RecordingArtifact | null> = [null, null, null];
        for (const stored of storedRecordings) {
          const round = orderedRounds.find((candidate) => candidate.id === stored.roundId);
          if (round && round.sequence < resumedRecordings.length && stored.mediaAvailability === "ready") {
            resumedRecordings[round.sequence] = artifactFromStored(stored);
          }
        }
        if (cancelled) return;
        if (snapshot.topicId) setSelectedTopic(snapshot.topicId);
        if (snapshot.topicId === "own-topic") setOwnTopicText(snapshot.prompt);
        setFrozenTask(snapshot.prompt);
        runRef.current = run.id;
        roundRefs.current = [orderedRounds[0]?.id ?? null, orderedRounds[1]?.id ?? null, orderedRounds[2]?.id ?? null];
        setRunId(run.id);
        setRoundIds(roundRefs.current);
        setRecordings(resumedRecordings);
        const nextRound = resumedRecordings.findIndex((recording) => !recording);
        setActiveRound(nextRound === -1 ? 0 : nextRound);
        return;
      }
      const snapshot = buildTaskSnapshot(MODE, {
        topicId: selectedTopic,
        prompt: task,
        roundDurationsSeconds: [...FOUR_THREE_TWO_SECONDS],
      });
      setFrozenTask(task);
      const createdRunId = await createRun(snapshot);
      const createdRoundIds = await Promise.all(
        FOUR_THREE_TWO_SECONDS.map((seconds, sequence) =>
          createRound({
            runId: createdRunId,
            sequence,
            targetDurationMs: seconds * 1000,
            classification: sequence === 0 ? "baseline" : "repetition",
          }),
        ),
      );
      if (cancelled) return;
      runRef.current = createdRunId;
      roundRefs.current = createdRoundIds;
      setRunId(createdRunId);
      setRoundIds(createdRoundIds);
    })().catch((error) => {
      logError("timed-round.initialize", error, { resumeRunId: resumeRunId ?? null });
      if (!cancelled) setInitializationError(`The timed practice journal could not be prepared: ${diagnosticErrorMessage(error)} Try again or return to practice choices.`);
    });
    return () => {
      cancelled = true;
    };
    // A run is an immutable snapshot. Topic controls are frozen on the first
    // render of this session, before the microphone can start.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [resumeRunId]);

  const saveTake = useCallback(
    async (artifact: RecordingArtifact) => {
      const roundId = roundRefs.current[activeRound];
      const currentRunId = runRef.current;
      if (!roundId || !currentRunId) throw new Error("The exercise is still preparing its local journal.");
      await saveRecording(storedRecording(artifact, roundId));
      await updateRoundStatus(roundId, "round_saved", artifact.interrupted ?? false);
      setRecordings((current) => current.map((item, index) => (index === activeRound ? artifact : item)));
      if (recordings.filter(Boolean).length === 2) {
        await updateRunStatus(currentRunId, "completed");
      }
    },
    [activeRound, recordings],
  );

  const requestRoundFeedback = async (index: number) => {
    const artifact = recordings[index];
    if (!artifact || !played[index] || loadingFeedback[index]) return;
    setLoadingFeedback((current) => current.map((value, item) => (item === index ? true : value)));
    setFeedbackErrors((current) => current.map((value, item) => (item === index ? null : value)));
    try {
      const response = await requestFeedback(artifact, frozenTask ?? task);
      setFeedback((current) => current.map((value, item) => (item === index ? response.feedback : value)));
      if (runRef.current) await updateRunStatus(runRef.current, "completed", undefined, "after-exercise");
    } catch (error) {
      logError("timed-round.feedback", error, { round: index + 1, operation: "request_feedback" });
      setFeedbackErrors((current) => current.map((value, item) => (item === index ? errorMessage(error) : value)));
    } finally {
      setLoadingFeedback((current) => current.map((value, item) => (item === index ? false : value)));
    }
  };

  const allSaved = recordings.every(Boolean);
  const currentArtifact = recordings[activeRound];
  const nextRoundAvailable = Boolean(currentArtifact && activeRound < 2);
  const topicFrozen = recordingActive || frozenTask !== null || recordings.some(Boolean);
  const sessionTask = frozenTask ?? task;

  return (
    <ScrollView
      contentInsetAdjustmentBehavior="automatic"
      contentContainerStyle={styles.scrollContent}
      style={styles.scroll}
    >
      <View style={styles.page}>
        <Pressable accessibilityRole="button" onPress={onBack} style={styles.backButton}>
          <Text style={styles.backText}>‹ Practice choices</Text>
        </Pressable>
        <View style={styles.header}>
          <Text style={styles.title} selectable>
            4–3–2 practice
          </Text>
          <Text style={styles.subtitle} selectable>
            Keep the message and adapt the delivery as the available time becomes shorter.
          </Text>
        </View>
        {initializationError ? <Text accessibilityLiveRegion="polite" style={styles.error} selectable>{initializationError}</Text> : null}

        <View style={styles.card}>
          <TopicPicker
            disabled={topicFrozen}
            onOwnTopicTextChange={setOwnTopicText}
            onSelect={setSelectedTopic}
            ownTopicText={ownTopicText}
            selectedTopic={selectedTopic}
          />
          <View style={styles.promptBox}>
            <Text style={styles.prompt} selectable>
              {sessionTask}
            </Text>
            <Text style={styles.helper} selectable>
              The topic is frozen across all three rounds. Rest before the next round when you are ready.
            </Text>
          </View>
        </View>

        <View style={styles.roundSelector}>
          {FOUR_THREE_TWO_SECONDS.map((seconds, index) => (
            <Pressable
              key={seconds}
              accessibilityRole="tab"
              accessibilityState={{ selected: activeRound === index }}
              onPress={() => setActiveRound(index)}
              style={[styles.roundTab, activeRound === index && styles.roundTabSelected]}
            >
              <Text style={styles.roundTabTitle}>{index === 0 ? "Round 1" : `Round ${index + 1}`}</Text>
              <Text style={styles.roundTabDuration}>{Math.round(seconds / 60)} minutes</Text>
              <Text style={styles.roundTabStatus}>{recordings[index] ? "Saved" : "Not started"}</Text>
            </Pressable>
          ))}
        </View>

        <View style={styles.card}>
          <Text style={styles.sectionTitle} selectable>
            {activeRound === 0 ? "Baseline" : `Repetition ${activeRound + 1}`}
          </Text>
          <RecordingPanel
            attemptLabel={`Round ${activeRound + 1}`}
            disabled={Boolean(currentArtifact)}
            maxDurationMs={FOUR_THREE_TWO_SECONDS[activeRound] * 1000}
            minDurationMs={0}
            onRecorded={saveTake}
            onRecordingStateChange={setRecordingActive}
            roundId={roundIds[activeRound] ?? undefined}
            targetDurationMs={FOUR_THREE_TWO_SECONDS[activeRound] * 1000}
          />
          {currentArtifact ? (
            <AudioPlayerCard
              artifact={currentArtifact}
              label={`Round ${activeRound + 1}`}
              onDownload={() => {
                setActionError(null);
                void shareRecordingArtifact(currentArtifact).catch((error) => {
                  logError("timed-round.export", error, { recordingId: currentArtifact.id, round: activeRound + 1 });
                  setActionError(`The recording could not be shared: ${diagnosticErrorMessage(error)}`);
                });
              }}
              onPlayed={() => setPlayed((current) => current.map((value, index) => (index === activeRound ? true : value)))}
              seekRequest={seekRequest}
            />
          ) : null}
          {actionError ? <Text accessibilityLiveRegion="polite" style={styles.error} selectable>{actionError}</Text> : null}
          {nextRoundAvailable ? (
            <Pressable
              accessibilityRole="button"
              onPress={() => setActiveRound((current) => Math.min(2, current + 1))}
              style={styles.primaryButton}
            >
              <Text style={styles.primaryButtonText}>Start next round</Text>
            </Pressable>
          ) : null}
        </View>

        <View style={styles.resultCard}>
          <Text style={styles.sectionTitle} selectable>
            Results
          </Text>
          {!allSaved ? (
            <Text style={styles.helper} selectable>
              Feedback and measured comparisons stay hidden until all three rounds are saved. A saved take can be replayed before then.
            </Text>
          ) : (
            <>
              <Text style={styles.success} selectable>
                All three rounds are saved. Each round remains independent evidence; shorter does not automatically mean better.
              </Text>
              <Pressable
                accessibilityRole="button"
                disabled={!currentArtifact || !played[activeRound] || loadingFeedback[activeRound]}
                onPress={() => void requestRoundFeedback(activeRound)}
                style={[styles.primaryButton, (!played[activeRound] || loadingFeedback[activeRound]) && styles.disabledButton]}
              >
                <Text style={styles.primaryButtonText}>
                  {loadingFeedback[activeRound] ? "Listening…" : "Request coaching for this round"}
                </Text>
              </Pressable>
              {!played[activeRound] ? (
                <Text style={styles.helper} selectable>
                  Replay the selected round once before requesting paid coaching.
                </Text>
              ) : null}
              {feedbackErrors[activeRound] ? <Text style={styles.error} selectable>{feedbackErrors[activeRound]}</Text> : null}
              {feedback[activeRound] ? (
                <FeedbackPanel
                  artifact={currentArtifact}
                  demo={false}
                  elapsedMs={null}
                  feedback={feedback[activeRound] as Feedback}
                  model={null}
                  onPlayMoment={() => undefined}
                  usage={null}
                />
              ) : null}
              {currentArtifact ? (
                <DeliveryAnalysisPanel
                  recordingId={currentArtifact.id}
                  onPlayMoment={(seconds) => setSeekRequest({ requestId: Date.now(), attemptId: currentArtifact.id, seconds })}
                />
              ) : null}
            </>
          )}
        </View>

        <Text style={styles.footer} selectable>
          Run {runId ? "saved locally" : "preparing local journal"} · 4–3–2 results are withheld until the exercise ends.
        </Text>
      </View>
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  scroll: { backgroundColor: "#f7efe8", flex: 1 },
  scrollContent: { paddingBottom: 48, paddingHorizontal: 18, paddingTop: 20 },
  page: { alignSelf: "center", gap: 16, maxWidth: 860, width: "100%" },
  backButton: { alignSelf: "flex-start", minHeight: 44, justifyContent: "center" },
  backText: { color: "#6f3c26", fontSize: 14, fontWeight: "800" },
  header: { gap: 5 },
  title: { color: "#2d241f", fontSize: 30, fontWeight: "900" },
  subtitle: { color: "#75685e", fontSize: 15, lineHeight: 22 },
  card: { backgroundColor: "#fffaf6", borderColor: "#e2d5ca", borderRadius: 18, borderWidth: 1, gap: 16, padding: 18 },
  promptBox: { backgroundColor: "#f8f0eb", borderLeftColor: "#b95f38", borderLeftWidth: 4, gap: 7, padding: 13 },
  prompt: { color: "#2d241f", fontSize: 20, fontWeight: "800", lineHeight: 29 },
  helper: { color: "#75685e", fontSize: 13, lineHeight: 20 },
  roundSelector: { flexDirection: "row", flexWrap: "wrap", gap: 9 },
  roundTab: { backgroundColor: "#f1e8e1", borderColor: "#d9c9ba", borderRadius: 13, borderWidth: 1, flex: 1, gap: 3, minWidth: 145, padding: 12 },
  roundTabSelected: { backgroundColor: "#fffaf6", borderColor: "#a34f2d" },
  roundTabTitle: { color: "#493d35", fontSize: 14, fontWeight: "800" },
  roundTabDuration: { color: "#6f3c26", fontSize: 13, fontWeight: "700" },
  roundTabStatus: { color: "#8f6a58", fontSize: 12 },
  sectionTitle: { color: "#2d241f", fontSize: 20, fontWeight: "800" },
  primaryButton: { alignItems: "center", backgroundColor: "#a34f2d", borderRadius: 13, justifyContent: "center", minHeight: 50, paddingHorizontal: 16 },
  primaryButtonText: { color: "#fffaf6", fontSize: 15, fontWeight: "800" },
  disabledButton: { backgroundColor: "#cbbdb1" },
  resultCard: { backgroundColor: "#fffaf6", borderColor: "#e2d5ca", borderRadius: 18, borderWidth: 1, gap: 14, padding: 18 },
  success: { backgroundColor: "#edf6ee", borderRadius: 11, color: "#356345", fontSize: 14, lineHeight: 21, padding: 12 },
  error: { color: "#8c302a", fontSize: 14, lineHeight: 20 },
  method: { color: "#75685e", fontSize: 12, lineHeight: 18 },
  footer: { color: "#75685e", fontSize: 12, lineHeight: 18 },
});
