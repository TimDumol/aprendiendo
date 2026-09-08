import { useCallback, useEffect, useRef, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";
import {
  AudioModule,
  IOSOutputFormat,
  RecordingPresets,
  setAudioModeAsync,
  useAudioRecorder,
  useAudioRecorderState,
} from "expo-audio";
import { activateKeepAwakeAsync, deactivateKeepAwake } from "expo-keep-awake";

import {
  DEFAULT_CAPTURE_RESERVATION_BYTES,
  finalizeCapturedRecording,
  guessMimeType,
} from "@/lib/media/recording";
import { errorMessage, logError, logWarn } from "@/lib/logging";
import { completeMediaOperation, recordMediaArtifact, reserveMediaBytes, updateRoundStatus } from "@/lib/storage/repository";
import type { RecordingArtifact } from "@/lib/types";

const RECORDING_OPTIONS = {
  ...RecordingPresets.HIGH_QUALITY,
  directory: "document" as const,
  numberOfChannels: 1,
  bitRate: 64_000,
  isMeteringEnabled: true,
  android: {
    ...RecordingPresets.HIGH_QUALITY.android,
    extension: ".m4a",
    outputFormat: "mpeg4" as const,
    audioEncoder: "aac" as const,
    maxFileSize: 10 * 1024 * 1024,
  },
  ios: {
    ...RecordingPresets.HIGH_QUALITY.ios,
    extension: ".m4a",
    outputFormat: IOSOutputFormat.MPEG4AAC,
    audioQuality: 64,
  },
  web: {
    ...RecordingPresets.HIGH_QUALITY.web,
    mimeType: "audio/webm",
    bitsPerSecond: 32_000,
  },
};

const MIN_DURATION_MS = 10_000;
const DEFAULT_MAX_DURATION_MS = 600_000;
const MAX_AUDIO_BYTES = 10 * 1024 * 1024;
const IS_WEB = process.env.EXPO_OS === "web";
const METER_MIN_DB = -60;
const METER_MAX_DB = 0;
const WAVEFORM_SHAPE = [
  0.32, 0.55, 0.78, 0.48, 0.92, 0.66, 0.4, 0.72, 0.52, 0.88, 0.6, 0.38,
  0.7, 0.95, 0.58, 0.42, 0.76, 0.5, 0.86, 0.62, 0.36, 0.68, 0.82, 0.46,
];

type RecordingPanelProps = {
  attemptLabel: string;
  disabled: boolean;
  onRecorded: (artifact: RecordingArtifact) => void | Promise<void>;
  onRecordingStateChange: (recording: boolean) => void;
  roundId?: string;
  targetDurationMs?: number;
  minDurationMs?: number;
  maxDurationMs?: number;
};

function formatTimer(durationMs: number): string {
  const totalSeconds = Math.max(0, Math.floor(durationMs / 1000));
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `${minutes}:${String(seconds).padStart(2, "0")}`;
}

async function readFileDurationMs(blob: Blob): Promise<number> {
  if (!IS_WEB || typeof document === "undefined") return 0;

  const probeUrl = URL.createObjectURL(blob);
  const audio = document.createElement("audio");
  audio.preload = "metadata";
  audio.src = probeUrl;

  try {
    return await new Promise<number>((resolve) => {
      audio.onloadedmetadata = () => {
        resolve(Number.isFinite(audio.duration) ? Math.round(audio.duration * 1000) : 0);
      };
      audio.onerror = () => resolve(0);
    });
  } finally {
    URL.revokeObjectURL(probeUrl);
  }
}

export function RecordingPanel({
  attemptLabel,
  disabled,
  onRecorded,
  onRecordingStateChange,
  roundId = "unsaved-round",
  targetDurationMs,
  minDurationMs = MIN_DURATION_MS,
  maxDurationMs = DEFAULT_MAX_DURATION_MS,
}: RecordingPanelProps) {
  const recorder = useAudioRecorder(RECORDING_OPTIONS);
  const recorderState = useAudioRecorderState(recorder, 200);
  const [wallDurationMs, setWallDurationMs] = useState(0);
  const [permissionError, setPermissionError] = useState<string | null>(null);
  const [captureError, setCaptureError] = useState<string | null>(null);
  const [partialWarning, setPartialWarning] = useState<string | null>(null);
  const [isFinalizing, setIsFinalizing] = useState(false);
  const startedAtRef = useRef<number | null>(null);
  const finalizingRef = useRef(false);
  const interruptionHandledRef = useRef(false);
  const maxStopHandledRef = useRef(false);
  const reservationRef = useRef<string | null>(null);

  const durationMs = Math.max(
    recorderState.durationMillis || 0,
    recorderState.isRecording ? wallDurationMs : 0,
  );
  const isRecording = recorderState.isRecording;
  const meteringAvailable =
    typeof recorderState.metering === "number" && Number.isFinite(recorderState.metering);
  const meteringDb = meteringAvailable
    ? Math.max(METER_MIN_DB, Math.min(METER_MAX_DB, recorderState.metering ?? METER_MIN_DB))
    : METER_MIN_DB;
  const meterProgress = (meteringDb - METER_MIN_DB) / (METER_MAX_DB - METER_MIN_DB);
  const meterLabel = !isRecording
    ? "Mic check starts when you record"
    : !meteringAvailable
      ? "Listening for microphone level…"
      : meteringDb <= METER_MIN_DB
        ? "No signal detected yet"
        : `${Math.round(meteringDb)} dBFS`;

  useEffect(() => {
    onRecordingStateChange(isRecording || isFinalizing);
  }, [isFinalizing, isRecording, onRecordingStateChange]);

  useEffect(() => {
    if (isRecording || isFinalizing) {
      void activateKeepAwakeAsync("practice-recording").catch((error) => logWarn("recording.keep-awake", "Could not keep the screen awake", { error: errorMessage(error) }));
    } else {
      void deactivateKeepAwake("practice-recording").catch((error) => logWarn("recording.keep-awake", "Could not release the keep-awake lock", { error: errorMessage(error) }));
    }
    return () => {
      void deactivateKeepAwake("practice-recording").catch((error) => logWarn("recording.keep-awake", "Could not release the keep-awake lock", { error: errorMessage(error) }));
    };
  }, [isFinalizing, isRecording]);

  useEffect(() => {
    if (!isRecording || !startedAtRef.current) {
      setWallDurationMs(0);
      return;
    }

    const update = () => {
      setWallDurationMs(Date.now() - (startedAtRef.current ?? Date.now()));
    };
    update();
    const timer = setInterval(update, 200);
    return () => clearInterval(timer);
  }, [isRecording]);

  const finishRecording = useCallback(
    async (warning?: string) => {
      if (finalizingRef.current) return;
      finalizingRef.current = true;
      setIsFinalizing(true);
      setCaptureError(null);
      if (roundId !== "unsaved-round") void updateRoundStatus(roundId, "finalizing");

      const reportedDurationMs = Math.round(
        recorderState.durationMillis ||
          recorder.currentTime * 1000 ||
          wallDurationMs ||
          (startedAtRef.current ? Date.now() - startedAtRef.current : 0),
      );

      try {
        await recorder.stop();
        const uri = recorder.uri ?? recorderState.url;
        if (!uri) throw new Error("The recorder did not return a file URL.");

        const artifact = await finalizeCapturedRecording({
          id: `recording-${Date.now()}`,
          uri,
          mimeType: IS_WEB ? guessMimeType(uri) : "audio/m4a",
          clientDurationMs: Math.max(0, reportedDurationMs),
          interrupted: Boolean(warning),
          warning,
        });
        if (reservationRef.current && artifact.relativePath) {
          await recordMediaArtifact(reservationRef.current, artifact.relativePath);
        }
        await onRecorded(artifact);
        if (artifact.bytes > MAX_AUDIO_BYTES) {
          setCaptureError("This take is larger than the 10 MiB submission limit.");
        } else if (reportedDurationMs < minDurationMs) {
          setCaptureError("This take is shorter than 10 seconds, so feedback is disabled.");
        } else if (reportedDurationMs > maxDurationMs) {
          setCaptureError("This take is longer than the selected exercise limit, so feedback is disabled.");
        }
        if (warning) setPartialWarning(warning);
        if (reservationRef.current) {
          await completeMediaOperation(reservationRef.current, "committed");
          reservationRef.current = null;
        }
      } catch (error) {
        logError("recording.finalize", error, { roundId, warning: warning ?? null });
        setCaptureError(
          `The recording could not be finalized: ${errorMessage(error)}. The candidate was not marked Saved; try again or recover it from the next launch.`,
        );
        if (roundId !== "unsaved-round") void updateRoundStatus(roundId, "interrupted", true);
        if (warning) setPartialWarning(warning);
        if (reservationRef.current) {
          await completeMediaOperation(reservationRef.current, "failed");
          reservationRef.current = null;
        }
      } finally {
        await setAudioModeAsync({ allowsRecording: false, playsInSilentMode: true }).catch(() => undefined);
        startedAtRef.current = null;
        setWallDurationMs(0);
        setIsFinalizing(false);
        finalizingRef.current = false;
      }
    },
    [
      maxDurationMs,
      minDurationMs,
      onRecorded,
      recorder,
      recorderState.durationMillis,
      recorderState.url,
      roundId,
      wallDurationMs,
    ],
  );

  useEffect(() => {
    const elapsed = Math.max(recorderState.durationMillis || 0, wallDurationMs);
    if (isRecording && elapsed >= maxDurationMs && !maxStopHandledRef.current) {
      maxStopHandledRef.current = true;
      void finishRecording("The take stopped automatically at the selected exercise limit.");
    }
    if (!isRecording) maxStopHandledRef.current = false;
  }, [finishRecording, isRecording, maxDurationMs, recorderState.durationMillis, wallDurationMs]);

  useEffect(() => {
    if (
      recorderState.mediaServicesDidReset &&
      isRecording &&
      !interruptionHandledRef.current
    ) {
      interruptionHandledRef.current = true;
      void finishRecording(
        "Recording interrupted. This partial take was finalized when possible; listen before submitting.",
      );
    }
    if (!isRecording) interruptionHandledRef.current = false;
  }, [finishRecording, isRecording, recorderState.mediaServicesDidReset]);

  const startRecording = async () => {
    if (disabled || isFinalizing) return;
    setPermissionError(null);
    setCaptureError(null);
    setPartialWarning(null);

    if (
      IS_WEB &&
      (typeof navigator === "undefined" ||
        !navigator.mediaDevices?.getUserMedia ||
        typeof MediaRecorder === "undefined")
    ) {
      setCaptureError(
        "This browser cannot capture a microphone recording. Use the labeled audio-file fallback below or try a current desktop browser.",
      );
      return;
    }

    try {
      const permission = await AudioModule.requestRecordingPermissionsAsync();
      if (!permission.granted) {
        setPermissionError("Microphone access was denied. Enable microphone access for Aprendiendo in the device or browser settings, then try Record again.");
        return;
      }
      await setAudioModeAsync({ allowsRecording: true, playsInSilentMode: true });
      await recorder.prepareToRecordAsync();
      reservationRef.current = await reserveMediaBytes(roundId, DEFAULT_CAPTURE_RESERVATION_BYTES);
      if (roundId !== "unsaved-round") await updateRoundStatus(roundId, "recording");
      startedAtRef.current = Date.now();
      maxStopHandledRef.current = false;
      interruptionHandledRef.current = false;
      recorder.record(targetDurationMs ? { forDuration: targetDurationMs / 1000 } : undefined);
    } catch (error) {
      logError("recording.start", error, { roundId });
      setCaptureError(`The microphone could not start: ${errorMessage(error)}. Check the device permission and try again.`);
    }
  };

  const stopRecording = () => {
    if (isRecording) void finishRecording();
  };

  const chooseAudioFile = () => {
    if (!IS_WEB || typeof document === "undefined") return;
    const input = document.createElement("input");
    input.type = "file";
    input.accept = "audio/webm,audio/ogg,audio/opus,audio/mp4,audio/m4a";
    input.onchange = () => {
      const file = input.files?.[0];
      if (!file) return;
      void (async () => {
        try {
          const duration = await readFileDurationMs(file);
          if (!duration) {
            setCaptureError("That audio file has no readable duration. Choose another file.");
            return;
          }
          const objectUrl = URL.createObjectURL(file);
          await Promise.resolve(onRecorded({
            id: `uploaded-${Date.now()}`,
            uri: objectUrl,
            blob: file,
            mimeType: file.type || guessMimeType(file.name),
            container: file.type.includes("mp4") || file.type.includes("m4a") ? "m4a/mp4" : "web",
            codec: file.type.includes("opus") ? "opus" : "browser-reported",
            durationMs: duration,
            clientDurationMs: duration,
            decodedDurationMs: duration,
            bytes: file.size,
            hash: `web-file-${file.size}-${file.lastModified}`,
            createdOrder: Date.now(),
            mediaAvailability: "ready",
            objectUrl,
            warning: "Audio-file fallback selected; microphone capture was not verified.",
          }));
        } catch (error) {
          logError("recording.file-fallback", error, { fileName: file.name, bytes: file.size });
          setCaptureError(`The selected audio file could not be imported: ${errorMessage(error)}`);
        }
      })();
    };
    input.click();
  };

  const webCaptureUnavailable =
    IS_WEB &&
    typeof navigator !== "undefined" &&
    (!navigator.mediaDevices?.getUserMedia || typeof MediaRecorder === "undefined");

  return (
    <View style={styles.container}>
      <View style={styles.statusRow}>
        <View style={styles.statusLabel}>
          <View style={[styles.statusDot, isRecording && styles.statusDotLive]} />
          <Text style={styles.statusText} selectable>
            {isFinalizing ? "Finalizing take" : isRecording ? "Recording" : attemptLabel}
          </Text>
        </View>
        <Text accessibilityLabel="Recording duration" style={styles.timer} selectable>
          {formatTimer(durationMs)}
        </Text>
      </View>

      <View
        accessibilityLabel={`Live microphone level: ${meterLabel}`}
        style={styles.meterCard}
      >
        <View style={styles.meterHeader}>
          <Text style={styles.meterTitle}>Live mic level</Text>
          <Text style={styles.meterValue} selectable>
            {meterLabel}
          </Text>
        </View>
        <View
          accessibilityRole="progressbar"
          accessibilityValue={{ max: 0, min: METER_MIN_DB, now: meteringDb }}
          style={styles.waveform}
        >
          {WAVEFORM_SHAPE.map((shape, index) => {
            const barProgress = Math.min(1, meterProgress * 1.35) * shape;
            const isActive = isRecording && meterProgress > index / WAVEFORM_SHAPE.length;
            return (
              <View
                key={index}
                style={[
                  styles.waveBar,
                  {
                    backgroundColor: isActive
                      ? index > WAVEFORM_SHAPE.length * 0.82
                        ? "#bf3e2d"
                        : index > WAVEFORM_SHAPE.length * 0.62
                          ? "#d48732"
                          : "#3f9b75"
                      : "#d9ccc2",
                    height: 8 + barProgress * 35,
                  },
                ]}
              />
            );
          })}
        </View>
        <Text style={styles.meterHint} selectable>
          Speak normally. The bars should move; a flat “No signal” usually means the mic is muted or permission was denied.
        </Text>
      </View>

      {isRecording ? (
        <Pressable
          accessibilityLabel="Stop recording"
          accessibilityRole="button"
          onPress={stopRecording}
          style={({ pressed }) => [styles.stopButton, pressed && styles.buttonPressed]}
        >
          <Text style={styles.stopButtonText}>Stop</Text>
        </Pressable>
      ) : (
        <Pressable
          accessibilityLabel={disabled ? "This attempt already has a recording" : "Record take"}
          accessibilityRole="button"
          disabled={disabled || isFinalizing}
          onPress={() => void startRecording()}
          style={({ pressed }) => [
            styles.recordButton,
            (disabled || isFinalizing) && styles.disabledButton,
            pressed && !disabled && styles.buttonPressed,
          ]}
        >
          <Text style={styles.recordButtonText}>{disabled ? "Take saved" : "Record"}</Text>
        </Pressable>
      )}

      <Text style={styles.helperText} selectable>
        {targetDurationMs
          ? `Target ${formatTimer(targetDurationMs)} · continuous take · native elapsed timer`
          : "Aim for 1–2 minutes · continuous take · native elapsed timer"}
      </Text>

      {permissionError ? (
        <Text accessibilityLiveRegion="polite" style={styles.errorText} selectable>
          {permissionError}
        </Text>
      ) : null}
      {captureError ? (
        <Text accessibilityLiveRegion="polite" style={styles.errorText} selectable>
          {captureError}
        </Text>
      ) : null}
      {partialWarning ? (
        <Text style={styles.warningText} selectable>
          {partialWarning}
        </Text>
      ) : null}

      {IS_WEB && !disabled && (webCaptureUnavailable || Boolean(captureError)) ? (
        <Pressable
          accessibilityLabel="Choose an audio file as a labeled fallback"
          accessibilityRole="button"
          onPress={chooseAudioFile}
          style={({ pressed }) => [styles.fallbackButton, pressed && styles.buttonPressed]}
        >
          <Text style={styles.fallbackButtonText}>Choose audio file (fallback)</Text>
        </Pressable>
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    gap: 14,
  },
  statusRow: {
    alignItems: "center",
    flexDirection: "row",
    justifyContent: "space-between",
  },
  statusLabel: {
    alignItems: "center",
    flexDirection: "row",
    gap: 9,
  },
  statusDot: {
    backgroundColor: "#9c8d81",
    borderRadius: 99,
    height: 11,
    width: 11,
  },
  statusDotLive: {
    backgroundColor: "#bf3e2d",
  },
  statusText: {
    color: "#493d35",
    fontSize: 15,
    fontWeight: "700",
  },
  timer: {
    color: "#2d241f",
    fontSize: 28,
    fontVariant: ["tabular-nums"],
    fontWeight: "800",
  },
  recordButton: {
    alignItems: "center",
    backgroundColor: "#a34f2d",
    borderRadius: 16,
    justifyContent: "center",
    minHeight: 56,
    paddingHorizontal: 18,
  },
  recordButtonText: {
    color: "#fffaf6",
    fontSize: 18,
    fontWeight: "800",
  },
  stopButton: {
    alignItems: "center",
    backgroundColor: "#5c2a27",
    borderRadius: 16,
    justifyContent: "center",
    minHeight: 56,
    paddingHorizontal: 18,
  },
  stopButtonText: {
    color: "#fffaf6",
    fontSize: 18,
    fontWeight: "800",
  },
  buttonPressed: {
    opacity: 0.78,
  },
  disabledButton: {
    backgroundColor: "#cbbdb1",
  },
  helperText: {
    color: "#75685e",
    fontSize: 13,
    lineHeight: 19,
    textAlign: "center",
  },
  meterCard: {
    backgroundColor: "#fbf7f3",
    borderColor: "#eaded5",
    borderRadius: 14,
    borderWidth: 1,
    gap: 9,
    padding: 13,
  },
  meterHeader: {
    alignItems: "center",
    flexDirection: "row",
    justifyContent: "space-between",
  },
  meterTitle: {
    color: "#493d35",
    fontSize: 13,
    fontWeight: "800",
    letterSpacing: 0.2,
  },
  meterValue: {
    color: "#75685e",
    fontSize: 12,
    fontVariant: ["tabular-nums"],
    fontWeight: "700",
  },
  waveform: {
    alignItems: "center",
    backgroundColor: "#f1e9e3",
    borderRadius: 9,
    flexDirection: "row",
    gap: 3,
    height: 55,
    justifyContent: "center",
    overflow: "hidden",
    paddingHorizontal: 10,
  },
  waveBar: {
    borderRadius: 99,
    minHeight: 8,
    width: 4,
  },
  meterHint: {
    color: "#8a7b70",
    fontSize: 11,
    lineHeight: 16,
  },
  errorText: {
    color: "#8c302a",
    fontSize: 14,
    lineHeight: 20,
  },
  warningText: {
    backgroundColor: "#fff2ce",
    borderRadius: 10,
    color: "#6b4b18",
    fontSize: 14,
    lineHeight: 20,
    padding: 12,
  },
  fallbackButton: {
    alignItems: "center",
    borderColor: "#b95f38",
    borderRadius: 12,
    borderWidth: 1,
    justifyContent: "center",
    minHeight: 46,
    paddingHorizontal: 12,
  },
  fallbackButtonText: {
    color: "#8f4325",
    fontSize: 14,
    fontWeight: "700",
  },
});
