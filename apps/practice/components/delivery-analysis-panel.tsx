import { useEffect, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { listAnalyses } from "@/lib/storage/repository";
import { errorMessage, logError } from "@/lib/logging";
import type { StoredAnalysis } from "@/lib/types";

type DeliveryMetrics = {
  duration_seconds?: number;
  response_span_seconds?: number | null;
  speech_active_seconds?: number | null;
  internal_pause_count?: number | null;
  long_pause_count?: number | null;
  pause_time_seconds?: number | null;
  pause_frequency_per_minute?: number | null;
  typical_pause_seconds?: number | null;
  longest_pause_seconds?: number | null;
  speech_intervals?: Array<{ start_seconds: number; end_seconds: number }>;
  limitations?: string[];
};

type DeliveryAnalysisPanelProps = {
  recordingId: string;
  onPlayMoment?: (seconds: number) => void;
};

function readMetrics(analysis: StoredAnalysis): DeliveryMetrics | null {
  try {
    const value = JSON.parse(analysis.metricsJson) as unknown;
    return value && typeof value === "object" ? (value as DeliveryMetrics) : null;
  } catch {
    return null;
  }
}

function readLimitations(analysis: StoredAnalysis): string[] {
  try {
    const value = JSON.parse(analysis.limitationsJson) as unknown;
    return Array.isArray(value) ? value.filter((item): item is string => typeof item === "string") : [];
  } catch {
    return [];
  }
}

function metric(value: number | null | undefined, digits = 1): string {
  return value === null || value === undefined || !Number.isFinite(value) ? "—" : value.toFixed(digits);
}

export function DeliveryAnalysisPanel({ recordingId, onPlayMoment }: DeliveryAnalysisPanelProps) {
  const [analysis, setAnalysis] = useState<StoredAnalysis | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    setError(null);
    void listAnalyses(recordingId).then((items) => {
      if (active) setAnalysis(items[0] ?? null);
    }).catch((loadError) => {
      logError("delivery-analysis.load", loadError, { recordingId });
      if (active) {
        setAnalysis(null);
        setError(`Delivery measurements could not be loaded: ${errorMessage(loadError)}`);
      }
    });
    return () => {
      active = false;
    };
  }, [recordingId]);

  if (error) {
    return <View style={styles.container}><Text style={styles.error} selectable>{error}</Text><Text style={styles.helper} selectable>The recording remains available; reload this screen to retry.</Text></View>;
  }
  if (!analysis) {
    return <Text style={styles.helper} selectable>Delivery measurements are pending connection. The take remains playable and queued work can resume after restart.</Text>;
  }

  const metrics = readMetrics(analysis);
  const limitations = [...readLimitations(analysis), ...(metrics?.limitations ?? [])];
  if (!metrics) {
    return (
      <View style={styles.container}>
        <Text style={styles.title} selectable>Delivery analysis · partial</Text>
        {limitations.map((item) => <Text key={item} style={styles.helper} selectable>• {item}</Text>)}
      </View>
    );
  }

  const speechIntervals = metrics.speech_intervals ?? [];
  const pauses = speechIntervals.slice(0, -1).map((interval, index) => ({
    start: interval.end_seconds,
    end: speechIntervals[index + 1]?.start_seconds ?? interval.end_seconds,
  })).filter((pause) => pause.end - pause.start >= 0.5);
  return (
    <View style={styles.container}>
      <Text style={styles.title} selectable>Detected pauses</Text>
      <Text style={styles.method} selectable>{analysis.processorVersion} · {analysis.modelVersion} · {metric(metrics.duration_seconds)}s decoded</Text>
      <View style={styles.grid}>
        <Text style={styles.metric} selectable>Internal pauses: {metrics.internal_pause_count ?? "—"}</Text>
        <Text style={styles.metric} selectable>Long pauses: {metrics.long_pause_count ?? "—"}</Text>
        <Text style={styles.metric} selectable>Pause time: {metric(metrics.pause_time_seconds)}s</Text>
        <Text style={styles.metric} selectable>Pause frequency: {metric(metrics.pause_frequency_per_minute)} / min</Text>
        <Text style={styles.metric} selectable>Typical pause: {metric(metrics.typical_pause_seconds)}s</Text>
        <Text style={styles.metric} selectable>Longest pause: {metric(metrics.longest_pause_seconds)}s</Text>
      </View>
      {onPlayMoment && pauses.length ? (
        <View style={styles.pauseRow}>
          {pauses.slice(0, 8).map((pause, index) => (
            <Pressable key={`${pause.start}-${index}`} onPress={() => onPlayMoment(Math.max(0, pause.start - 0.5))} style={styles.pauseButton}>
              <Text style={styles.pauseText}>Pause {index + 1} · {pause.end - pause.start >= 2 ? "long" : ""}</Text>
            </Pressable>
          ))}
        </View>
      ) : null}
      <Text style={styles.helper} selectable>These are detector intervals and non-speech gaps. They do not diagnose thinking, breathing, hesitation, or language quality.</Text>
      {limitations.map((item) => <Text key={item} style={styles.helper} selectable>• {item}</Text>)}
    </View>
  );
}

const styles = StyleSheet.create({
  container: { backgroundColor: "#f8f0eb", borderRadius: 12, gap: 8, padding: 12 },
  title: { color: "#493d35", fontSize: 15, fontWeight: "800" },
  method: { color: "#8a7b70", fontSize: 11, lineHeight: 16 },
  grid: { flexDirection: "row", flexWrap: "wrap", gap: 8 },
  metric: { color: "#6f3c26", fontSize: 12, fontVariant: ["tabular-nums"], lineHeight: 18, minWidth: "46%" },
  helper: { color: "#75685e", fontSize: 12, lineHeight: 18 },
  error: { color: "#8c302a", fontSize: 13, lineHeight: 19 },
  pauseRow: { flexDirection: "row", flexWrap: "wrap", gap: 6 },
  pauseButton: { borderColor: "#c9b8a8", borderRadius: 8, borderWidth: 1, minHeight: 36, justifyContent: "center", paddingHorizontal: 9 },
  pauseText: { color: "#6f3c26", fontSize: 11, fontWeight: "700" },
});
