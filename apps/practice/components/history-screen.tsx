import { useCallback, useEffect, useMemo, useState } from "react";
import { Alert, Linking, Pressable, ScrollView, StyleSheet, Text, TextInput, View } from "react-native";
import { File as ExpoFile, Paths } from "expo-file-system";
import * as Sharing from "expo-sharing";
import { strToU8, zipSync } from "fflate";

import { deleteRecordingFile, uriForRelativePath } from "@/lib/media/recording";
import {
  deleteRun,
  listRecordings,
  listAnalyses,
  listRounds,
  listRuns,
  parseSnapshot,
  type StoredRecording,
} from "@/lib/storage/repository";
import type { RecordingArtifact, StoredPracticeRound, StoredPracticeRun } from "@/lib/types";
import { errorMessage, logError, logWarn } from "@/lib/logging";
import { AudioPlayerCard } from "./audio-player-card";
import { DeliveryAnalysisPanel } from "./delivery-analysis-panel";

type HistoryScreenProps = { onBack?: () => void };

function modeLabel(mode: string): string {
  if (mode === "four-three-two") return "4–3–2";
  if (mode === "describe-photo") return "Describe a photo";
  return "Free speaking";
}

function formatDate(value: string): string {
  const date = new Date(value);
  return Number.isNaN(date.valueOf()) ? "Date unavailable" : date.toLocaleString();
}

