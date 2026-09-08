import { useEffect, useMemo, useState } from "react";
import { Image, Modal, Pressable, ScrollView, StyleSheet, Text, TextInput, View } from "react-native";
import * as ImagePicker from "expo-image-picker";

import { ApiError, requestFeedback, requestPhotoFeedback, type ImageAttachment } from "@/lib/api";
import { buildTaskSnapshot, type PhotoTask } from "@/lib/domain/tasks";
import { persistImageAsset, shareRecordingArtifact, uriForRelativePath, type PersistedImage } from "@/lib/media/recording";
import {
  createRound,
  createRun,
  getSettings,
  listRecordings,
  listRounds,
  listRuns,
  parseSnapshot,
  saveRecording,
  updateRoundStatus,
  updateRunStatus,
} from "@/lib/storage/repository";
import type { Feedback, RecordingArtifact, StoredPracticeRun } from "@/lib/types";
import { AudioPlayerCard } from "./audio-player-card";
import type { SeekRequest } from "./audio-player-card";
import { DeliveryAnalysisPanel } from "./delivery-analysis-panel";
import { FeedbackPanel } from "./feedback-panel";
import { RecordingPanel } from "./recording-panel";
import { PHOTO_TASKS } from "@/lib/domain/tasks";
import { errorMessage as diagnosticErrorMessage, logError } from "@/lib/logging";

type PhotoPracticeScreenProps = {
  photo: PhotoTask;
  onBack: () => void;
  resumeRunId?: string;
};

