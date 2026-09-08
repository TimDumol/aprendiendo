import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Alert,
  Linking,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  useWindowDimensions,
  View,
} from "react-native";
import { Link } from "expo-router";

import { ApiError, getHealth, requestFeedback } from "@/lib/api";
import { getAccessToken, getOAuthConfiguration } from "@/lib/auth";
import { DEMO_FEEDBACK } from "@/lib/demo";
import { buildTaskSnapshot } from "@/lib/domain/tasks";
import { errorMessage, logError, logWarn } from "@/lib/logging";
import { deleteRecordingFile, uriForRelativePath } from "@/lib/media/recording";
import {
  createRound,
  createRun,
  deleteRecording,
  deleteRun,
  initializeStorage,
  listRecordings,
  listRounds,
  listRuns,
  parseSnapshot,
  saveRecording,
  updateRecordingFeedback,
  updateRoundStatus,
  updateRunStatus,
} from "@/lib/storage/repository";
import { topicById } from "@/lib/topics";
import type {
  AssessmentResponse,
  Attempt,
  AttemptId,
  Feedback,
  HealthResponse,
  RecordingArtifact,
  TopicId,
  Usage,
} from "@/lib/types";
import { AudioPlayerCard, type SeekRequest } from "./audio-player-card";
import { DeliveryAnalysisPanel } from "./delivery-analysis-panel";
import { FeedbackPanel } from "./feedback-panel";
import { RecordingPanel } from "./recording-panel";
import { TopicPicker } from "./topic-picker";

type Mode = "demo" | "gemini";

type HealthState =
  | { status: "idle" }
  | { status: "loading" }
  | { status: "ready"; data: HealthResponse }
  | { status: "error"; message: string };

function emptyAttempt(id: AttemptId): Attempt {
  return {
    id,
    audio: null,
    hasPlayed: false,
    feedback: null,
    model: null,
    elapsedMs: null,
    usage: null,
    requestStatus: "idle",
    requestError: null,
    requestStartedAt: null,
  };
}

function releaseArtifact(artifact: RecordingArtifact | null) {
  if (artifact?.objectUrl && typeof URL !== "undefined") {
    URL.revokeObjectURL(artifact.objectUrl);
  }
}

function formatDuration(durationMs: number): string {
  const totalSeconds = Math.max(0, Math.round(durationMs / 1000));
  return `${Math.floor(totalSeconds / 60)}:${String(totalSeconds % 60).padStart(2, "0")}`;
}

function confirmDestructive(title: string, message: string, onConfirm: () => void) {
  if (process.env.EXPO_OS === "web") {
    if (typeof globalThis.confirm === "function" && globalThis.confirm(`${title}\n\n${message}`)) {
      onConfirm();
    }
    return;
  }

  Alert.alert(title, message, [
    { text: "Cancel", style: "cancel" },
    { text: "Continue", style: "destructive", onPress: onConfirm },
  ]);
}

function feedbackErrorMessage(error: unknown): string {
  if (!(error instanceof ApiError)) {
    return `${errorMessage(error, "The feedback request failed.")} Your recording is still here; try again manually.`;
  }
  if (error.code === "missing_key") {
    return "The practice server is running without GEMINI_API_KEY. Recording and replay still work; add the key to .env.mvp, .env.mcp, or apps/practice/.env, restart the Rust service, then retry.";
  }
  if (error.code === "network_unavailable") {
    return "The practice service is unavailable. Your recording is still here; check the API URL and server health, then retry when ready.";
  }
  if (error.code === "busy") {
    return "Another feedback request is already in progress. Wait for it to finish, then retry manually; your recording is preserved.";
  }
  if (error.code === "timeout" || error.status === 504) {
    return "Gemini timed out. Retry manually; another attempt may incur another model charge. Your recording is preserved.";
  }
  if (error.status === 429) {
    return "Gemini returned HTTP 429 for this project (rate limit or quota). Check Gemini usage/quota and retry manually; your recording is preserved.";
  }
  if (error.status === 502) {
    return "Gemini returned an unusable response or rejected the configured model. Check GEMINI_MODEL, then retry manually.";
  }
  return error.message;
}