function artifactFromStored(recording: StoredRecording): RecordingArtifact {
  const uri = recording.relativePath ? uriForRelativePath(recording.relativePath) : "";
  return {
    id: recording.id,
    uri,
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

export function HistoryScreen({ onBack }: HistoryScreenProps) {
  const [runs, setRuns] = useState<StoredPracticeRun[]>([]);
  const [rounds, setRounds] = useState<StoredPracticeRound[]>([]);
  const [recordings, setRecordings] = useState<StoredRecording[]>([]);
  const [analyses, setAnalyses] = useState<Awaited<ReturnType<typeof listAnalyses>>>([]);
  const [query, setQuery] = useState("");
  const [loading, setLoading] = useState(true);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [nextRuns, nextRounds, nextRecordings, nextAnalyses] = await Promise.all([listRuns(), listRounds(), listRecordings(), listAnalyses()]);
      setRuns(nextRuns);
      setRounds(nextRounds);
      setRecordings(nextRecordings);
      setAnalyses(nextAnalyses);
    } catch (loadError) {
      logError("history.load", loadError);
      setError(`History could not be loaded: ${errorMessage(loadError)}`);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const visibleRuns = useMemo(() => {
    const normalized = query.trim().toLowerCase();
    if (!normalized) return runs;
    return runs.filter((run) => {
      const snapshot = parseSnapshot(run);
      return `${snapshot?.title ?? ""} ${snapshot?.prompt ?? ""} ${run.mode}`.toLowerCase().includes(normalized);
    });
  }, [query, runs]);

  const exportRun = async (run: StoredPracticeRun) => {
    setError(null);
    try {
      const snapshot = parseSnapshot(run);
      const runRounds = rounds.filter((round) => round.runId === run.id);
      const runRecordings = recordings.filter((recording) => runRounds.some((round) => round.id === recording.roundId));
      const runAnalyses = analyses.filter((analysis) => runRecordings.some((recording) => recording.id === analysis.recordingId));
      const files: Record<string, Uint8Array> = {};
      const audioEntries: Array<{ recording_id: string; path: string; included: boolean }> = [];
      let missingAudioCount = 0;

      for (const recording of runRecordings) {
        const extension = recording.mimeType.includes("m4a") || recording.mimeType.includes("mp4") ? "m4a" : recording.mimeType.includes("ogg") || recording.mimeType.includes("opus") ? "ogg" : "webm";
        const path = `audio/${recording.id}.${extension}`;
        let included = false;
        if (recording.mediaAvailability === "ready" && recording.relativePath) {
          try {
            const bytes = await new ExpoFile(uriForRelativePath(recording.relativePath)).arrayBuffer();
            files[path] = new Uint8Array(bytes);
            included = true;
          } catch (audioError) {
            logWarn("history.export", "Audio was unavailable during export", { recordingId: recording.id, error: errorMessage(audioError) });
          }
        }
        if (!included) missingAudioCount += 1;
        audioEntries.push({ recording_id: recording.id, path, included });
      }

      const manifest = {
        export_version: 2,
        exported_at: new Date().toISOString(),
        run,
        task_snapshot: snapshot,
        rounds: runRounds,
        recordings: runRecordings,
        analyses: runAnalyses,
        audio: audioEntries,
        note: "Audio and metadata in this ZIP are a portable export. Exported files are outside the managed device quota; a missing audio entry means the local file was already evicted or unavailable.",
      };
      files["manifest.json"] = strToU8(JSON.stringify(manifest, null, 2));
      const archive = zipSync(files, { level: 6 });

      if (process.env.EXPO_OS === "web") {
        const blob = new Blob([archive], { type: "application/zip" });
        const url = URL.createObjectURL(blob);
        const link = document.createElement("a");
        link.href = url;
        link.download = `aprendiendo-${run.id}.zip`;
        link.click();
        URL.revokeObjectURL(url);
      } else {
        const file = new ExpoFile(Paths.cache, `aprendiendo-${run.id}.zip`);
        file.create({ overwrite: true, intermediates: true });
        file.write(archive);
        if (await Sharing.isAvailableAsync()) {
          await Sharing.shareAsync(file.uri, { mimeType: "application/zip", dialogTitle: "Export practice run" });
        } else {
          await Linking.openURL(file.uri);
        }
      }
      setMessage(missingAudioCount ? `Practice exported, but ${missingAudioCount} audio file(s) were unavailable; see manifest.json.` : "Practice exported successfully.");
    } catch (exportError) {
      logError("history.export", exportError, { runId: run.id });
      setError(`Practice export failed: ${errorMessage(exportError)}`);
    }
  };

  const removeRun = (run: StoredPracticeRun) => {
    const remove = async () => {
      setError(null);
      try {
        const removed = await deleteRun(run.id);
        removed.forEach((recording) => deleteRecordingFile(artifactFromStored(recording)));
        setMessage("Practice and its local results were deleted. Any queued server cleanup will resume when connected.");
        await refresh();
      } catch (deleteError) {
        logError("history.delete", deleteError, { runId: run.id });
        setError(`Practice deletion failed: ${errorMessage(deleteError)}`);
      }
    };
    if (process.env.EXPO_OS === "web") {
      if (globalThis.confirm?.("Delete this practice run and its saved results?")) void remove();
    } else {
      Alert.alert("Delete practice?", "This removes the local audio, task, results, and metadata.", [
        { text: "Cancel", style: "cancel" },
        { text: "Delete", style: "destructive", onPress: () => void remove() },
      ]);
    }
  };

  return (
    <ScrollView
      contentInsetAdjustmentBehavior="automatic"
      contentContainerStyle={styles.scrollContent}
      style={styles.scroll}
    >
      <View style={styles.page}>
        {onBack ? <Pressable onPress={onBack} style={styles.backButton}><Text style={styles.backText}>‹ Practice</Text></Pressable> : null}
        <View style={styles.header}>
          <Text style={styles.title} selectable>History</Text>
          <Text style={styles.subtitle} selectable>Saved practice remains readable offline. Audio may later be removed by the local quota.</Text>
        </View>
        <TextInput
          accessibilityLabel="Search practice history"
          onChangeText={setQuery}
          placeholder="Search topics or modes"
          placeholderTextColor="#87796d"
          style={styles.search}
          value={query}
        />
        {message ? <Text style={styles.message} selectable>{message}</Text> : null}
        {error ? <Text accessibilityLiveRegion="polite" style={styles.error} selectable>{error}</Text> : null}
        {loading ? <Text style={styles.helper} selectable>Loading local history…</Text> : null}
        {!loading && !visibleRuns.length ? (
          <View style={styles.empty}><Text style={styles.emptyTitle} selectable>No saved practice yet</Text><Text style={styles.helper} selectable>Finish a round and it will appear here, even when the device is offline.</Text></View>
        ) : null}
        {visibleRuns.map((run) => {
          const snapshot = parseSnapshot(run);
          const runRounds = rounds.filter((round) => round.runId === run.id).sort((a, b) => a.sequence - b.sequence);
          const runRecordings = recordings.filter((recording) => runRounds.some((round) => round.id === recording.roundId));
          return (
            <View key={run.id} style={styles.runCard}>
              <View style={styles.runHeader}>
                <View style={styles.runTitleBlock}>
                  <Text style={styles.runTitle} selectable>{snapshot?.title ?? modeLabel(run.mode)}</Text>
                  <Text style={styles.runDate} selectable>{formatDate(run.updatedAt)} · {run.status}</Text>
                </View>
                <Text style={styles.runMode} selectable>{modeLabel(run.mode)}</Text>
              </View>
              <Text style={styles.prompt} selectable>{snapshot?.prompt ?? "Task snapshot unavailable"}</Text>
              {runRounds.map((round) => {
                const roundRecording = runRecordings.find((recording) => recording.roundId === round.id);
                return (
                  <View key={round.id} style={styles.roundRow}>
                    <View style={styles.roundMeta}>
                      <Text style={styles.roundLabel} selectable>Round {round.sequence + 1} · {round.status}</Text>
                      <Text style={styles.roundDetail} selectable>
                        {roundRecording
                          ? `${((roundRecording.decodedDurationMs ?? roundRecording.clientDurationMs) / 1000).toFixed(1)}s · ${roundRecording.bytes.toLocaleString()} bytes · ${roundRecording.mediaAvailability}`
                          : "No finalized recording"}
                      </Text>
                    </View>
                    {roundRecording && roundRecording.mediaAvailability === "ready" && roundRecording.relativePath ? (
                      <AudioPlayerCard
                        artifact={artifactFromStored(roundRecording)}
                        label={`Round ${round.sequence + 1}`}
                        onDownload={() => void exportRun(run)}
                        onPlayed={() => undefined}
                        seekRequest={null}
                      />
                    ) : null}
                    {roundRecording ? <DeliveryAnalysisPanel recordingId={roundRecording.id} /> : null}
                  </View>
                );
              })}
              <View style={styles.actions}>
                <Pressable onPress={() => void exportRun(run)} style={styles.secondaryButton}><Text style={styles.secondaryText}>Export run</Text></Pressable>
                <Pressable onPress={() => removeRun(run)} style={styles.deleteButton}><Text style={styles.deleteText}>Delete practice</Text></Pressable>
              </View>
            </View>
          );
        })}
      </View>
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  scroll: { backgroundColor: "#f7efe8", flex: 1 },
  scrollContent: { paddingBottom: 48, paddingHorizontal: 18, paddingTop: 20 },
  page: { alignSelf: "center", gap: 15, maxWidth: 860, width: "100%" },
  backButton: { alignSelf: "flex-start", minHeight: 44, justifyContent: "center" },
  backText: { color: "#6f3c26", fontSize: 14, fontWeight: "800" },
  header: { gap: 5 },
  title: { color: "#2d241f", fontSize: 30, fontWeight: "900" },
  subtitle: { color: "#75685e", fontSize: 15, lineHeight: 22 },
  search: { backgroundColor: "#fffaf6", borderColor: "#d9c9ba", borderRadius: 12, borderWidth: 1, color: "#2d241f", fontSize: 15, minHeight: 48, paddingHorizontal: 13 },
  message: { backgroundColor: "#edf6ee", borderRadius: 10, color: "#356345", fontSize: 13, lineHeight: 19, padding: 11 },
  error: { backgroundColor: "#fbe8e5", borderColor: "#c86a5c", borderRadius: 10, borderWidth: 1, color: "#8c302a", fontFamily: "monospace", fontSize: 13, lineHeight: 19, padding: 11 },
  helper: { color: "#75685e", fontSize: 13, lineHeight: 20 },
  empty: { backgroundColor: "#f1e8e1", borderRadius: 15, gap: 6, padding: 17 },
  emptyTitle: { color: "#493d35", fontSize: 17, fontWeight: "800" },
  runCard: { backgroundColor: "#fffaf6", borderColor: "#e2d5ca", borderRadius: 18, borderWidth: 1, gap: 13, padding: 16 },
  runHeader: { alignItems: "flex-start", flexDirection: "row", gap: 10, justifyContent: "space-between" },
  runTitleBlock: { flex: 1, gap: 3 },
  runTitle: { color: "#2d241f", fontSize: 18, fontWeight: "800" },
  runDate: { color: "#75685e", fontSize: 12 },
  runMode: { backgroundColor: "#f1e2d6", borderRadius: 99, color: "#7c422c", fontSize: 11, fontWeight: "800", overflow: "hidden", paddingHorizontal: 9, paddingVertical: 5 },
  prompt: { color: "#493d35", fontSize: 14, lineHeight: 21 },
  roundRow: { borderColor: "#eaded5", borderTopWidth: 1, gap: 8, paddingTop: 12 },
  roundMeta: { gap: 3 },
  roundLabel: { color: "#493d35", fontSize: 14, fontWeight: "800" },
  roundDetail: { color: "#75685e", fontSize: 12, lineHeight: 18 },
  actions: { flexDirection: "row", flexWrap: "wrap", gap: 9 },
  secondaryButton: { alignItems: "center", borderColor: "#c9b8a8", borderRadius: 10, borderWidth: 1, justifyContent: "center", minHeight: 44, paddingHorizontal: 12 },
  secondaryText: { color: "#6f3c26", fontSize: 13, fontWeight: "800" },
  deleteButton: { alignItems: "center", borderColor: "#b94c42", borderRadius: 10, borderWidth: 1, justifyContent: "center", minHeight: 44, paddingHorizontal: 12 },
  deleteText: { color: "#8c302a", fontSize: 13, fontWeight: "800" },
});