function errorMessage(error: unknown): string {
  if (error instanceof ApiError) return error.message;
  return `${diagnosticErrorMessage(error, "The coaching request failed.")} The saved take is still available for a manual retry.`;
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

export function PhotoPracticeScreen({ photo: initialPhoto, onBack, resumeRunId }: PhotoPracticeScreenProps) {
  const [photo, setPhoto] = useState(initialPhoto);
  const [image, setImage] = useState<PersistedImage | null>(null);
  const [preparationSeconds, setPreparationSeconds] = useState(120);
  const [practiceStyle, setPracticeStyle] = useState<"guided" | "simulation">("guided");
  const [notes, setNotes] = useState("");
  const [preparing, setPreparing] = useState(false);
  const [preparationRemaining, setPreparationRemaining] = useState(120);
  const [started, setStarted] = useState(false);
  const [recording, setRecording] = useState<RecordingArtifact | null>(null);
  const [played, setPlayed] = useState(false);
  const [feedback, setFeedback] = useState<Feedback | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [runId, setRunId] = useState<string | null>(null);
  const [roundId, setRoundId] = useState<string | null>(null);
  const [recordingActive, setRecordingActive] = useState(false);
  const [imageZoomOpen, setImageZoomOpen] = useState(false);
  const [seekRequest, setSeekRequest] = useState<SeekRequest | null>(null);

  const visibleImageSource = image?.uri
    ? { uri: image.uri }
    : photo.imageAsset ?? (photo.imageUri ? { uri: photo.imageUri } : null);
  const task = useMemo(() => photo.prompt, [photo.prompt]);

  useEffect(() => {
    void getSettings().then((settings) => {
      setPreparationSeconds(settings.preparationSeconds);
      setPracticeStyle(settings.photoPracticeStyle);
    }).catch((error) => {
      logError("photo-practice.settings", error, { operation: "read_settings" });
      setError(`Photo practice settings could not be read: ${diagnosticErrorMessage(error)}`);
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
      const snapshot = run ? parseSnapshot(run as StoredPracticeRun) : null;
      if (!active || !run || !snapshot) return;
      const savedPhoto = snapshot.photo ?? initialPhoto;
      setPhoto(savedPhoto);
      setPreparationSeconds(snapshot.preparationSeconds);
      setPracticeStyle(snapshot.photoPracticeStyle ?? "guided");
      setPreparationRemaining(0);
      setNotes(run.assistance ?? "");
      setRunId(run.id);
      setStarted(true);
      setPreparing(false);
      const round = rounds.sort((a, b) => a.sequence - b.sequence)[0];
      setRoundId(round?.id ?? null);
      if (savedPhoto.imageRelativePath) {
        setImage({
          uri: uriForRelativePath(savedPhoto.imageRelativePath),
          relativePath: savedPhoto.imageRelativePath,
          mimeType: "image/jpeg",
          bytes: 0,
          sha256: savedPhoto.imageSha256 ?? undefined,
        });
      }
      const saved = storedRecordings.find((recording) => recording.roundId === round?.id && recording.mediaAvailability === "ready");
      if (saved) {
        setRecording(artifactFromStored(saved));
      }
    })().catch((error) => {
      logError("photo-practice.resume", error, { resumeRunId });
      if (active) setError(`Saved photo practice could not be resumed: ${diagnosticErrorMessage(error)}`);
    });
    return () => {
      active = false;
    };
  }, [initialPhoto, resumeRunId]);

  useEffect(() => {
    if (!preparing) return;
    if (preparationRemaining <= 0) {
      setPreparing(false);
      return;
    }
    const timer = setInterval(() => setPreparationRemaining((current) => Math.max(0, current - 1)), 1_000);
    return () => clearInterval(timer);
  }, [preparationRemaining, preparing]);

  const choosePhoto = async () => {
    if (started || recordingActive) return;
    setError(null);
    try {
      const result = await ImagePicker.launchImageLibraryAsync({
        mediaTypes: ["images"],
        allowsEditing: false,
        quality: 0.8,
      });
      const asset = result.canceled ? null : result.assets[0];
      if (!asset) return;
      if ((asset.fileSize ?? 0) > 5 * 1024 * 1024) {
        setError("Choose an image up to 5 MiB. The original practice recording remains available.");
        return;
      }
      setPhoto({
        ...photo,
        id: `selected-${Date.now()}`,
        title: "Tu imagen",
        scene: "A user-selected everyday image.",
        prompt: "Describe la imagen: el lugar, las personas, sus posiciones, sus acciones y los detalles importantes.",
        provenance: "Selected by the learner; resized by the system picker where supported. Location metadata is not used by the app.",
        imageUri: asset.uri,
      });
      setImage({
        uri: asset.uri,
        relativePath: null,
        mimeType: asset.mimeType ?? "image/jpeg",
        bytes: asset.fileSize ?? 0,
        sha256: undefined,
      });
      setFeedback(null);
    } catch (error) {
      logError("photo-practice.image-picker", error, { operation: "choose_image" });
      setError(`The image picker failed: ${diagnosticErrorMessage(error)}`);
    }
  };

  const beginExercise = async () => {
    if (started || recordingActive) return;
    setError(null);
    try {
      const bundledImageUri = photo.imageAsset
        ? Image.resolveAssetSource(photo.imageAsset)?.uri
        : photo.imageUri;
      const sourceImage = image ?? (bundledImageUri
        ? { uri: bundledImageUri, mimeType: "image/jpeg", bytes: 0, relativePath: null }
        : null);
      const persistedImage = sourceImage
        ? await persistImageAsset(sourceImage.uri, sourceImage.mimeType, `photo-${Date.now()}`)
        : null;
      const selectedPhoto = persistedImage
        ? {
            ...photo,
            imageUri: null,
            imageRelativePath: persistedImage.relativePath,
            imageSha256: persistedImage.sha256 ?? null,
          }
        : photo;
      const snapshot = buildTaskSnapshot("describe-photo", {
        photo: selectedPhoto,
        prompt: task,
        photoPracticeStyle: practiceStyle,
        preparationSeconds,
      });
      const createdRunId = await createRun(snapshot);
      const createdRoundId = await createRound({
        runId: createdRunId,
        sequence: 0,
        targetDurationMs: 180_000,
        classification: "image-description",
      });
      await updateRunStatus(createdRunId, "recording", notes.trim() || null);
      setRunId(createdRunId);
      setRoundId(createdRoundId);
      setStarted(true);
      setPreparing(true);
      setPreparationRemaining(preparationSeconds);
      if (persistedImage) setImage(persistedImage);
    } catch (error) {
      logError("photo-practice.begin", error, { operation: "create_local_journal" });
      setError(`The local exercise journal could not be prepared: ${diagnosticErrorMessage(error)} Try again before recording.`);
    }
  };

  const saveTake = async (artifact: RecordingArtifact) => {
    if (!roundId) throw new Error("The local round has not been created.");
    await saveRecording(storedRecording(artifact, roundId));
    await updateRoundStatus(roundId, "round_saved", artifact.interrupted ?? false);
    if (runId) await updateRunStatus(runId, "completed", notes.trim() || null);
    setRecording(artifact);
  };

  const requestCoaching = async () => {
    if (!recording || !played || loading) return;
    setLoading(true);
    setError(null);
    try {
      const attachment: ImageAttachment | null = image
        ? { uri: image.uri, relativePath: image.relativePath, mimeType: image.mimeType, bytes: image.bytes, name: "practice-image" }
        : null;
      const response = attachment
        ? await requestPhotoFeedback(recording, task, attachment)
        : await requestFeedback(recording, task);
      setFeedback(response.feedback);
      if (runId) await updateRunStatus(runId, "completed", undefined, "after-round");
    } catch (requestError) {
      logError("photo-practice.feedback", requestError, { operation: "request_feedback" });
      setError(errorMessage(requestError));
    } finally {
      setLoading(false);
    }
  };

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
            Describe a photo
          </Text>
          <Text style={styles.subtitle} selectable>
            A practice version of DELE A2 oral task 2: one everyday image, a short preparation period, and a three-minute description.
          </Text>
        </View>
        {error && !recording ? <Text accessibilityLiveRegion="polite" style={styles.error} selectable>{error}</Text> : null}

        <View style={styles.photoCard}>
          {visibleImageSource ? (
            <Pressable accessibilityLabel="Open practice image zoom" accessibilityRole="button" onPress={() => setImageZoomOpen(true)}>
              <Image accessibilityLabel="Practice image" resizeMode="cover" source={visibleImageSource} style={styles.photo} />
            </Pressable>
          ) : (
            <View accessibilityLabel={`Scene illustration: ${photo.scene}`} style={styles.sceneCard}>
              <Text style={styles.sceneEmoji}>◌</Text>
              <Text style={styles.sceneText} selectable>{photo.scene}</Text>
              <Text style={styles.sceneNote} selectable>Bundled practice visual · not an official exam photograph</Text>
            </View>
          )}
          <Text style={styles.photoTitle} selectable>{photo.title}</Text>
          <Text style={styles.provenance} selectable>{photo.provenance}</Text>
          <View style={styles.photoChoices}>
            {PHOTO_TASKS.map((candidate) => (
              <Pressable disabled={started} key={candidate.id} onPress={() => { setPhoto(candidate); setImage(null); setFeedback(null); }} style={[styles.smallChoice, candidate.id === photo.id && styles.smallChoiceSelected, started && styles.disabledChoice]}>
                <Text style={styles.smallChoiceText}>{candidate.title}</Text>
              </Pressable>
            ))}
          </View>
          <Pressable accessibilityRole="button" disabled={started} onPress={() => void choosePhoto()} style={[styles.secondaryButton, started && styles.disabledButton]}>
            <Text style={styles.secondaryButtonText}>Choose my own image</Text>
          </Pressable>
        </View>

        <View style={styles.card}>
          <Text style={styles.sectionTitle} selectable>Task</Text>
          <Text style={styles.modeLabel} selectable>
            {practiceStyle === "guided" ? "Guided practice · optional prompts" : "Task simulation · keep the task conditions"}
          </Text>
          <Text style={styles.prompt} selectable>{task}</Text>
          <Text style={styles.helper} selectable>
            You may make notes. Notes are assistance metadata and are never treated as spoken Spanish.
          </Text>
          <TextInput
            accessibilityLabel="Optional preparation notes"
            editable={!started}
            multiline
            onChangeText={setNotes}
            placeholder="Optional notes for your preparation"
            placeholderTextColor="#87796d"
            style={styles.notes}
            textAlignVertical="top"
            value={notes}
          />
          {!started ? (
            <Pressable accessibilityRole="button" onPress={() => void beginExercise()} style={styles.primaryButton}>
              <Text style={styles.primaryButtonText}>Begin {Math.floor(preparationSeconds / 60)}:{String(preparationSeconds % 60).padStart(2, "0")} preparation</Text>
            </Pressable>
          ) : preparing ? (
            <View style={styles.timerBox}>
              <Text style={styles.timer} selectable>{Math.floor(preparationRemaining / 60)}:{String(preparationRemaining % 60).padStart(2, "0")}</Text>
              <Text style={styles.helper} selectable>Prepare your description. You can start recording before the timer ends.</Text>
              <Pressable accessibilityRole="button" onPress={() => setPreparing(false)} style={styles.secondaryButton}>
                <Text style={styles.secondaryButtonText}>Start recording now</Text>
              </Pressable>
            </View>
          ) : null}
        </View>

        {started ? (
          <View style={styles.card}>
            <Text style={styles.sectionTitle} selectable>Record up to 3 minutes</Text>
            <RecordingPanel
              attemptLabel="Photo description"
              disabled={Boolean(recording)}
              maxDurationMs={180_000}
              minDurationMs={0}
              onRecorded={saveTake}
              onRecordingStateChange={setRecordingActive}
              roundId={roundId ?? undefined}
              targetDurationMs={180_000}
            />
            {recording ? (
              <AudioPlayerCard
                artifact={recording}
                label="Photo description"
                onDownload={() => void shareRecordingArtifact(recording).catch((shareError) => {
                  logError("photo-practice.export", shareError, { recordingId: recording.id });
                  setError(`The recording could not be shared: ${diagnosticErrorMessage(shareError)}`);
                })}
                onPlayed={() => setPlayed(true)}
                seekRequest={seekRequest}
              />
            ) : null}
            {recording ? <DeliveryAnalysisPanel recordingId={recording.id} onPlayMoment={(seconds) => setSeekRequest({ requestId: Date.now(), attemptId: recording.id, seconds })} /> : null}
          </View>
        ) : null}

        {recording ? (
          <View style={styles.resultCard}>
            <Text style={styles.sectionTitle} selectable>Review</Text>
            <Text style={styles.helper} selectable>
              Image-aware coaching receives the selected image and the audio. If no image was selected, only general language and audio feedback is requested.
            </Text>
            <Pressable
              accessibilityRole="button"
              disabled={!played || loading}
              onPress={() => void requestCoaching()}
              style={[styles.primaryButton, (!played || loading) && styles.disabledButton]}
            >
              <Text style={styles.primaryButtonText}>{loading ? "Uploading and listening…" : "Request coaching"}</Text>
            </Pressable>
            {!played ? <Text style={styles.helper} selectable>Replay the take once before requesting coaching.</Text> : null}
            {error ? <Text style={styles.error} selectable>{error}</Text> : null}
            {feedback ? (
              <FeedbackPanel
                artifact={recording}
                demo={false}
                elapsedMs={null}
                feedback={feedback}
                model={null}
                onPlayMoment={() => undefined}
                usage={null}
              />
            ) : null}
          </View>
        ) : null}

        <Text style={styles.footer} selectable>
          {runId ? "Exercise saved locally" : "No recording has been saved yet"} · This is task-2 practice, not an official exam simulation or a pass/fail prediction.
        </Text>
      </View>
      <Modal animationType="slide" onRequestClose={() => setImageZoomOpen(false)} visible={imageZoomOpen}>
        <View style={styles.zoomModal}>
          <Pressable accessibilityRole="button" onPress={() => setImageZoomOpen(false)} style={styles.closeButton}>
            <Text style={styles.closeButtonText}>Close image</Text>
          </Pressable>
          <ScrollView
            contentContainerStyle={styles.zoomContent}
            maximumZoomScale={3}
            minimumZoomScale={1}
            showsHorizontalScrollIndicator={false}
            showsVerticalScrollIndicator={false}
          >
            {visibleImageSource ? <Image accessibilityLabel="Zoomed practice image" resizeMode="contain" source={visibleImageSource} style={styles.zoomImage} /> : null}
          </ScrollView>
        </View>
      </Modal>
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
  photoCard: { backgroundColor: "#fffaf6", borderColor: "#e2d5ca", borderRadius: 18, borderWidth: 1, gap: 10, padding: 14 },
  photo: { backgroundColor: "#e6d8cc", borderRadius: 12, height: 230, width: "100%" },
  sceneCard: { alignItems: "center", backgroundColor: "#e9ddd2", borderRadius: 12, gap: 10, justifyContent: "center", minHeight: 230, padding: 24 },
  sceneEmoji: { color: "#a34f2d", fontSize: 56 },
  sceneText: { color: "#493d35", fontSize: 16, lineHeight: 23, textAlign: "center" },
  sceneNote: { color: "#8f6a58", fontSize: 11, lineHeight: 16, textAlign: "center" },
  photoTitle: { color: "#2d241f", fontSize: 18, fontWeight: "800" },
  provenance: { color: "#75685e", fontSize: 12, lineHeight: 18 },
  photoChoices: { flexDirection: "row", flexWrap: "wrap", gap: 7 },
  smallChoice: { borderColor: "#d9c9ba", borderRadius: 9, borderWidth: 1, minHeight: 40, justifyContent: "center", paddingHorizontal: 9 },
  smallChoiceSelected: { backgroundColor: "#fff1e7", borderColor: "#a34f2d" },
  smallChoiceText: { color: "#6f3c26", fontSize: 12, fontWeight: "700" },
  card: { backgroundColor: "#fffaf6", borderColor: "#e2d5ca", borderRadius: 18, borderWidth: 1, gap: 13, padding: 18 },
  resultCard: { backgroundColor: "#fffaf6", borderColor: "#e2d5ca", borderRadius: 18, borderWidth: 1, gap: 14, padding: 18 },
  sectionTitle: { color: "#2d241f", fontSize: 20, fontWeight: "800" },
  prompt: { color: "#2d241f", fontSize: 19, fontWeight: "800", lineHeight: 28 },
  modeLabel: { color: "#7c422c", fontSize: 12, fontWeight: "800" },
  helper: { color: "#75685e", fontSize: 13, lineHeight: 20 },
  notes: { backgroundColor: "#fffaf6", borderColor: "#d9c9ba", borderRadius: 12, borderWidth: 1, color: "#2d241f", fontSize: 15, lineHeight: 22, minHeight: 80, padding: 12 },
  primaryButton: { alignItems: "center", backgroundColor: "#a34f2d", borderRadius: 13, justifyContent: "center", minHeight: 50, paddingHorizontal: 16 },
  primaryButtonText: { color: "#fffaf6", fontSize: 15, fontWeight: "800" },
  secondaryButton: { alignItems: "center", borderColor: "#c9b8a8", borderRadius: 11, borderWidth: 1, justifyContent: "center", minHeight: 44, paddingHorizontal: 12 },
  secondaryButtonText: { color: "#6f3c26", fontSize: 13, fontWeight: "800" },
  timerBox: { alignItems: "center", backgroundColor: "#f1e2d6", borderRadius: 13, gap: 9, padding: 14 },
  timer: { color: "#2d241f", fontSize: 34, fontVariant: ["tabular-nums"], fontWeight: "900" },
  disabledButton: { backgroundColor: "#cbbdb1" },
  disabledChoice: { opacity: 0.55 },
  error: { color: "#8c302a", fontSize: 14, lineHeight: 20 },
  footer: { color: "#75685e", fontSize: 12, lineHeight: 18 },
  zoomModal: { backgroundColor: "#211a17", flex: 1, paddingTop: 44 },
  closeButton: { alignSelf: "flex-end", minHeight: 48, justifyContent: "center", paddingHorizontal: 18 },
  closeButtonText: { color: "#fffaf6", fontSize: 14, fontWeight: "800" },
  zoomContent: { alignItems: "center", flexGrow: 1, justifyContent: "center" },
  zoomImage: { height: 560, width: "100%" },
});