function taskFor(topicId: TopicId, ownTopicText: string): string {
  if (topicId === "own-topic" && ownTopicText.trim()) return ownTopicText.trim();
  return topicById(topicId).prompt;
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

type PracticeScreenProps = { onBack?: () => void; resumeRunId?: string };

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

export function PracticeScreen({ onBack, resumeRunId }: PracticeScreenProps) {
  const { width } = useWindowDimensions();
  const isWide = width >= 900;
  const [mode, setMode] = useState<Mode>(process.env.EXPO_OS === "web" ? "demo" : "gemini");
  const [selectedTopic, setSelectedTopic] = useState<TopicId>("change-of-plans");
  const [ownTopicText, setOwnTopicText] = useState("");
  const [attempts, setAttempts] = useState<Record<AttemptId, Attempt>>({
    "attempt-1": emptyAttempt("attempt-1"),
    "attempt-2": emptyAttempt("attempt-2"),
  });
  const [activeAttemptId, setActiveAttemptId] = useState<AttemptId>("attempt-1");
  const [recordingActive, setRecordingActive] = useState(false);
  const [modeNotice, setModeNotice] = useState<string | null>(null);
  const [health, setHealth] = useState<HealthState>({ status: "idle" });
  const [seekRequest, setSeekRequest] = useState<SeekRequest | null>(null);
  const [waitingSeconds, setWaitingSeconds] = useState(0);
  const [runId, setRunId] = useState<string | null>(null);
  const [roundIds, setRoundIds] = useState<Record<AttemptId, string | null>>({
    "attempt-1": null,
    "attempt-2": null,
  });
  const runIdRef = useRef<string | null>(null);
  const roundIdsRef = useRef<Record<AttemptId, string | null>>({
    "attempt-1": null,
    "attempt-2": null,
  });

  const activeAttempt = attempts[activeAttemptId];
  const task = useMemo(
    () => taskFor(selectedTopic, ownTopicText),
    [ownTopicText, selectedTopic],
  );
  const topicFrozen = recordingActive || Object.values(attempts).some((attempt) => attempt.audio);
  const requestInFlight = Object.values(attempts).some(
    (attempt) => attempt.requestStatus === "loading",
  );
  const hasSecondAttempt = Boolean(attempts["attempt-2"].audio) || activeAttemptId === "attempt-2";

  useEffect(() => {
    void initializeStorage().catch((error) => {
      logError("practice.storage", error, { operation: "initialize" });
      setModeNotice(`Local practice storage could not be initialized: ${errorMessage(error)}`);
    });
  }, []);

  useEffect(() => {
    if (!resumeRunId) return;
    let active = true;
    void (async () => {
      const [run, rounds, storedRecordings] = await Promise.all([
        listRuns().then((runs) => runs.find((candidate) => candidate.id === resumeRunId)),
        listRounds(resumeRunId),
        listRecordings(resumeRunId),
      ]);
      const snapshot = run ? parseSnapshot(run) : null;
      if (!active || !run || !snapshot) return;
      if (snapshot.topicId) setSelectedTopic(snapshot.topicId);
      if (snapshot.topicId === "own-topic") setOwnTopicText(snapshot.prompt);
      runIdRef.current = run.id;
      setRunId(run.id);
      const nextRoundIds: Record<AttemptId, string | null> = {
        "attempt-1": rounds.find((round) => round.sequence === 0)?.id ?? null,
        "attempt-2": rounds.find((round) => round.sequence === 1)?.id ?? null,
      };
      roundIdsRef.current = nextRoundIds;
      setRoundIds(nextRoundIds);
      const nextAttempts: Record<AttemptId, Attempt> = {
        "attempt-1": emptyAttempt("attempt-1"),
        "attempt-2": emptyAttempt("attempt-2"),
      };
      for (const stored of storedRecordings) {
        const round = rounds.find((candidate) => candidate.id === stored.roundId);
        const attemptId: AttemptId | null = round?.sequence === 0 ? "attempt-1" : round?.sequence === 1 ? "attempt-2" : null;
        if (!attemptId || stored.mediaAvailability !== "ready") continue;
        let feedback: Feedback | null = null;
        let usage: Usage | null = null;
        try { feedback = stored.feedbackJson ? JSON.parse(stored.feedbackJson) as Feedback : null; } catch { feedback = null; }
        try { usage = stored.usageJson ? JSON.parse(stored.usageJson) as Usage : null; } catch { usage = null; }
        nextAttempts[attemptId] = {
          ...emptyAttempt(attemptId),
          audio: artifactFromStored(stored),
          feedback,
          model: stored.model,
          usage,
          requestStatus: feedback ? "success" : "idle",
        };
      }
      setAttempts(nextAttempts);
      setActiveAttemptId(nextAttempts["attempt-2"].audio ? "attempt-2" : "attempt-1");
      setModeNotice("Resumed a saved practice boundary. Start the microphone manually when ready.");
    })().catch((error) => {
      logError("practice.resume", error, { resumeRunId });
      if (active) setModeNotice(`Saved practice could not be resumed: ${errorMessage(error)}`);
    });
    return () => {
      active = false;
    };
  }, [resumeRunId]);

  const ensureJournal = useCallback(
    async (attemptId: AttemptId): Promise<{ run: string; round: string }> => {
      if (!runIdRef.current) {
        const run = await createRun(
          buildTaskSnapshot("free-speaking", {
            topicId: selectedTopic,
            prompt: task,
          }),
        );
        runIdRef.current = run;
        setRunId(run);
      }
      if (!roundIdsRef.current[attemptId]) {
        const round = await createRound({
          runId: runIdRef.current,
          sequence: attemptId === "attempt-1" ? 0 : 1,
          targetDurationMs: 120_000,
          classification: attemptId === "attempt-1" ? "baseline" : "repetition",
        });
        roundIdsRef.current = { ...roundIdsRef.current, [attemptId]: round };
        setRoundIds(roundIdsRef.current);
      }
      return { run: runIdRef.current, round: roundIdsRef.current[attemptId] as string };
    },
    [selectedTopic, task],
  );

  const updateAttempt = useCallback(
    (id: AttemptId, update: (attempt: Attempt) => Attempt) => {
      setAttempts((current) => ({ ...current, [id]: update(current[id]) }));
    },
    [],
  );

  const markActiveAttemptPlayed = useCallback(() => {
    updateAttempt(activeAttemptId, (attempt) => ({ ...attempt, hasPlayed: true }));
  }, [activeAttemptId, updateAttempt]);

  useEffect(() => {
    if (mode !== "gemini") {
      setHealth({ status: "idle" });
      return;
    }

    let cancelled = false;
    setHealth({ status: "loading" });
    void getHealth()
      .then((data) => {
        if (!cancelled) setHealth({ status: "ready", data });
      })
      .catch((error: unknown) => {
        logError("practice.health", error, { mode });
        if (!cancelled) setHealth({ status: "error", message: feedbackErrorMessage(error) });
      });
    return () => {
      cancelled = true;
    };
  }, [mode]);

  useEffect(() => {
    if (activeAttempt.requestStatus !== "loading" || !activeAttempt.requestStartedAt) {
      setWaitingSeconds(0);
      return;
    }
    const update = () => {
      setWaitingSeconds(Math.floor((Date.now() - (activeAttempt.requestStartedAt ?? Date.now())) / 1000));
    };
    update();
    const timer = setInterval(update, 250);
    return () => clearInterval(timer);
  }, [activeAttempt.requestStartedAt, activeAttempt.requestStatus]);

  const changeMode = (nextMode: Mode) => {
    if (nextMode === mode) return;
    if (requestInFlight) {
      setModeNotice("Wait for the current feedback request to finish before changing mode.");
      return;
    }
    setMode(nextMode);
    setModeNotice(
      "The recording is kept on this page, but previous feedback was cleared and will not be reused across modes.",
    );
    setAttempts((current) => ({
      "attempt-1": {
        ...current["attempt-1"],
        feedback: null,
        model: null,
        elapsedMs: null,
        usage: null,
        requestStatus: "idle",
        requestError: null,
        requestStartedAt: null,
      },
      "attempt-2": {
        ...current["attempt-2"],
        feedback: null,
        model: null,
        elapsedMs: null,
        usage: null,
        requestStatus: "idle",
        requestError: null,
        requestStartedAt: null,
      },
    }));
  };

  const onRecorded = async (artifact: RecordingArtifact) => {
    const journal = await ensureJournal(activeAttemptId);
    await saveRecording(storedRecording(artifact, journal.round));
    await updateRoundStatus(journal.round, "round_saved", artifact.interrupted ?? false);
    updateAttempt(activeAttemptId, (attempt) => ({
      ...attempt,
      audio: artifact,
      hasPlayed: false,
      feedback: null,
      model: null,
      elapsedMs: null,
      usage: null,
      requestStatus: "idle",
      requestError: null,
      requestStartedAt: null,
    }));
    setModeNotice(null);
  };

  const discardActiveTake = () => {
    if (!activeAttempt.audio) return;
    if (requestInFlight) {
      setModeNotice("Wait for the current feedback request to finish before discarding a take.");
      return;
    }
    confirmDestructive(
      "Discard this take?",
      "This removes the local recording and its feedback from this page. It cannot be recovered here.",
      () => {
        releaseArtifact(activeAttempt.audio);
        const recordingId = activeAttempt.audio?.id;
        if (recordingId) {
          void deleteRecording(recordingId).catch((error) => {
            logError("practice.recording", error, { operation: "delete_recording", recordingId });
            setModeNotice(`The recording was removed from this page, but local deletion failed: ${errorMessage(error)}`);
          });
        }
        updateAttempt(activeAttemptId, (attempt) => ({
          ...attempt,
          audio: null,
          hasPlayed: false,
          feedback: null,
          model: null,
          elapsedMs: null,
          usage: null,
          requestStatus: "idle",
          requestError: null,
          requestStartedAt: null,
        }));
      },
    );
  };

  const startRetry = () => {
    if (activeAttemptId !== "attempt-1" || activeAttempt.feedback === null) return;
    setAttempts((current) => ({
      ...current,
      "attempt-2": emptyAttempt("attempt-2"),
    }));
    setActiveAttemptId("attempt-2");
    setModeNotice("Retry after feedback uses the same frozen task; it is application metadata, not independent transfer evidence.");
  };

  const resetPractice = () => {
    if (requestInFlight) {
      setModeNotice("Wait for the current feedback request to finish before starting new practice.");
      return;
    }
    confirmDestructive(
      "Start new practice?",
      "This clears both takes, feedback and the current topic from this page.",
      () => {
        releaseArtifact(attempts["attempt-1"].audio);
        releaseArtifact(attempts["attempt-2"].audio);
        if (runIdRef.current) {
          void deleteRun(runIdRef.current).then((stored) => {
            stored.forEach((recording) => {
              if (recording.relativePath) {
                deleteRecordingFile({
                  id: recording.id,
                  uri: recording.relativePath,
                  relativePath: recording.relativePath,
                  mimeType: recording.mimeType,
                  durationMs: recording.decodedDurationMs ?? recording.clientDurationMs,
                  bytes: recording.bytes,
                });
              }
            });
          }).catch((error) => {
            logError("practice.reset", error, { operation: "delete_run" });
            setModeNotice(`The page was reset, but saved practice cleanup failed: ${errorMessage(error)}`);
          });
        }
        runIdRef.current = null;
        roundIdsRef.current = { "attempt-1": null, "attempt-2": null };
        setRunId(null);
        setRoundIds({ "attempt-1": null, "attempt-2": null });
        setAttempts({
          "attempt-1": emptyAttempt("attempt-1"),
          "attempt-2": emptyAttempt("attempt-2"),
        });
        setActiveAttemptId("attempt-1");
        setSelectedTopic("change-of-plans");
        setOwnTopicText("");
        setModeNotice(null);
      },
    );
  };

  const submitFeedback = async () => {
    const submittedAttemptId = activeAttemptId;
    const target = attempts[submittedAttemptId];
    if (mode !== "gemini" || !target.audio || target.requestStatus === "loading") return;
    if (!target.hasPlayed) {
      setModeNotice("Play the original take once before requesting feedback.");
      return;
    }
    const maximumFeedbackDurationMs = process.env.EXPO_OS === "web" ? 300_000 : 600_000;
    if (target.audio.durationMs < 10_000 || target.audio.durationMs > maximumFeedbackDurationMs) {
      setModeNotice(`Feedback accepts client-measured takes from 10 seconds through ${maximumFeedbackDurationMs / 60_000} minutes.`);
      return;
    }
    if (target.audio.bytes > 10 * 1024 * 1024) {
      setModeNotice("This take is larger than the 10 MiB submission limit; replay or download it, then record a smaller take.");
      return;
    }

    updateAttempt(submittedAttemptId, (attempt) => ({
      ...attempt,
      requestStatus: "loading",
      requestError: null,
      requestStartedAt: Date.now(),
    }));
    try {
      const response: AssessmentResponse = await requestFeedback(target.audio, task);
      updateAttempt(submittedAttemptId, (attempt) => ({
        ...attempt,
        feedback: response.feedback,
        model: response.model,
        elapsedMs: response.elapsed_ms,
        usage: response.usage,
        requestStatus: "success",
        requestError: null,
        requestStartedAt: null,
      }));
      await updateRecordingFeedback(
        target.audio.id,
        JSON.stringify(response.feedback),
        response.model,
        JSON.stringify(response.usage),
      );
      if (runIdRef.current) await updateRunStatus(runIdRef.current, "ready", undefined, "after-round");
    } catch (error) {
      logError("practice.feedback", error, { attemptId: submittedAttemptId, mode });
      updateAttempt(submittedAttemptId, (attempt) => ({
        ...attempt,
        requestStatus: "error",
        requestError: feedbackErrorMessage(error),
        requestStartedAt: null,
      }));
    }
  };

  const playMoment = (seconds: number | null) => {
    if (!activeAttempt.audio || mode !== "gemini") return;
    setSeekRequest({
      requestId: Date.now(),
      attemptId: activeAttempt.audio.id,
      seconds,
    });
  };

  const downloadRecording = async (artifact: RecordingArtifact) => {
    if (process.env.EXPO_OS === "web" && typeof document !== "undefined") {
      const link = document.createElement("a");
      link.href = artifact.objectUrl ?? artifact.uri;
      link.download = artifact.mimeType.toLowerCase().includes("m4a")
        ? "aprendiendo-practice-take.m4a"
        : "aprendiendo-practice-take.webm";
      link.rel = "noopener";
      document.body.appendChild(link);
      link.click();
      link.remove();
      return;
    }
    try {
      await Linking.openURL(artifact.uri);
    } catch (error) {
      logError("practice.playback", error, { operation: "open_recording", recordingId: artifact.id });
      throw new Error(`The recording could not be opened: ${errorMessage(error)}`);
    }
  };

  const activeFeedback: Feedback | null = mode === "gemini" ? activeAttempt.feedback : DEMO_FEEDBACK;
  const activeLabel = activeAttemptId === "attempt-1" ? "Attempt 1" : "Retry after feedback";
  const canSubmit = Boolean(
    mode === "gemini" &&
      activeAttempt.audio &&
      activeAttempt.hasPlayed &&
      activeAttempt.audio.durationMs >= 10_000 &&
      activeAttempt.audio.durationMs <= (process.env.EXPO_OS === "web" ? 300_000 : 600_000) &&
      activeAttempt.audio.bytes <= 10 * 1024 * 1024 &&
      activeAttempt.requestStatus !== "loading",
  );
  const oauth = getOAuthConfiguration();

  return (
    <ScrollView
      contentContainerStyle={styles.scrollContent}
      contentInsetAdjustmentBehavior="automatic"
      style={styles.scroll}
    >
      <View style={styles.page}>
        {onBack ? (
          <Pressable accessibilityRole="button" onPress={onBack} style={styles.backButton}>
            <Text style={styles.backButtonText}>‹ Practice choices</Text>
          </Pressable>
        ) : null}
        <View style={styles.topRow}>
          <View style={styles.brandBlock}>
            <Text style={styles.brand} selectable>
              Aprendiendo
            </Text>
            <Text style={styles.subtitle} selectable>
              A small, inspectable Spanish speaking experiment
            </Text>
          </View>
          <View accessibilityRole="tablist" style={styles.modeToggle}>
            {(["demo", "gemini"] as Mode[]).map((option) => (
              <Pressable
                key={option}
                accessibilityRole="tab"
                accessibilityState={{ selected: mode === option }}
                onPress={() => changeMode(option)}
                style={({ pressed }) => [
                  styles.modeButton,
                  mode === option && styles.modeButtonSelected,
                  pressed && styles.pressed,
                ]}
              >
                <Text style={[styles.modeButtonText, mode === option && styles.modeButtonTextSelected]}>
                  {option === "demo" ? "Demo" : "Cloud"}
                </Text>
              </Pressable>
            ))}
          </View>
        </View>

        <View style={styles.prototypeNotice}>
          <Text style={styles.prototypeNoticeText} selectable>
            Device practice · finalized takes are saved locally · cloud coaching is optional and can wait for a connection
          </Text>
        </View>
        {modeNotice ? (
          <Text accessibilityLiveRegion="polite" style={styles.modeNotice} selectable>
            {modeNotice}
          </Text>
        ) : null}

        {mode === "gemini" ? (
          <View style={styles.healthBox}>
            {health.status === "loading" ? (
              <Text style={styles.healthText} selectable>
                Checking the practice API and account configuration…
              </Text>
            ) : health.status === "ready" ? (
              <View style={styles.healthContent}>
                <Text style={styles.healthText} selectable>
                  Practice API reachable · {health.data.endpoint}
                </Text>
                <Text style={styles.healthDetail} selectable>
                  Cloud account: {health.data.tokenPresent ? "access token stored locally; server authorization is checked on feedback" : "not connected"}
                </Text>
                {!oauth.configured ? (
                  <Text style={styles.healthError} selectable>
                    Pocket ID configuration issue: {oauth.issues.join(" ")}
                  </Text>
                ) : null}
                <Link href={"/settings" as any} asChild>
                  <Pressable accessibilityRole="button" style={styles.healthLink}>
                    <Text style={styles.healthLinkText}>Open account settings</Text>
                  </Pressable>
                </Link>
              </View>
            ) : health.status === "error" ? (
              <View style={styles.healthContent}>
                <Text style={styles.healthError} selectable>
                  Practice API check failed: {health.message}
                </Text>
                <Text style={styles.healthDetail} selectable>Recording remains available locally.</Text>
                <Link href={"/settings" as any} asChild>
                  <Pressable accessibilityRole="button" style={styles.healthLink}>
                    <Text style={styles.healthLinkText}>Open account and endpoint settings</Text>
                  </Pressable>
                </Link>
              </View>
            ) : null}
          </View>
        ) : null}

        <View style={[styles.columns, isWide && styles.columnsWide]}>
          <View style={[styles.leftColumn, isWide && styles.wideColumn]}>
            <View style={styles.card}>
              <TopicPicker
                disabled={topicFrozen}
                onOwnTopicTextChange={setOwnTopicText}
                onSelect={setSelectedTopic}
                ownTopicText={ownTopicText}
                selectedTopic={selectedTopic}
              />
              <View style={styles.taskBlock}>
                <Text style={styles.taskPrompt} selectable>
                  {task}
                </Text>
                <Text style={styles.taskHint} selectable>
                  Explain what happened and how you reacted. No notes, answer or target grammar is supplied by this prototype.
                </Text>
              </View>
              <RecordingPanel
                attemptLabel={activeLabel}
                disabled={Boolean(activeAttempt.audio)}
                onRecorded={onRecorded}
                onRecordingStateChange={setRecordingActive}
                roundId={roundIds[activeAttemptId] ?? undefined}
              />
            </View>

            {activeAttempt.audio ? (
              <AudioPlayerCard
                key={activeAttempt.audio.id}
                artifact={activeAttempt.audio}
                label={activeLabel}
                onDownload={() => void downloadRecording(activeAttempt.audio as RecordingArtifact).catch((error) => {
                  logError("practice.playback", error, { operation: "download_recording", recordingId: activeAttempt.audio?.id });
                  setModeNotice(errorMessage(error, "The recording could not be downloaded."));
                })}
                onPlayed={markActiveAttemptPlayed}
                seekRequest={seekRequest}
              />
            ) : (
              <View style={styles.emptyPlayer}>
                <Text style={styles.emptyPlayerTitle} selectable>
                  Your take will appear here
                </Text>
                <Text style={styles.emptyPlayerText} selectable>
                  Record continuously, stop when you are done, then listen back before asking Gemini for feedback.
                </Text>
              </View>
            )}

            {activeAttempt.audio ? (
              <DeliveryAnalysisPanel
                recordingId={activeAttempt.audio.id}
                onPlayMoment={(seconds) => setSeekRequest({ requestId: Date.now(), attemptId: activeAttempt.audio?.id ?? "", seconds })}
              />
            ) : null}

            {activeAttempt.audio ? (
              <View style={styles.reviewActions}>
                <Pressable
                  accessibilityLabel="Get feedback"
                  accessibilityRole="button"
                  disabled={!canSubmit}
                  onPress={() => void submitFeedback()}
                  style={({ pressed }) => [
                    styles.primaryAction,
                    !canSubmit && styles.disabledAction,
                    pressed && canSubmit && styles.pressed,
                  ]}
                >
                  <Text style={styles.primaryActionText}>
                    {activeAttempt.requestStatus === "loading" ? "Listening…" : "Get feedback"}
                  </Text>
                </Pressable>
                <Pressable
                  accessibilityLabel="Discard take"
                  accessibilityRole="button"
                  onPress={discardActiveTake}
                  style={({ pressed }) => [styles.secondaryAction, pressed && styles.pressed]}
                >
                  <Text style={styles.secondaryActionText}>Discard take</Text>
                </Pressable>
                {!activeAttempt.hasPlayed ? (
                  <Text style={styles.actionHint} selectable>
                    Replay once to enable feedback.
                  </Text>
                ) : activeAttempt.audio.durationMs < 10_000 ? (
                  <Text style={styles.actionHint} selectable>
                    This client-measured take is {formatDuration(activeAttempt.audio.durationMs)}; feedback starts at 0:10.
                  </Text>
                ) : activeAttempt.audio.bytes > 10 * 1024 * 1024 ? (
                  <Text style={styles.actionHint} selectable>
                    This file is larger than the 10 MiB server limit.
                  </Text>
                ) : null}
              </View>
            ) : null}

            {activeAttempt.requestStatus === "loading" ? (
              <View style={styles.waitingBox}>
                <Text style={styles.waitingTitle} selectable>
                  Uploading and listening… {waitingSeconds}s
                </Text>
                <Text style={styles.waitingText} selectable>
                  One model request is in progress. There are no fake progress stages; you can wait here or keep the take for a manual retry.
                </Text>
              </View>
            ) : null}
            {activeAttempt.requestError ? (
              <View style={styles.errorBox}>
                <Text style={styles.errorText} selectable>
                  {activeAttempt.requestError}
                </Text>
                <Pressable
                  accessibilityLabel="Retry feedback"
                  accessibilityRole="button"
                  disabled={!activeAttempt.audio}
                  onPress={() => void submitFeedback()}
                  style={({ pressed }) => [styles.retryButton, pressed && styles.pressed]}
                >
                  <Text style={styles.retryButtonText}>Retry feedback</Text>
                </Pressable>
              </View>
            ) : null}

            {hasSecondAttempt ? (
              <View style={styles.attemptsBox}>
                <Text style={styles.attemptsTitle} selectable>
                  Attempts
                </Text>
                <View style={styles.attemptSelectorRow}>
                  {(["attempt-1", "attempt-2"] as AttemptId[]).map((id) => (
                    <Pressable
                      key={id}
                      accessibilityRole="tab"
                      accessibilityState={{ selected: activeAttemptId === id }}
                      onPress={() => setActiveAttemptId(id)}
                      style={({ pressed }) => [
                        styles.attemptButton,
                        activeAttemptId === id && styles.attemptButtonSelected,
                        pressed && styles.pressed,
                      ]}
                    >
                      <Text style={styles.attemptButtonText} selectable>
                        {id === "attempt-1" ? "Attempt 1" : "Retry after feedback"}
                      </Text>
                    </Pressable>
                  ))}
                </View>
                <Text style={styles.attemptsNote} selectable>
                  Retry after feedback is application metadata, not independent transfer evidence. Switch takes to replay and read each result; no comparison call is made.
                </Text>
              </View>
            ) : null}

            {mode === "gemini" && activeAttemptId === "attempt-1" && activeAttempt.feedback ? (
              <Pressable
                accessibilityLabel="Try again"
                accessibilityRole="button"
                disabled={Boolean(attempts["attempt-2"].audio)}
                onPress={startRetry}
                style={({ pressed }) => [
                  styles.primaryAction,
                  Boolean(attempts["attempt-2"].audio) && styles.disabledAction,
                  pressed && !attempts["attempt-2"].audio && styles.pressed,
                ]}
              >
                <Text style={styles.primaryActionText}>Try again</Text>
              </Pressable>
            ) : null}
            {hasSecondAttempt && activeAttemptId === "attempt-2" && activeAttempt.feedback ? (
              <Text style={styles.finishNote} selectable>
                Both takes are kept above. Start new practice to explicitly clear them.
              </Text>
            ) : null}
          </View>

          <View style={[styles.rightColumn, isWide && styles.wideColumn]}>
            <View style={styles.feedbackCard}>
              {activeFeedback ? (
                <FeedbackPanel
                  artifact={mode === "gemini" ? activeAttempt.audio : null}
                  demo={mode === "demo"}
                  elapsedMs={mode === "gemini" ? activeAttempt.elapsedMs : null}
                  feedback={activeFeedback}
                  model={mode === "gemini" ? activeAttempt.model : null}
                  usage={mode === "gemini" ? activeAttempt.usage : null}
                  onPlayMoment={playMoment}
                />
              ) : (
                <View style={styles.feedbackEmpty}>
                  <Text style={styles.feedbackEmptyTitle} selectable>
                    Feedback will appear here
                  </Text>
                  <Text style={styles.feedbackEmptyText} selectable>
                  In Cloud mode, only clicking Get feedback sends this original take to the configured practice API. An unavailable service never removes your recording.
                  </Text>
                </View>
              )}
            </View>
          </View>
        </View>

        <View style={styles.footer}>
          <Text style={styles.footerText} selectable>
            Local history keeps task snapshots, finalized audio metadata, feedback and provenance. Audio quota, export, deletion, and server job sync are explicit settings; no CEFR or global fluency score is shown.
          </Text>
          <Pressable
            accessibilityLabel="Start new practice"
            accessibilityRole="button"
            onPress={resetPractice}
            style={({ pressed }) => [styles.resetButton, pressed && styles.pressed]}
          >
            <Text style={styles.resetButtonText}>Start new practice</Text>
          </Pressable>
        </View>
      </View>
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  scroll: {
    backgroundColor: "#f7efe8",
    flex: 1,
  },
  scrollContent: {
    paddingBottom: 48,
    paddingHorizontal: 18,
    paddingTop: 22,
  },
  page: {
    alignSelf: "center",
    gap: 18,
    maxWidth: 1280,
    width: "100%",
  },
  backButton: {
    alignSelf: "flex-start",
    justifyContent: "center",
    minHeight: 44,
  },
  backButtonText: {
    color: "#6f3c26",
    fontSize: 14,
    fontWeight: "800",
  },
  topRow: {
    alignItems: "center",
    flexDirection: "row",
    flexWrap: "wrap",
    gap: 16,
    justifyContent: "space-between",
  },
  brandBlock: {
    gap: 3,
  },
  brand: {
    color: "#2d241f",
    fontSize: 28,
    fontWeight: "900",
    letterSpacing: -0.5,
  },
  subtitle: {
    color: "#75685e",
    fontSize: 13,
  },
  modeToggle: {
    backgroundColor: "#eadbd0",
    borderRadius: 99,
    flexDirection: "row",
    gap: 3,
    padding: 3,
  },
  modeButton: {
    alignItems: "center",
    borderRadius: 99,
    justifyContent: "center",
    minHeight: 44,
    minWidth: 78,
    paddingHorizontal: 14,
  },
  modeButtonSelected: {
    backgroundColor: "#fffaf6",
  },
  modeButtonText: {
    color: "#75685e",
    fontSize: 14,
    fontWeight: "800",
  },
  modeButtonTextSelected: {
    color: "#8f4325",
  },
  prototypeNotice: {
    backgroundColor: "#f1e2d6",
    borderRadius: 11,
    padding: 12,
  },
  prototypeNoticeText: {
    color: "#6f3c26",
    fontSize: 13,
    lineHeight: 19,
  },
  modeNotice: {
    backgroundColor: "#fff2ce",
    borderColor: "#e7c979",
    borderRadius: 11,
    borderWidth: 1,
    color: "#6b4b18",
    fontSize: 13,
    lineHeight: 19,
    padding: 12,
  },
  healthBox: {
    minHeight: 22,
  },
  healthContent: {
    gap: 5,
  },
  healthText: {
    color: "#3f7254",
    fontSize: 13,
    lineHeight: 19,
  },
  healthDetail: {
    color: "#75685e",
    fontSize: 12,
    lineHeight: 18,
  },
  healthError: {
    color: "#8c302a",
    fontSize: 13,
    lineHeight: 19,
  },
  healthLink: {
    alignSelf: "flex-start",
    borderColor: "#c9b8a8",
    borderRadius: 9,
    borderWidth: 1,
    minHeight: 40,
    justifyContent: "center",
    paddingHorizontal: 11,
  },
  healthLinkText: {
    color: "#6f3c26",
    fontSize: 12,
    fontWeight: "800",
  },
  columns: {
    gap: 18,
  },
  columnsWide: {
    flexDirection: "row",
    alignItems: "flex-start",
  },
  leftColumn: {
    gap: 16,
    minWidth: 0,
  },
  rightColumn: {
    minWidth: 0,
  },
  wideColumn: {
    flex: 1,
  },
  card: {
    backgroundColor: "#fffaf6",
    borderColor: "#e2d5ca",
    borderRadius: 18,
    borderWidth: 1,
    gap: 20,
    padding: 18,
  },
  taskBlock: {
    backgroundColor: "#f8f0eb",
    borderLeftColor: "#b95f38",
    borderLeftWidth: 4,
    gap: 8,
    paddingHorizontal: 14,
    paddingVertical: 12,
  },
  taskPrompt: {
    color: "#2d241f",
    fontSize: 20,
    fontWeight: "800",
    lineHeight: 29,
  },
  taskHint: {
    color: "#75685e",
    fontSize: 13,
    lineHeight: 19,
  },
  emptyPlayer: {
    backgroundColor: "#f1e8e1",
    borderColor: "#e2d5ca",
    borderRadius: 16,
    borderWidth: 1,
    gap: 5,
    padding: 16,
  },
  emptyPlayerTitle: {
    color: "#493d35",
    fontSize: 15,
    fontWeight: "800",
  },
  emptyPlayerText: {
    color: "#75685e",
    fontSize: 13,
    lineHeight: 19,
  },
  reviewActions: {
    gap: 10,
  },
  primaryAction: {
    alignItems: "center",
    backgroundColor: "#a34f2d",
    borderRadius: 13,
    justifyContent: "center",
    minHeight: 50,
    paddingHorizontal: 16,
  },
  primaryActionText: {
    color: "#fffaf6",
    fontSize: 15,
    fontWeight: "800",
  },
  secondaryAction: {
    alignItems: "center",
    borderColor: "#c9b8a8",
    borderRadius: 13,
    borderWidth: 1,
    justifyContent: "center",
    minHeight: 46,
    paddingHorizontal: 16,
  },
  secondaryActionText: {
    color: "#6f3c26",
    fontSize: 14,
    fontWeight: "800",
  },
  disabledAction: {
    backgroundColor: "#cbbdb1",
  },
  actionHint: {
    color: "#75685e",
    fontSize: 13,
    lineHeight: 19,
  },
  waitingBox: {
    backgroundColor: "#f1e2d6",
    borderRadius: 12,
    gap: 5,
    padding: 13,
  },
  waitingTitle: {
    color: "#6f3c26",
    fontSize: 14,
    fontWeight: "800",
  },
  waitingText: {
    color: "#75685e",
    fontSize: 13,
    lineHeight: 19,
  },
  errorBox: {
    backgroundColor: "#fff0ee",
    borderColor: "#e5b7b0",
    borderRadius: 12,
    borderWidth: 1,
    gap: 10,
    padding: 13,
  },
  errorText: {
    color: "#8c302a",
    fontSize: 14,
    lineHeight: 20,
  },
  retryButton: {
    alignItems: "center",
    alignSelf: "flex-start",
    borderColor: "#b94c42",
    borderRadius: 10,
    borderWidth: 1,
    justifyContent: "center",
    minHeight: 44,
    paddingHorizontal: 12,
  },
  retryButtonText: {
    color: "#8c302a",
    fontSize: 13,
    fontWeight: "800",
  },
  attemptsBox: {
    backgroundColor: "#f1e8e1",
    borderRadius: 13,
    gap: 10,
    padding: 13,
  },
  attemptsTitle: {
    color: "#493d35",
    fontSize: 14,
    fontWeight: "800",
  },
  attemptSelectorRow: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: 8,
  },
  attemptButton: {
    alignItems: "center",
    borderColor: "#c9b8a8",
    borderRadius: 10,
    borderWidth: 1,
    justifyContent: "center",
    minHeight: 44,
    paddingHorizontal: 11,
  },
  attemptButtonSelected: {
    backgroundColor: "#fffaf6",
    borderColor: "#a34f2d",
  },
  attemptButtonText: {
    color: "#6f3c26",
    fontSize: 13,
    fontWeight: "700",
  },
  attemptsNote: {
    color: "#75685e",
    fontSize: 12,
    lineHeight: 18,
  },
  finishNote: {
    color: "#75685e",
    fontSize: 13,
    lineHeight: 19,
  },
  feedbackCard: {
    backgroundColor: "#fffaf6",
    borderColor: "#e2d5ca",
    borderRadius: 18,
    borderWidth: 1,
    minHeight: 340,
    padding: 18,
  },
  feedbackEmpty: {
    gap: 10,
    paddingVertical: 18,
  },
  feedbackEmptyTitle: {
    color: "#2d241f",
    fontSize: 20,
    fontWeight: "800",
  },
  feedbackEmptyText: {
    color: "#75685e",
    fontSize: 15,
    lineHeight: 23,
  },
  footer: {
    alignItems: "center",
    flexDirection: "row",
    flexWrap: "wrap",
    gap: 12,
    justifyContent: "space-between",
  },
  footerText: {
    color: "#75685e",
    flex: 1,
    fontSize: 12,
    lineHeight: 18,
    minWidth: 240,
  },
  resetButton: {
    alignItems: "center",
    borderColor: "#c9b8a8",
    borderRadius: 10,
    borderWidth: 1,
    justifyContent: "center",
    minHeight: 44,
    paddingHorizontal: 12,
  },
  resetButtonText: {
    color: "#6f3c26",
    fontSize: 13,
    fontWeight: "800",
  },
  pressed: {
    opacity: 0.72,
  },
});
