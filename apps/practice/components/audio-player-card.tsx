import { useEffect, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";
import Slider from "@react-native-community/slider";
import { useAudioPlayer, useAudioPlayerStatus } from "expo-audio";

import type { RecordingArtifact } from "@/lib/types";

export type SeekRequest = {
  requestId: number;
  attemptId: string;
  seconds: number | null;
};

type AudioPlayerCardProps = {
  artifact: RecordingArtifact;
  label: string;
  seekRequest: SeekRequest | null;
  onPlayed: () => void;
  onDownload: () => void;
};

function formatTime(seconds: number): string {
  const totalSeconds = Math.max(0, Math.floor(seconds));
  const minutes = Math.floor(totalSeconds / 60);
  return `${minutes}:${String(totalSeconds % 60).padStart(2, "0")}`;
}

export function AudioPlayerCard({
  artifact,
  label,
  seekRequest,
  onPlayed,
  onDownload,
}: AudioPlayerCardProps) {
  const player = useAudioPlayer(artifact.objectUrl ?? artifact.uri);
  const status = useAudioPlayerStatus(player);
  const [playbackRate, setPlaybackRate] = useState(1);
  const [repeatExcerpt, setRepeatExcerpt] = useState<{ start: number; end: number } | null>(null);
  const duration = status.duration > 0 ? status.duration : artifact.durationMs / 1000;
  const currentTime = Math.min(status.currentTime, duration || status.currentTime);

  useEffect(() => {
    player.playbackRate = playbackRate;
  }, [playbackRate, player]);

  useEffect(() => {
    if (!repeatExcerpt || !status.playing || status.currentTime < repeatExcerpt.end - 0.08) return;
    void player.seekTo(repeatExcerpt.start);
    player.play();
  }, [player, repeatExcerpt, status.currentTime, status.playing]);

  useEffect(() => {
    if (!seekRequest || seekRequest.attemptId !== artifact.id) return;
    const requestedSeconds = seekRequest.seconds ?? 0;
    void player.seekTo(Math.max(0, requestedSeconds - (seekRequest.seconds === null ? 0 : 2)));
    player.play();
    onPlayed();
  }, [artifact.id, onPlayed, player, seekRequest]);

  const togglePlayback = () => {
    if (status.playing) {
      player.pause();
      return;
    }
    if (duration > 0 && status.currentTime >= duration - 0.1) {
      void player.seekTo(0);
    }
    player.play();
    onPlayed();
  };

  const seekTo = (value: number) => {
    void player.seekTo(Math.max(0, value));
  };

  const skipBack = () => {
    setRepeatExcerpt(null);
    void player.seekTo(Math.max(0, currentTime - 5));
  };

  const toggleRepeatExcerpt = () => {
    if (repeatExcerpt) {
      setRepeatExcerpt(null);
      return;
    }
    const end = Math.min(duration, Math.max(10, currentTime + 5));
    setRepeatExcerpt({ start: Math.max(0, end - 10), end });
    void player.seekTo(Math.max(0, end - 10));
    player.play();
    onPlayed();
  };

  const cycleSpeed = () => {
    const next = playbackRate >= 1.5 ? 0.75 : playbackRate + 0.25;
    setPlaybackRate(next);
  };

  return (
    <View style={styles.container}>
      <View style={styles.headerRow}>
        <Text style={styles.label} selectable>
          {label}
        </Text>
        <Text style={styles.duration} selectable>
          {formatTime(artifact.durationMs / 1000)}
        </Text>
      </View>

      <View style={styles.controlsRow}>
        <Pressable
          accessibilityLabel={status.playing ? `Pause ${label}` : `Play ${label}`}
          accessibilityRole="button"
          onPress={togglePlayback}
          style={({ pressed }) => [styles.playButton, pressed && styles.pressed]}
        >
          <Text style={styles.playButtonText}>{status.playing ? "Pause" : "Play"}</Text>
        </Pressable>
        <View style={styles.sliderWrap}>
          <Slider
            accessibilityLabel={`Seek ${label}`}
            maximumTrackTintColor="#d9c9ba"
            maximumValue={Math.max(duration, 0.1)}
            minimumTrackTintColor="#a34f2d"
            minimumValue={0}
            onSlidingComplete={seekTo}
            onValueChange={seekTo}
            style={styles.slider}
            value={currentTime}
          />
        </View>
        <Text style={styles.position} selectable>
          {formatTime(currentTime)}
        </Text>
      </View>

      <View style={styles.utilityRow}>
        <Pressable accessibilityLabel="Skip back five seconds" accessibilityRole="button" onPress={skipBack} style={styles.utilityButton}>
          <Text style={styles.utilityText}>−5 sec</Text>
        </Pressable>
        <Pressable accessibilityLabel={`Playback speed ${playbackRate} times; change speed`} accessibilityRole="button" onPress={cycleSpeed} style={styles.utilityButton}>
          <Text style={styles.utilityText}>{playbackRate.toFixed(2).replace(".00", "")}×</Text>
        </Pressable>
        <Pressable accessibilityLabel="Repeat ten second excerpt" accessibilityRole="button" onPress={toggleRepeatExcerpt} style={[styles.utilityButton, repeatExcerpt && styles.utilitySelected]}>
          <Text style={styles.utilityText}>{repeatExcerpt ? "Repeating 10 sec" : "Repeat 10 sec"}</Text>
        </Pressable>
      </View>

      {status.error ? (
        <Text style={styles.errorText} selectable>
          Replay could not load this take: {status.error}
        </Text>
      ) : null}

      <Pressable
        accessibilityLabel="Download recording"
        accessibilityRole="button"
        onPress={onDownload}
        style={({ pressed }) => [styles.downloadButton, pressed && styles.pressed]}
      >
        <Text style={styles.downloadButtonText}>Download recording</Text>
      </Pressable>

      <Text style={styles.note} selectable>
        Original take · {artifact.mimeType || "audio type not reported"} · {artifact.bytes.toLocaleString()} bytes
      </Text>
      {artifact.warning ? (
        <Text style={styles.warning} selectable>
          {artifact.warning}
        </Text>
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    backgroundColor: "#fffaf6",
    borderColor: "#d9c9ba",
    borderRadius: 16,
    borderWidth: 1,
    gap: 12,
    padding: 16,
  },
  headerRow: {
    alignItems: "center",
    flexDirection: "row",
    justifyContent: "space-between",
  },
  label: {
    color: "#2d241f",
    fontSize: 16,
    fontWeight: "800",
  },
  duration: {
    color: "#6d6259",
    fontSize: 14,
    fontVariant: ["tabular-nums"],
  },
  controlsRow: {
    alignItems: "center",
    flexDirection: "row",
    gap: 10,
  },
  utilityRow: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: 8,
  },
  utilityButton: {
    alignItems: "center",
    borderColor: "#c9b8a8",
    borderRadius: 9,
    borderWidth: 1,
    justifyContent: "center",
    minHeight: 40,
    paddingHorizontal: 10,
  },
  utilitySelected: {
    backgroundColor: "#fff1e7",
    borderColor: "#a34f2d",
  },
  utilityText: {
    color: "#6f3c26",
    fontSize: 12,
    fontWeight: "800",
  },
  playButton: {
    alignItems: "center",
    backgroundColor: "#2d241f",
    borderRadius: 11,
    justifyContent: "center",
    minHeight: 46,
    minWidth: 72,
    paddingHorizontal: 12,
  },
  playButtonText: {
    color: "#fffaf6",
    fontSize: 14,
    fontWeight: "800",
  },
  sliderWrap: {
    flex: 1,
    minWidth: 80,
  },
  slider: {
    height: 44,
    width: "100%",
  },
  position: {
    color: "#6d6259",
    fontSize: 13,
    fontVariant: ["tabular-nums"],
    minWidth: 36,
    textAlign: "right",
  },
  pressed: {
    opacity: 0.74,
  },
  errorText: {
    color: "#8c302a",
    fontSize: 13,
    lineHeight: 19,
  },
  downloadButton: {
    alignItems: "center",
    alignSelf: "flex-start",
    borderColor: "#c9b8a8",
    borderRadius: 10,
    borderWidth: 1,
    justifyContent: "center",
    minHeight: 44,
    paddingHorizontal: 12,
  },
  downloadButtonText: {
    color: "#6f3c26",
    fontSize: 13,
    fontWeight: "700",
  },
  note: {
    color: "#75685e",
    fontSize: 12,
    lineHeight: 18,
  },
  warning: {
    backgroundColor: "#fff2ce",
    borderRadius: 9,
    color: "#6b4b18",
    fontSize: 13,
    lineHeight: 19,
    padding: 10,
  },
});
